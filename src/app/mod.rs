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
use tokio::time;

pub struct RgbApp {
    workspace: WorkspaceManager,
    layout: LayoutEngine,
    ui: Ui,
    config: AppConfig,
    state: AppState,
    terminal: Terminal<CrosstermBackend<io::Stdout>>,
    should_quit: bool,
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

        // Force initial update with a small yield to ensure terminal is ready
        tokio::task::yield_now().await;

        // Optimized main loop
        let mut needs_redraw = true;
        let mut last_update = std::time::Instant::now();

        loop {
            // Check if we should quit
            if self.should_quit {
                tracing::debug!("Quit flag set, exiting loop");
                break;
            }

            // Only update terminals periodically or when needed
            let now = std::time::Instant::now();
            if needs_redraw || now.duration_since(last_update) > Duration::from_millis(16) {
                match self.workspace.update().await {
                    Ok(_) => {},
                    Err(e) => tracing::error!("Workspace update error: {}", e),
                }
                last_update = now;
            }

            // Draw UI only when needed
            if needs_redraw {
                match self.terminal.draw(|frame| {
                    tracing::trace!("Drawing frame");

                    // Draw something simple first
                    let size = frame.area();
                    let block = ratatui::widgets::Block::default()
                        .title("RGB Terminal - Press Ctrl+Q to quit")
                        .borders(ratatui::widgets::Borders::ALL);
                    frame.render_widget(block, size);

                    // Now draw the actual UI
                    self.ui.draw(frame, &self.workspace, &mut self.layout, &self.state);
                }) {
                    Ok(_) => {},
                    Err(e) => tracing::error!("Draw failed: {}", e),
                }
                needs_redraw = false;
            }

            // Handle events with minimal blocking
            if event::poll(Duration::from_millis(1))? {
                match event::read()? {
                    Event::Key(key) => {
                        tracing::debug!("Key event: {:?}", key.code);

                        // Handle the key event
                        self.handle_key_event(key).await?;

                        // Mark for redraw after input
                        needs_redraw = true;
                    }
                    Event::Resize(width, height) => {
                        tracing::debug!("Terminal resized to {}x{}", width, height);
                        needs_redraw = true;
                    }
                    _ => {}
                }
            } else {
                // Small yield to prevent busy-waiting
                tokio::task::yield_now().await;
            }

            // Small delay
            std::thread::sleep(Duration::from_millis(50));
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
            // Close terminal
            (KeyCode::Char('w') | KeyCode::Char('W'), KeyModifiers::CONTROL) => {
                self.workspace.close_active_terminal().await?;
                if self.workspace.terminals().is_empty() {
                    self.should_quit = true;
                }
            }
            // Navigation
            (KeyCode::Char('h'), KeyModifiers::NONE) => {
                self.layout.focus_left(&mut self.workspace);
            }
            (KeyCode::Char('j'), KeyModifiers::NONE) => {
                self.layout.focus_down(&mut self.workspace);
            }
            (KeyCode::Char('k'), KeyModifiers::NONE) => {
                self.layout.focus_up(&mut self.workspace);
            }
            (KeyCode::Char('l'), KeyModifiers::NONE) => {
                self.layout.focus_right(&mut self.workspace);
            }
            // Mode switching
            (KeyCode::Char('i'), KeyModifiers::NONE) => {
                tracing::info!("Switching to Insert mode");
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
                    self.workspace.close_active_terminal().await?;
                    if self.workspace.terminals().is_empty() {
                        self.should_quit = true;
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
            KeyCode::Esc => {
                tracing::info!("ESC detected - switching back to Normal mode");
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