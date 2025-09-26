use crate::config::AppConfig;
use crate::layout::LayoutEngine;
use crate::ui::Ui;
use crate::workspace::WorkspaceManager;
use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    Terminal,
};
use std::{io, path::PathBuf};
use std::time::Duration;
use tokio::sync::mpsc;

#[derive(Debug, Clone, Copy, PartialEq)]
enum FocusArea {
    Terminal,
    FileExplorer,
}

pub struct RgbApp {
    workspace: WorkspaceManager,
    layout: LayoutEngine,
    ui: Ui,
    config: AppConfig,
    state: AppState,
    terminal: Terminal<CrosstermBackend<io::Stdout>>,
    should_quit: bool,
    focus: FocusArea,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AppState {
    Normal,
    Insert,
    Command,
    Visual,
}

impl RgbApp {
    pub fn new(config: AppConfig, project_dir: PathBuf) -> Result<Self> {
        tracing::info!("RgbApp::new called with project_dir: {:?}", project_dir);

        // Setup terminal
        enable_raw_mode()?;
        tracing::info!("Raw mode enabled");

        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen)?;
        tracing::info!("Entered alternate screen");

        let backend = CrosstermBackend::new(stdout);
        let terminal = Terminal::new(backend)?;
        tracing::info!("Terminal created");

        // Initialize components
        let workspace = WorkspaceManager::new(project_dir.clone())?;
        tracing::info!("WorkspaceManager created");

        let layout = LayoutEngine::new();
        let ui = Ui::new();
        tracing::info!("Layout and UI created");

        Ok(Self {
            workspace,
            layout,
            ui,
            config,
            state: AppState::Normal,
            terminal,
            should_quit: false,
            focus: FocusArea::Terminal,
        })
    }

    pub async fn create_terminal_with_command(&mut self, command: &str) -> Result<()> {
        self.workspace.create_terminal(Some(command.to_string())).await?;
        Ok(())
    }

    pub async fn run(&mut self) -> Result<()> {
        tracing::info!("App::run started");

        // Create initial terminal if workspace is empty
        if self.workspace.terminals().is_empty() {
            tracing::info!("Creating initial terminal");
            self.workspace.create_terminal(None).await?;
        }

        tracing::info!("Starting simplified main loop");

        // Do an initial update to get terminal content
        tracing::info!("Doing initial workspace update");
        match self.workspace.update().await {
            Ok(_) => tracing::info!("Initial workspace update complete"),
            Err(e) => tracing::error!("Initial workspace update error: {}", e),
        }

        // Create channel for redraw signals
        let (redraw_tx, mut redraw_rx) = mpsc::unbounded_channel::<()>();

        // Give workspace a way to signal redraws
        self.workspace.set_redraw_sender(redraw_tx.clone());

        // Initial draw
        self.draw_ui();

        // Event-driven main loop with continuous terminal monitoring
        let mut update_interval = tokio::time::interval(Duration::from_millis(5));
        update_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            if self.should_quit {
                tracing::debug!("Quit flag set, exiting loop");
                break;
            }

            tokio::select! {
                // Continuous terminal output monitoring
                _ = update_interval.tick() => {
                    // Update terminal buffers
                    match self.workspace.update().await {
                        Ok(_) => {
                            // Workspace update will signal redraw if needed
                        },
                        Err(e) => tracing::error!("Workspace update error: {}", e),
                    }
                }

                // Handle explicit redraw signals
                _ = redraw_rx.recv() => {
                    tracing::trace!("Redraw signal received");
                    self.draw_ui();
                }

                // Handle keyboard/mouse events
                _ = tokio::time::sleep(Duration::from_millis(1)) => {
                    if event::poll(Duration::from_millis(0))? {
                        match event::read()? {
                            Event::Key(key) => {
                                tracing::debug!("Key event: {:?}", key.code);
                                self.handle_key_event(key).await?;

                                // Immediate redraw after input
                                self.draw_ui();
                            }
                            Event::Resize(width, height) => {
                                tracing::debug!("Terminal resized to {}x{}", width, height);
                                self.draw_ui();
                            }
                            _ => {}
                        }
                    }
                }
            }
        }

