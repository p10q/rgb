use crate::git::GitManager;
use crate::monitor::FileTracker;
use crate::terminal::TerminalEmulator;
use anyhow::Result;
use crossterm::event::KeyEvent;
use parking_lot::RwLock;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use uuid::Uuid;

pub type TerminalId = Uuid;

pub struct WorkspaceManager {
    terminals: Arc<RwLock<Vec<TerminalSession>>>,
    active_terminal: Arc<RwLock<Option<TerminalId>>>,
    project_dir: PathBuf,
    git_manager: Arc<GitManager>,
    file_tracker: Arc<FileTracker>,
    max_terminals: usize,
    redraw_tx: Arc<RwLock<Option<mpsc::UnboundedSender<()>>>>,
}

pub struct TerminalSession {
    pub id: TerminalId,
    pub title: String,
    pub emulator: Arc<RwLock<TerminalEmulator>>,
    pub working_dir: PathBuf,
    pub active_files: HashSet<PathBuf>,
    pub worktree_path: Option<PathBuf>,
}

impl WorkspaceManager {
    pub fn new(project_dir: PathBuf) -> Result<Self> {
        let git_manager = Arc::new(GitManager::new(&project_dir)?);
        // Skip file tracker for now - it might be blocking
        // let file_tracker = Arc::new(FileTracker::new(&project_dir)?);
        let file_tracker = Arc::new(FileTracker::new_disabled());

        Ok(Self {
            terminals: Arc::new(RwLock::new(Vec::new())),
            active_terminal: Arc::new(RwLock::new(None)),
            project_dir,
            git_manager,
            file_tracker,
            max_terminals: 10,
            redraw_tx: Arc::new(RwLock::new(None)),
        })
    }

    pub async fn create_terminal(&self, command: Option<String>) -> Result<TerminalId> {
        let terminals = self.terminals.read();
        if terminals.len() >= self.max_terminals {
            anyhow::bail!("Maximum number of terminals ({}) reached", self.max_terminals);
        }
        drop(terminals);

        let id = Uuid::new_v4();
        let title = format!("Terminal {}", self.terminals.read().len() + 1);

        // If no command specified, pass empty string to let TerminalEmulator handle shell setup
        let cmd = command.unwrap_or_else(|| String::new());

        tracing::info!("Creating terminal with command: {:?}", cmd);

        // Create worktree if git is enabled
        let worktree_path = if self.git_manager.is_git_repo() {
            self.git_manager.create_worktree(id).await.ok()
        } else {
            None
        };

        // Determine working directory
        let working_dir = worktree_path.clone().unwrap_or_else(|| self.project_dir.clone());

        // Create terminal emulator
        let emulator = TerminalEmulator::new(&cmd, &working_dir, (80, 24))?;

        // Create Arc for the emulator
        let emulator_arc = Arc::new(RwLock::new(emulator));

        let session = TerminalSession {
            id,
            title,
            emulator: emulator_arc,
            working_dir,
            active_files: HashSet::new(),
            worktree_path,
        };

        // Add to terminals list
        self.terminals.write().push(session);

        // Set as active if it's the first terminal
        if self.active_terminal.read().is_none() {
            *self.active_terminal.write() = Some(id);
        }

        // Start file tracking for this terminal
        self.file_tracker.start_tracking_terminal(id);

        Ok(id)
    }

    pub async fn close_terminal(&self, id: TerminalId) -> Result<()> {
        // Stop file tracking
        self.file_tracker.stop_tracking_terminal(id);

        // Clean up worktree if exists
        if self.git_manager.is_git_repo() {
            self.git_manager.cleanup_worktree(id).await?;
        }

        // Shutdown the terminal emulator properly
        let mut terminals = self.terminals.write();
        if let Some(terminal) = terminals.iter_mut().find(|t| t.id == id) {
            terminal.emulator.write().shutdown();
        }

        // Remove from terminals list
        terminals.retain(|t| t.id != id);

        // Update active terminal if needed
        let mut active = self.active_terminal.write();
        if active.as_ref() == Some(&id) {
            *active = terminals.first().map(|t| t.id);
        }

        Ok(())
    }

    pub async fn close_active_terminal(&self) -> Result<()> {
        if let Some(id) = *self.active_terminal.read() {
            self.close_terminal(id).await?;
        }
        Ok(())
    }

    pub fn terminals(&self) -> Vec<TerminalInfo> {
        self.terminals
            .read()
            .iter()
            .map(|t| TerminalInfo {
                id: t.id,
                title: t.title.clone(),
                working_dir: t.working_dir.clone(),
                active_files_count: t.active_files.len(),
                has_worktree: t.worktree_path.is_some(),
            })
            .collect()
    }

    pub fn active_terminal_id(&self) -> Option<TerminalId> {
        *self.active_terminal.read()
    }

    pub fn next_terminal(&self) {
        let terminals = self.terminals.read();
        if terminals.is_empty() {
            return;
        }

        let mut active = self.active_terminal.write();
        if let Some(current_id) = *active {
            let current_index = terminals.iter().position(|t| t.id == current_id);
            if let Some(idx) = current_index {
                let next_idx = (idx + 1) % terminals.len();
                *active = Some(terminals[next_idx].id);
            }
        } else if !terminals.is_empty() {
            *active = Some(terminals[0].id);
        }
    }

    pub fn previous_terminal(&self) {
        let terminals = self.terminals.read();
        if terminals.is_empty() {
            return;
        }

        let mut active = self.active_terminal.write();
        if let Some(current_id) = *active {
            let current_index = terminals.iter().position(|t| t.id == current_id);
            if let Some(idx) = current_index {
                let prev_idx = if idx == 0 {
                    terminals.len() - 1
                } else {
                    idx - 1
                };
                *active = Some(terminals[prev_idx].id);
            }
        } else if !terminals.is_empty() {
            *active = Some(terminals[0].id);
        }
    }

