use crate::config::AppConfig;
use crate::layout::LayoutEngine;
use crate::ui::Ui;
use crate::workspace::WorkspaceManager;
use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind, MouseButton, EnableMouseCapture, DisableMouseCapture},
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
    terminal: Terminal<CrosstermBackend<io::Stdout>>,
    should_quit: bool,
    focus: FocusArea,
    command_mode: bool,
    command_buffer: String,
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

        // Enable mouse support
        execute!(
            io::stdout(),
            EnableMouseCapture,
        )?;
        tracing::info!("Mouse capture enabled");

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
            terminal,
            should_quit: false,
            focus: FocusArea::Terminal,
            command_mode: false,
            command_buffer: String::new(),
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
        let mut update_interval = tokio::time::interval(Duration::from_millis(50));
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
                _ = tokio::time::sleep(Duration::from_millis(10)) => {
                    // Use a longer timeout for event polling to ensure we don't miss events
                    if event::poll(Duration::from_millis(10))? {
                        tracing::trace!("Event available");
                        match event::read()? {
                            Event::Key(key) => {
                                tracing::info!("KEY EVENT RECEIVED: {:?}", key);
                                self.handle_key_event(key).await?;
                                tracing::trace!("Key event handled");

                                // Immediate redraw after input
                                self.draw_ui();
                            }
                            Event::Mouse(mouse) => {
                                tracing::info!("MOUSE EVENT: {:?}", mouse);
                                self.handle_mouse_event(mouse).await?;
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

    async fn handle_mouse_event(&mut self, mouse: MouseEvent) -> Result<()> {
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                // Get terminal areas from layout
                let terminal_areas = self.layout.get_terminal_areas();

                // Check which terminal was clicked
                for (id, area) in terminal_areas {
                    if mouse.column >= area.x && mouse.column < area.x + area.width
                        && mouse.row >= area.y && mouse.row < area.y + area.height {
                        // Set this terminal as active
                        self.workspace.set_active_terminal(id);
                        self.focus = FocusArea::Terminal;
                        break;
                    }
                }

                // Check if file explorer was clicked
                if let Some(explorer_area) = self.ui.get_file_explorer_area() {
                    if mouse.column >= explorer_area.x && mouse.column < explorer_area.x + explorer_area.width
                        && mouse.row >= explorer_area.y && mouse.row < explorer_area.y + explorer_area.height {
                        self.focus = FocusArea::FileExplorer;
                        // Handle file selection in explorer
                        self.ui.handle_file_explorer_click(mouse.column, mouse.row);
                    }
                }
            }
            MouseEventKind::ScrollDown => {
                if self.focus == FocusArea::FileExplorer {
                    self.ui.file_explorer_move_down();
                }
            }
            MouseEventKind::ScrollUp => {
                if self.focus == FocusArea::FileExplorer {
                    self.ui.file_explorer_move_up();
                }
            }
            _ => {}
        }
        Ok(())
    }

    async fn handle_key_event(&mut self, key: KeyEvent) -> Result<()> {
        tracing::info!("Handling key {:?}, command_mode: {}", key, self.command_mode);

        // Handle command mode
        if self.command_mode {
            match key.code {
                KeyCode::Esc => {
                    self.command_mode = false;
                    self.command_buffer.clear();
                    self.ui.clear_command();
                }
                KeyCode::Enter => {
                    let command = self.command_buffer.clone();
                    self.execute_command(&command).await?;
                    self.command_mode = false;
                    self.command_buffer.clear();
                    self.ui.clear_command();
                }
                KeyCode::Backspace => {
                    self.command_buffer.pop();
                    self.ui.command_backspace();
                }
                KeyCode::Char(c) => {
                    self.command_buffer.push(c);
                    self.ui.command_push(c);
                }
                _ => {}
            }
            return Ok(());
        }

        // Handle special keys that override terminal input
        match (key.code, key.modifiers) {
            // Quit application
            (KeyCode::Char('q') | KeyCode::Char('Q'), KeyModifiers::CONTROL) => {
                self.should_quit = true;
            }
            // New terminal
            (KeyCode::Char('t') | KeyCode::Char('T'), KeyModifiers::CONTROL) => {
                self.workspace.create_terminal(None).await?;
            }
            // Close terminal
            (KeyCode::Char('w') | KeyCode::Char('W'), KeyModifiers::CONTROL) => {
                if self.focus == FocusArea::FileExplorer {
                    self.focus = FocusArea::Terminal;
                } else {
                    self.workspace.close_active_terminal().await?;
                    if self.workspace.terminals().is_empty() {
                        self.workspace.create_terminal(None).await?;
                    }
                }
            }
            // Enter command mode
            (KeyCode::Char(':'), KeyModifiers::NONE) => {
                self.command_mode = true;
                self.command_buffer.clear();
            }
            // Toggle help
            (KeyCode::Char('?'), KeyModifiers::NONE) => {
                self.ui.toggle_help();
            }
            // Toggle file explorer
            (KeyCode::Char('e'), KeyModifiers::CONTROL) => {
                self.ui.toggle_file_explorer();
                self.focus = FocusArea::Terminal;
            }
            // Switch focus
            (KeyCode::Char('f'), KeyModifiers::CONTROL) => {
                self.focus = if self.focus == FocusArea::FileExplorer {
                    FocusArea::Terminal
                } else {
                    FocusArea::FileExplorer
                };
            }
            // Toggle git panel
            (KeyCode::Char('g'), KeyModifiers::CONTROL) => {
                self.ui.toggle_git_panel();
            }
            // Arrow keys for terminal navigation
            (KeyCode::Left, KeyModifiers::ALT) => {
                self.workspace.previous_terminal();
            }
            (KeyCode::Right, KeyModifiers::ALT) => {
                self.workspace.next_terminal();
            }
            // Tab switching
            (KeyCode::Tab, KeyModifiers::CONTROL) => {
                self.workspace.next_terminal();
            }
            (KeyCode::BackTab, KeyModifiers::SHIFT) => {
                self.workspace.previous_terminal();
            }
            // Quick terminal switch (F1-F10)
            (KeyCode::F(n), KeyModifiers::NONE) if n >= 1 && n <= 10 => {
                self.workspace.switch_to_terminal(n as usize - 1);
            }
            // File explorer navigation when focused
            _ if self.focus == FocusArea::FileExplorer => {
                match key.code {
                    KeyCode::Up => self.ui.file_explorer_move_up(),
                    KeyCode::Down => self.ui.file_explorer_move_down(),
                    KeyCode::Left => self.ui.file_explorer_toggle_expand(),
                    KeyCode::Right => self.ui.file_explorer_toggle_expand(),
                    KeyCode::Enter => {
                        if let Some(path) = self.ui.file_explorer_open() {
                            let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vi".to_string());
                            let command = format!("{} {}", editor, path);
                            self.workspace.create_terminal(Some(command)).await?;
                            self.focus = FocusArea::Terminal;
                        }
                    }
                    _ => {}
                }
            }
            // Forward all other keys to the active terminal
            _ => {
                if self.focus == FocusArea::Terminal {
                    if let Some(emulator) = self.workspace.get_active_terminal_emulator() {
                        if let Some(em) = emulator.try_read() {
                            if em.is_alive() {
                                drop(em);
                                self.workspace.send_key_to_active_terminal(key).await?;
                            } else {
                                // Terminal is dead, don't forward input but allow Ctrl+W to close
                                tracing::debug!("Terminal is dead, not forwarding key: {:?}", key);
                            }
                        } else {
                            // Could not get lock, try to send anyway
                            self.workspace.send_key_to_active_terminal(key).await?;
                        }
                    }
                }
            }
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

            let state = if self.command_mode { AppState::Command } else { AppState::Normal };
            self.ui.draw(frame, &self.workspace, &mut self.layout, &state);
        }) {
            Ok(_) => {},
            Err(e) => tracing::error!("Draw failed: {}", e),
        }
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
            DisableMouseCapture,
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