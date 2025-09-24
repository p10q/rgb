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
use std::{io, path::PathBuf, time::Duration};
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
        // Setup terminal
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen)?;
        let backend = CrosstermBackend::new(stdout);
        let terminal = Terminal::new(backend)?;

        // Initialize components
        let workspace = WorkspaceManager::new(project_dir.clone())?;
        let layout = LayoutEngine::new();
        let ui = Ui::new();

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
        // Create initial terminal if workspace is empty
        if self.workspace.terminals().is_empty() {
            self.workspace.create_terminal(None).await?;
        }

        let mut tick_rate = time::interval(Duration::from_millis(100));

        loop {
            // Draw UI
            self.terminal.draw(|frame| {
                self.ui.draw(frame, &self.workspace, &self.layout, &self.state);
            })?;

            // Handle events
            tokio::select! {
                _ = tick_rate.tick() => {
                    // Update workspace state
                    self.workspace.update().await?;
                }
                _ = tokio::time::sleep(Duration::from_millis(50)) => {
                    if event::poll(Duration::from_millis(0))? {
                        if let Event::Key(key) = event::read()? {
                            self.handle_key_event(key).await?;
                        }
                    }
                }
            }

            if self.should_quit {
                break;
            }
        }

        self.cleanup()?;
        Ok(())
    }

    async fn handle_key_event(&mut self, key: KeyEvent) -> Result<()> {
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
            (KeyCode::Char('q'), KeyModifiers::CONTROL) => {
                self.should_quit = true;
            }
            // New terminal
            (KeyCode::Char('t'), KeyModifiers::CONTROL) => {
                self.workspace.create_terminal(None).await?;
            }
            // Close terminal
            (KeyCode::Char('w'), KeyModifiers::CONTROL) => {
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
                self.state = AppState::Insert;
            }
            (KeyCode::Char(':'), KeyModifiers::NONE) => {
                self.state = AppState::Command;
            }
            (KeyCode::Char('v'), KeyModifiers::NONE) => {
                self.state = AppState::Visual;
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
        match key.code {
            KeyCode::Esc => {
                self.state = AppState::Normal;
            }
            _ => {
                // Forward key to active terminal
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