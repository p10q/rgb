use crate::workspace::TerminalId;
use anyhow::Result;
use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use parking_lot::RwLock;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

pub struct FileTracker {
    terminal_files: Arc<RwLock<HashMap<TerminalId, HashSet<PathBuf>>>>,
    file_changes: Arc<RwLock<Vec<FileChange>>>,
    conflict_detector: Arc<ConflictDetector>,
    monitor: Arc<RwLock<Option<FileMonitor>>>,
}

pub struct FileMonitor {
    watcher: RecommendedWatcher,
    event_rx: mpsc::UnboundedReceiver<FileEvent>,
    tracked_paths: Arc<RwLock<HashSet<PathBuf>>>,
}

#[derive(Debug, Clone)]
pub struct FileChange {
    pub terminal_id: Option<TerminalId>,
    pub file_path: PathBuf,
    pub change_type: ChangeType,
    pub timestamp: Instant,
}

#[derive(Debug, Clone)]
pub enum ChangeType {
    Created,
    Modified,
    Deleted,
    Renamed { from: PathBuf, to: PathBuf },
}

#[derive(Debug, Clone)]
pub struct FileEvent {
    pub path: PathBuf,
    pub kind: FileEventKind,
    pub timestamp: Instant,
}

#[derive(Debug, Clone)]
pub enum FileEventKind {
    Create,
    Modify,
    Delete,
}

pub struct ConflictDetector {
    overlaps: Arc<RwLock<HashMap<PathBuf, Vec<TerminalId>>>>,
    resolution_strategy: ConflictResolution,
}

#[derive(Debug, Clone)]
pub enum ConflictResolution {
    Warn,
    Block,
    AutoResolve,
}

impl FileTracker {
    pub fn new_disabled() -> Self {
        // Create a disabled file tracker that doesn't monitor anything
        Self {
            terminal_files: Arc::new(RwLock::new(HashMap::new())),
            file_changes: Arc::new(RwLock::new(Vec::new())),
            conflict_detector: Arc::new(ConflictDetector::new()),
            monitor: Arc::new(RwLock::new(None)),
        }
    }

    pub fn new(project_dir: &Path) -> Result<Self> {
        let conflict_detector = Arc::new(ConflictDetector::new());
        let mut tracker = Self {
            terminal_files: Arc::new(RwLock::new(HashMap::new())),
            file_changes: Arc::new(RwLock::new(Vec::new())),
            conflict_detector,
            monitor: Arc::new(RwLock::new(None)),
        };

        // Start monitoring the project directory
        tracker.start_monitoring(project_dir)?;

        Ok(tracker)
    }

    fn start_monitoring(&mut self, path: &Path) -> Result<()> {
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        let event_tx_clone = event_tx.clone();

        let mut watcher = RecommendedWatcher::new(
            move |res: Result<Event, notify::Error>| {
                if let Ok(event) = res {
                    let file_event = match event.kind {
                        EventKind::Create(_) => Some(FileEventKind::Create),
                        EventKind::Modify(_) => Some(FileEventKind::Modify),
                        EventKind::Remove(_) => Some(FileEventKind::Delete),
                        _ => None,
                    };

                    if let Some(kind) = file_event {
                        for path in event.paths {
                            let _ = event_tx_clone.send(FileEvent {
                                path,
                                kind: kind.clone(),
                                timestamp: Instant::now(),
                            });
                        }
                    }
                }
            },
            Config::default()
                .with_poll_interval(Duration::from_secs(1))
                .with_compare_contents(false),
        )?;

        watcher.watch(path, RecursiveMode::Recursive)?;

        let monitor = FileMonitor {
            watcher,
            event_rx,
            tracked_paths: Arc::new(RwLock::new(HashSet::new())),
        };

        *self.monitor.write() = Some(monitor);

        // Start processing events
        self.spawn_event_processor();

        Ok(())
    }