        self.cleanup()?;
        Ok(())
    }

    async fn handle_key_event(&mut self, key: KeyEvent) -> Result<()> {
        tracing::info!("Handling key {:?} in state {:?}", key, self.state);
        match self.state {
            AppState::Normal => self.handle_normal_mode(key).await?,
            AppState::Insert => self.handle_insert_mode(key).await?,
            AppState::Command => self.handle_command_mode(key).await?,
            AppState::Visual => self.handle_visual_mode(key).await?,
        }
        Ok(())
    }

    fn draw_ui(&mut self) {
        match self.terminal.draw(|frame| {
            tracing::trace!("Drawing frame");

            let size = frame.area();
            let block = ratatui::widgets::Block::default()
                .title("RGB Terminal - Press Ctrl+Q to quit")
                .borders(ratatui::widgets::Borders::ALL);
            frame.render_widget(block, size);

            self.ui.draw(frame, &self.workspace, &mut self.layout, &self.state);
        }) {
            Ok(_) => {},
            Err(e) => tracing::error!("Draw failed: {}", e),
        }
    }

    async fn handle_normal_mode(&mut self, key: KeyEvent) -> Result<()> {
        match (key.code, key.modifiers) {
            // Quit
            (KeyCode::Char('q') | KeyCode::Char('Q'), KeyModifiers::CONTROL) => {
                self.should_quit = true;
            }
            // New terminal
            (KeyCode::Char('t') | KeyCode::Char('T'), KeyModifiers::CONTROL) => {
                self.workspace.create_terminal(None).await?;
            }
            // Close terminal or exit file explorer
            (KeyCode::Char('w') | KeyCode::Char('W'), KeyModifiers::CONTROL) => {
                if self.focus == FocusArea::FileExplorer {
                    self.focus = FocusArea::Terminal;
                } else {
                    // Check if terminal is dead first
                    let should_close = if let Some(emulator) = self.workspace.get_active_terminal_emulator() {
                        true  // Always allow closing
                    } else {
                        false
                    };

                    if should_close {
                        self.workspace.close_active_terminal().await?;
                        if self.workspace.terminals().is_empty() {
                            self.should_quit = true;
                        }
                    }
                }
            }
            // Show help with '?'
            (KeyCode::Char('?'), KeyModifiers::NONE) => {
                self.ui.toggle_help();
            }
            // Navigation
            (KeyCode::Char('h'), KeyModifiers::NONE) => {
                match self.focus {
                    FocusArea::Terminal => self.layout.focus_left(&mut self.workspace),
                    FocusArea::FileExplorer => self.ui.file_explorer_toggle_expand(),
                }
            }
            (KeyCode::Char('j'), KeyModifiers::NONE) => {
                match self.focus {
                    FocusArea::Terminal => self.layout.focus_down(&mut self.workspace),
                    FocusArea::FileExplorer => self.ui.file_explorer_move_down(),
                }
            }
            (KeyCode::Char('k'), KeyModifiers::NONE) => {
                match self.focus {
                    FocusArea::Terminal => self.layout.focus_up(&mut self.workspace),
                    FocusArea::FileExplorer => self.ui.file_explorer_move_up(),
                }
            }
            (KeyCode::Char('l'), KeyModifiers::NONE) => {
                match self.focus {
                    FocusArea::Terminal => self.layout.focus_right(&mut self.workspace),
                    FocusArea::FileExplorer => self.ui.file_explorer_toggle_expand(),
                }
            }
            // Enter key for file explorer
            (KeyCode::Enter, KeyModifiers::NONE) if self.focus == FocusArea::FileExplorer => {
                if let Some(path) = self.ui.file_explorer_open() {
                    // Open file in new terminal with appropriate editor
                    let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vi".to_string());
                    let command = format!("{} {}", editor, path);
                    self.workspace.create_terminal(Some(command)).await?;
                    self.focus = FocusArea::Terminal;
                }
            }
            // Mode switching - Use Cmd+I (or Alt+I on Linux/Windows) for insert mode
            (KeyCode::Char('i'), KeyModifiers::NONE) => {
                // In normal mode, 'i' alone does nothing - must use Cmd+I
                // This allows 'i' to work normally in terminal programs like vim
                tracing::debug!("Plain 'i' key - not switching to insert mode (use Cmd+I)");
            }
            // Cmd+I or Alt+I to enter insert mode
            (KeyCode::Char('i') | KeyCode::Char('I'), KeyModifiers::ALT) => {
                // Check if current terminal is alive before entering insert mode
                if let Some(emulator) = self.workspace.get_active_terminal_emulator() {
                    let is_alive = emulator.read().is_alive();
                    if !is_alive {
                        tracing::info!("Cannot enter insert mode - terminal is dead");
                        // Don't switch to insert mode for dead terminals
                        return Ok(());
                    }
                }
                tracing::info!("Switching to Insert mode via Alt+I");
                self.state = AppState::Insert;
            }
            (KeyCode::Char(':'), KeyModifiers::NONE) => {
                self.state = AppState::Command;
            }
            (KeyCode::Char('v'), KeyModifiers::NONE) => {
                self.state = AppState::Visual;
            }
            // Toggle file explorer
            (KeyCode::Char('e'), KeyModifiers::CONTROL) => {
                self.ui.toggle_file_explorer();
                // Reset focus to terminal if explorer was closed
                self.focus = FocusArea::Terminal;
            }
            // Switch focus to file explorer
            (KeyCode::Char('f'), KeyModifiers::CONTROL) => {
                self.focus = if self.focus == FocusArea::FileExplorer {
                    FocusArea::Terminal
                } else {
                    FocusArea::FileExplorer
                };
                tracing::info!("Focus switched to {:?}", self.focus);
            }
            // Toggle git panel
            (KeyCode::Char('g'), KeyModifiers::CONTROL) => {
                self.ui.toggle_git_panel();
            }
            // Tab switching
            (KeyCode::Tab, KeyModifiers::NONE) => {
                self.workspace.next_terminal();
            }
            (KeyCode::BackTab, KeyModifiers::SHIFT) => {
                self.workspace.previous_terminal();
            }
            // Quick terminal switch (F1-F10)
            (KeyCode::F(n), KeyModifiers::NONE) if n >= 1 && n <= 10 => {
                self.workspace.switch_to_terminal(n as usize - 1);
            }
            _ => {}
        }
        Ok(())
    }

    async fn handle_insert_mode(&mut self, key: KeyEvent) -> Result<()> {
        tracing::info!("Insert mode handling key: {:?}, modifiers bits: {:b}", key, key.modifiers.bits());

        // Check if current terminal is dead - if so, allow mode switching and terminal management
        if let Some(emulator) = self.workspace.get_active_terminal_emulator() {
            let is_alive = emulator.read().is_alive();
            if !is_alive {
                // Terminal is dead, but still allow essential keys
                match (key.code, key.modifiers) {
                    // Allow exiting insert mode
                    (KeyCode::Esc, _) => {
                        tracing::info!("Exiting insert mode (terminal is dead)");
                        self.state = AppState::Normal;
                        return Ok(());
                    }
                    // Allow Alt+F to exit insert mode
                    (KeyCode::Char('f') | KeyCode::Char('F'), KeyModifiers::ALT) => {
                        tracing::info!("Alt+F detected - exiting insert mode (terminal is dead)");
                        self.state = AppState::Normal;
                        return Ok(());
                    }
                    // Allow closing the dead terminal
                    (KeyCode::Char('w') | KeyCode::Char('W'), KeyModifiers::CONTROL) => {
                        tracing::info!("Closing dead terminal");
                        self.workspace.close_active_terminal().await?;
                        if self.workspace.terminals().is_empty() {
                            self.should_quit = true;
                        }
                        return Ok(());
                    }
                    // Allow switching terminals
                    (KeyCode::Tab, _) => {
                        self.workspace.next_terminal();
                        return Ok(());
                    }
                    // Allow quitting the app
                    (KeyCode::Char('q') | KeyCode::Char('Q'), KeyModifiers::CONTROL) => {
                        tracing::info!("Ctrl+Q - quitting app");
                        self.should_quit = true;
                        return Ok(());
                    }
                    _ => {
                        // Ignore other input for dead terminals (don't forward to terminal)
                        tracing::debug!("Ignoring key input for dead terminal");
                        return Ok(());
                    }
                }
            }
        }

        // Check for Ctrl combinations first (they take priority)
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            tracing::info!("Ctrl modifier detected");
            match key.code {
                KeyCode::Char('q') | KeyCode::Char('Q') => {
                    tracing::info!("Ctrl+Q in Insert mode - quitting");
                    self.should_quit = true;
                    return Ok(());
                }
                KeyCode::Char('t') | KeyCode::Char('T') => {
                    tracing::info!("Ctrl+T in Insert mode - creating new terminal");
                    self.workspace.create_terminal(None).await?;
                    return Ok(());
                }
                KeyCode::Char('w') | KeyCode::Char('W') => {
                    tracing::info!("Ctrl+W in Insert mode - closing terminal");
                    if self.focus == FocusArea::FileExplorer {
                        self.focus = FocusArea::Terminal;
                    } else {
                        self.workspace.close_active_terminal().await?;
                        if self.workspace.terminals().is_empty() {
                            self.should_quit = true;
                        }
                    }
                    return Ok(());
                }
                _ => {
                    // Forward other Ctrl combinations to terminal
                    tracing::info!("Forwarding Ctrl key to terminal: {:?}", key);
                    self.workspace.send_key_to_active_terminal(key).await?;
                    return Ok(());
                }
            }
        }

        // Now check for special keys
        match key.code {
            // Allow both Esc and Alt+Esc to exit insert mode
            KeyCode::Esc => {
                tracing::info!("ESC detected - switching back to Normal mode");
                // Close help if it's open, otherwise switch to Normal mode
                if self.ui.is_help_visible() {
                    self.ui.toggle_help();
                } else {
                    self.state = AppState::Normal;
                }
            }
            // Alt+F as an alternative to exit insert mode (easier than Esc on some keyboards)
            KeyCode::Char('f') | KeyCode::Char('F') if key.modifiers.contains(KeyModifiers::ALT) => {
                tracing::info!("Alt+F detected - switching back to Normal mode");
                self.state = AppState::Normal;
            }
            _ => {
                // Forward regular keys to terminal
                tracing::info!("Forwarding key to terminal: {:?}", key);
                self.workspace.send_key_to_active_terminal(key).await?;
            }
        }
        Ok(())
    }

    async fn handle_command_mode(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Esc => {
                self.state = AppState::Normal;
                self.ui.clear_command();
            }
            KeyCode::Enter => {
                let command = self.ui.get_command();
                self.execute_command(&command).await?;
                self.state = AppState::Normal;
                self.ui.clear_command();
            }
            KeyCode::Backspace => {
                self.ui.command_backspace();
            }
            KeyCode::Char(c) => {
                self.ui.command_push(c);
            }
            _ => {}
        }
        Ok(())
    }

    async fn handle_visual_mode(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Esc => {
                self.state = AppState::Normal;
            }
            // TODO: Implement visual mode selection and operations
            _ => {}
        }
        Ok(())
    }

    async fn execute_command(&mut self, command: &str) -> Result<()> {
        let parts: Vec<&str> = command.split_whitespace().collect();
        if parts.is_empty() {
            return Ok(());
        }

        match parts[0] {
            "quit" | "q" => self.should_quit = true,
            "new" => {
                let cmd = parts.get(1).map(|s| s.to_string());
                self.workspace.create_terminal(cmd).await?;
            }
            "worktree" => {
                // Show worktree info
                self.ui.show_worktree_info(&self.workspace);
            }
            "commit" => {
                // Open commit interface
                self.ui.show_commit_interface();
            }
            "layout" => {
                if let Some(layout_name) = parts.get(1) {
                    self.layout.apply_layout(layout_name)?;
                }
            }
            "config" => {
                // Open configuration
                self.ui.show_config_editor(&self.config);
            }
            _ => {
                self.ui.show_error(&format!("Unknown command: {}", parts[0]));
            }
        }
        Ok(())
    }

    fn cleanup(&mut self) -> Result<()> {
        disable_raw_mode()?;
        execute!(
            self.terminal.backend_mut(),
            LeaveAlternateScreen,
        )?;
        self.terminal.show_cursor()?;
        Ok(())
    }
}

impl Drop for RgbApp {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}