    pub fn switch_to_terminal(&self, index: usize) {
        let terminals = self.terminals.read();
        if index < terminals.len() {
            *self.active_terminal.write() = Some(terminals[index].id);
        }
    }

    pub fn set_active_terminal(&self, id: TerminalId) {
        let terminals = self.terminals.read();
        if terminals.iter().any(|t| t.id == id) {
            *self.active_terminal.write() = Some(id);
        }
    }

    pub fn set_redraw_sender(&self, tx: mpsc::UnboundedSender<()>) {
        *self.redraw_tx.write() = Some(tx);
    }

    fn signal_redraw(&self) {
        if let Some(ref tx) = *self.redraw_tx.read() {
            let _ = tx.send(());
        }
    }

    pub async fn send_key_to_active_terminal(&self, key: KeyEvent) -> Result<()> {
        // Get the active terminal ID first, then drop the lock immediately
        let active_id = {
            let guard = self.active_terminal.read();
            guard.clone()
        };

        if let Some(id) = active_id {
            // Get the emulator reference, then drop the terminals lock
            let emulator = {
                let terminals = self.terminals.read();
                terminals.iter()
                    .find(|t| t.id == id)
                    .map(|t| t.emulator.clone())
            };

            // Now we can safely write to the emulator without holding other locks
            if let Some(emulator) = emulator {
                emulator.write().handle_key_event(key)?;
            }
        }
        Ok(())
    }

    pub async fn update(&self) -> Result<()> {
        tracing::trace!("WorkspaceManager::update start");

        // Get terminal emulator references first, then drop the lock
        let emulators: Vec<Arc<RwLock<TerminalEmulator>>> = {
            let terminals = self.terminals.read();
            terminals.iter().map(|t| t.emulator.clone()).collect()
        };
        // terminals lock is now dropped

        let mut had_output = false;

        // Update each terminal emulator without holding the terminals lock
        for emulator in emulators {
            // Try to get a write lock - if we can't, skip this update
            if let Some(mut em) = emulator.try_write() {
                // Skip dead terminals to avoid infinite EOF reading
                if !em.is_alive() {
                    tracing::trace!("Skipping update for dead terminal");
                    continue;
                }

                tracing::trace!("Calling terminal update");
                match em.update() {
                    Ok(has_output) => {
                        if has_output {
                            had_output = true;
                            tracing::trace!("Terminal update returned true (redraw needed)");
                        }
                    }
                    Err(e) => {
                        tracing::warn!("Failed to update terminal: {}", e);
                    }
                }
            } else {
                tracing::warn!("Skipping terminal update - couldn't get write lock");
            }
        }

        // Signal redraw if we had output
        if had_output {
            self.signal_redraw();
        }
        tracing::debug!("All terminal emulators updated");

        // Skip file tracking for now - might be blocking
        // self.file_tracker.update()?;
        tracing::trace!("File tracker skipped");

        // Check for file conflicts
        let conflicts = self.detect_file_conflicts();
        if !conflicts.is_empty() {
            // TODO: Handle conflicts (show warnings, etc.)
            tracing::warn!("File conflicts detected: {:?}", conflicts);
        }

        // Skip git worktree sync for now - it might be blocking
        // TODO: Fix git worktree sync
        /*
        if self.git_manager.is_git_repo() {
            for terminal in self.terminals.read().iter() {
                if terminal.worktree_path.is_some() {
                    self.git_manager.sync_worktree(terminal.id).await.ok();
                }
            }
        }
        */

        Ok(())
    }

    fn detect_file_conflicts(&self) -> Vec<FileConflict> {
        let mut file_terminals: HashMap<PathBuf, Vec<TerminalId>> = HashMap::new();
        let terminals = self.terminals.read();

        for terminal in terminals.iter() {
            for file in &terminal.active_files {
                file_terminals
                    .entry(file.clone())
                    .or_default()
                    .push(terminal.id);
            }
        }

        let mut conflicts = Vec::new();
        for (file, terminal_ids) in file_terminals {
            if terminal_ids.len() > 1 {
                conflicts.push(FileConflict {
                    file,
                    terminal_ids,
                });
            }
        }

        conflicts
    }

    pub fn get_terminal_emulator(&self, id: TerminalId) -> Option<Arc<RwLock<TerminalEmulator>>> {
        // Get the emulator and drop the lock immediately
        let terminals = self.terminals.read();
        let result = terminals
            .iter()
            .find(|t| t.id == id)
            .map(|t| t.emulator.clone());
        drop(terminals);  // Explicitly drop lock
        result
    }

    pub fn get_active_terminal_emulator(&self) -> Option<Arc<RwLock<TerminalEmulator>>> {
        // Get active ID and drop lock immediately
        let active_id = {
            let guard = self.active_terminal.read();
            guard.clone()
        }?;

        // Now get the emulator without holding the active_terminal lock
        self.get_terminal_emulator(active_id)
    }

    pub fn resize_terminal(&self, id: TerminalId, width: u16, height: u16) -> Result<()> {
        let terminals = self.terminals.read();
        if let Some(terminal) = terminals.iter().find(|t| t.id == id) {
            terminal.emulator.write().resize((width, height))?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct TerminalInfo {
    pub id: TerminalId,
    pub title: String,
    pub working_dir: PathBuf,
    pub active_files_count: usize,
    pub has_worktree: bool,
}

#[derive(Debug, Clone)]
pub struct FileConflict {
    pub file: PathBuf,
    pub terminal_ids: Vec<TerminalId>,
}