    fn spawn_event_processor(&self) {
        let file_changes = self.file_changes.clone();
        let monitor = self.monitor.clone();

        tokio::spawn(async move {
            loop {
                if let Some(ref mut mon) = *monitor.write() {
                    if let Ok(event) = mon.event_rx.try_recv() {
                        // Process file event
                        let change = FileChange {
                            terminal_id: None, // Will be determined by tracking
                            file_path: event.path,
                            change_type: match event.kind {
                                FileEventKind::Create => ChangeType::Created,
                                FileEventKind::Modify => ChangeType::Modified,
                                FileEventKind::Delete => ChangeType::Deleted,
                            },
                            timestamp: event.timestamp,
                        };

                        file_changes.write().push(change);

                        // Keep only recent changes (last 1000)
                        if file_changes.read().len() > 1000 {
                            let mut changes = file_changes.write();
                            let drain_count = changes.len().saturating_sub(1000);
                            changes.drain(0..drain_count);
                        }
                    }
                } else {
                    break;
                }

                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        });
    }

    pub fn start_tracking_terminal(&self, terminal_id: TerminalId) {
        self.terminal_files.write().insert(terminal_id, HashSet::new());
    }

    pub fn stop_tracking_terminal(&self, terminal_id: TerminalId) {
        self.terminal_files.write().remove(&terminal_id);
        self.conflict_detector.remove_terminal(terminal_id);
    }

    pub fn track_file(&self, terminal_id: TerminalId, file: PathBuf) {
        if let Some(files) = self.terminal_files.write().get_mut(&terminal_id) {
            files.insert(file.clone());
            self.conflict_detector.add_file_terminal(file, terminal_id);
        }
    }

    pub fn untrack_file(&self, terminal_id: TerminalId, file: &Path) {
        if let Some(files) = self.terminal_files.write().get_mut(&terminal_id) {
            files.remove(file);
            self.conflict_detector.remove_file_terminal(file, terminal_id);
        }
    }

    pub fn get_terminal_files(&self, terminal_id: TerminalId) -> HashSet<PathBuf> {
        self.terminal_files
            .read()
            .get(&terminal_id)
            .cloned()
            .unwrap_or_default()
    }

    pub fn get_file_changes(&self, since: Option<Instant>) -> Vec<FileChange> {
        let changes = self.file_changes.read();
        if let Some(since_time) = since {
            changes
                .iter()
                .filter(|c| c.timestamp > since_time)
                .cloned()
                .collect()
        } else {
            changes.clone()
        }
    }

    pub fn detect_conflicts(&self) -> Vec<FileConflict> {
        self.conflict_detector.get_conflicts()
    }

    pub fn update(&self) -> Result<()> {
        // Process any pending file events
        // This is handled by the background processor
        Ok(())
    }
}

impl ConflictDetector {
    pub fn new() -> Self {
        Self {
            overlaps: Arc::new(RwLock::new(HashMap::new())),
            resolution_strategy: ConflictResolution::Warn,
        }
    }

    pub fn add_file_terminal(&self, file: PathBuf, terminal_id: TerminalId) {
        self.overlaps
            .write()
            .entry(file)
            .or_insert_with(Vec::new)
            .push(terminal_id);
    }

    pub fn remove_file_terminal(&self, file: &Path, terminal_id: TerminalId) {
        if let Some(terminals) = self.overlaps.write().get_mut(file) {
            terminals.retain(|&id| id != terminal_id);
            if terminals.is_empty() {
                self.overlaps.write().remove(file);
            }
        }
    }

    pub fn remove_terminal(&self, terminal_id: TerminalId) {
        let mut overlaps = self.overlaps.write();
        overlaps.retain(|_, terminals| {
            terminals.retain(|&id| id != terminal_id);
            !terminals.is_empty()
        });
    }

    pub fn get_conflicts(&self) -> Vec<FileConflict> {
        self.overlaps
            .read()
            .iter()
            .filter(|(_, terminals)| terminals.len() > 1)
            .map(|(file, terminals)| FileConflict {
                file: file.clone(),
                terminal_ids: terminals.clone(),
            })
            .collect()
    }

    pub fn set_resolution_strategy(&mut self, strategy: ConflictResolution) {
        self.resolution_strategy = strategy;
    }
}

#[derive(Debug, Clone)]
pub struct FileConflict {
    pub file: PathBuf,
    pub terminal_ids: Vec<TerminalId>,
}

impl Drop for FileMonitor {
    fn drop(&mut self) {
        // Watcher will be dropped automatically
        tracing::info!("File monitor stopped");
    }
}