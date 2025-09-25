pub mod widgets;
pub mod components;

use crate::app::AppState;
use crate::config::AppConfig;
use crate::layout::LayoutEngine;
use crate::workspace::{TerminalId, WorkspaceManager};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph, Tabs},
    Frame,
};

pub struct Ui {
    command_buffer: String,
    error_message: Option<String>,
    show_help: bool,
    show_git_panel: bool,
    show_file_explorer: bool,
}

impl Ui {
    pub fn new() -> Self {
        Self {
            command_buffer: String::new(),
            error_message: None,
            show_help: false,
            show_git_panel: false,  // Hidden by default to save space
            show_file_explorer: true,  // Shown by default
        }
    }

    pub fn draw(
        &self,
        frame: &mut Frame,
        workspace: &WorkspaceManager,
        layout: &mut LayoutEngine,
        state: &AppState,
    ) {
        tracing::info!("UI::draw called");
        let size = frame.area();
        tracing::info!("Frame area: {:?}", size);

        // Main layout: header, body, footer
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),  // Header
                Constraint::Min(10),     // Body
                Constraint::Length(1),  // Footer
            ])
            .split(size);

        // Draw header
        self.draw_header(frame, chunks[0], workspace);

        // Body layout: file explorer, terminals, git panel
        let body_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(if self.show_file_explorer && self.show_git_panel {
                vec![
                    Constraint::Percentage(20), // File explorer
                    Constraint::Percentage(60), // Terminals
                    Constraint::Percentage(20), // Git panel
                ]
            } else if self.show_file_explorer {
                vec![
                    Constraint::Percentage(25), // File explorer
                    Constraint::Percentage(75), // Terminals
                ]
            } else if self.show_git_panel {
                vec![
                    Constraint::Percentage(75), // Terminals
                    Constraint::Percentage(25), // Git panel
                ]
            } else {
                vec![Constraint::Percentage(100)] // Terminals only
            })
            .split(chunks[1]);

        let mut terminal_area_index = 0;

        // Draw file explorer if visible
        if self.show_file_explorer {
            self.draw_file_explorer(frame, body_chunks[0], workspace);
            terminal_area_index = 1;
        }

        // Draw terminals
        let terminal_area = body_chunks[terminal_area_index];
        self.draw_terminals(frame, terminal_area, workspace, layout);

        // Draw git panel if visible
        if self.show_git_panel {
            let git_index = if self.show_file_explorer { 2 } else { 1 };
            self.draw_git_panel(frame, body_chunks[git_index], workspace);
        }

        // Draw footer
        self.draw_footer(frame, chunks[2], state);

        // Draw command line if in command mode
        if matches!(state, AppState::Command) {
            self.draw_command_line(frame, size);
        }

        // Draw error message if present
        if let Some(ref error) = self.error_message {
            self.draw_error(frame, size, error);
        }

        // Draw help if visible
        if self.show_help {
            self.draw_help(frame, size);
        }
    }

    fn draw_header(&self, frame: &mut Frame, area: Rect, workspace: &WorkspaceManager) {
        let terminals = workspace.terminals();
        let terminal_count = terminals.len();
        let active_id = workspace.active_terminal_id();

        let header_text = vec![
            Span::raw("[Project: "),
            Span::styled("rgb-workspace", Style::default().fg(Color::Cyan)),
            Span::raw("] "),
            Span::raw("[Terminals: "),
            Span::styled(
                terminal_count.to_string(),
                Style::default().fg(Color::Green),
            ),
            Span::raw("] "),
            if let Some(id) = active_id {
                Span::styled(
                    format!("[Active: {}]", &id.to_string()[..8]),
                    Style::default().fg(Color::Yellow),
                )
            } else {
                Span::raw("")
            },
        ];

        let header = Paragraph::new(Line::from(header_text))
            .style(Style::default().bg(Color::DarkGray));

        frame.render_widget(header, area);
    }

    fn draw_terminals(
        &self,
        frame: &mut Frame,
        area: Rect,
        workspace: &WorkspaceManager,
        layout: &mut LayoutEngine,
    ) {
        tracing::info!("draw_terminals called with area: {:?}", area);

        let terminals = workspace.terminals();
        tracing::info!("Found {} terminals", terminals.len());

        let terminal_ids: Vec<TerminalId> = terminals.iter().map(|t| t.id).collect();

        // Calculate layout for terminals
        let terminal_rects = layout.calculate_layout(area, &terminal_ids);
        tracing::info!("Layout calculated {} rectangles", terminal_rects.len());

        // Draw each terminal
        for (terminal_id, rect) in terminal_rects {
            tracing::info!("Drawing terminal {:?} in rect {:?}", terminal_id, rect);

            if let Some(emulator) = workspace.get_terminal_emulator(terminal_id) {
                let is_active = workspace.active_terminal_id() == Some(terminal_id);
                tracing::info!("Terminal is_active: {}", is_active);

                // Create terminal widget
                let terminal_widget = widgets::TerminalWidget::new(emulator.clone())
                    .active(is_active);

                frame.render_widget(terminal_widget, rect);
                tracing::info!("Widget rendered for terminal {:?}", terminal_id);
            } else {
                tracing::warn!("No emulator found for terminal {:?}", terminal_id);
            }
        }
    }

    fn draw_file_explorer(&self, frame: &mut Frame, area: Rect, _workspace: &WorkspaceManager) {
        let block = Block::default()
            .title("Files")
            .borders(Borders::ALL)
            .style(Style::default().fg(Color::White));

        // TODO: Implement actual file tree
        let items = vec![
            ListItem::new("▼ src/"),
            ListItem::new("  ▶ app/"),
            ListItem::new("  ▶ config/"),
            ListItem::new("  ▶ git/"),
            ListItem::new("  ▶ layout/"),
            ListItem::new("  ▶ monitor/"),
            ListItem::new("  ▶ terminal/"),
            ListItem::new("  ▶ ui/"),
            ListItem::new("  ▶ workspace/"),
            ListItem::new("  • main.rs"),
        ];

        let list = List::new(items)
            .block(block)
            .style(Style::default().fg(Color::White))
            .highlight_style(
                Style::default()
                    .add_modifier(Modifier::BOLD)
                    .bg(Color::DarkGray),
            );

        frame.render_widget(list, area);
    }

    fn draw_git_panel(&self, frame: &mut Frame, area: Rect, _workspace: &WorkspaceManager) {
        let block = Block::default()
            .title("Git")
            .borders(Borders::ALL)
            .style(Style::default().fg(Color::White));

        // TODO: Implement actual git status
        let items = vec![
            ListItem::new("Changes:"),
            ListItem::new(Line::from(vec![
                Span::styled("M ", Style::default().fg(Color::Yellow)),
                Span::raw("src/main.rs"),
            ])),
            ListItem::new(Line::from(vec![
                Span::styled("A ", Style::default().fg(Color::Green)),
                Span::raw("src/test.rs"),
            ])),
            ListItem::new(""),
            ListItem::new("Timeline:"),
            ListItem::new("10:45 commit"),
            ListItem::new("10:32 edit"),
        ];

        let list = List::new(items).block(block);

        frame.render_widget(list, area);
    }

    fn draw_footer(&self, frame: &mut Frame, area: Rect, state: &AppState) {
        let mode_text = match state {
            AppState::Normal => "NORMAL",
            AppState::Insert => "INSERT",
            AppState::Command => "COMMAND",
            AppState::Visual => "VISUAL",
        };

        let footer_text = vec![
            Span::raw("["),
            Span::styled("Ctrl+Q", Style::default().fg(Color::Yellow)),
            Span::raw(" Quit] ["),
            Span::styled("Tab", Style::default().fg(Color::Yellow)),
            Span::raw(" Switch] ["),
            Span::styled("Ctrl+T", Style::default().fg(Color::Yellow)),
            Span::raw(" New] ["),
            Span::styled("F1", Style::default().fg(Color::Yellow)),
            Span::raw(" Help] [Mode: "),
            Span::styled(mode_text, Style::default().fg(Color::Cyan)),
            Span::raw("]"),
        ];

        let footer = Paragraph::new(Line::from(footer_text))
            .style(Style::default().bg(Color::DarkGray));

        frame.render_widget(footer, area);
    }

    fn draw_command_line(&self, frame: &mut Frame, _size: Rect) {
        let area = centered_rect(60, 3, frame.area());

        let block = Block::default()
            .title("Command")
            .borders(Borders::ALL)
            .style(Style::default().fg(Color::Yellow));

        let input = Paragraph::new(format!(":{}", self.command_buffer))
            .block(block)
            .style(Style::default());

        frame.render_widget(input, area);
    }

    fn draw_error(&self, frame: &mut Frame, _size: Rect, message: &str) {
        let area = centered_rect(50, 5, frame.area());

        let block = Block::default()
            .title("Error")
            .borders(Borders::ALL)
            .style(Style::default().fg(Color::Red));

        let text = Paragraph::new(message)
            .block(block)
            .wrap(ratatui::widgets::Wrap { trim: true });

        frame.render_widget(text, area);
    }

    fn draw_help(&self, frame: &mut Frame, _size: Rect) {
        let area = centered_rect(60, 20, frame.area());

        let block = Block::default()
            .title("Help")
            .borders(Borders::ALL)
            .style(Style::default().fg(Color::Cyan));

        let help_text = vec![
            "Navigation:",
            "  h/j/k/l    - Move between terminals",
            "  Tab        - Next terminal",
            "  Shift+Tab  - Previous terminal",
            "",
            "Terminal Management:",
            "  Ctrl+T     - New terminal",
            "  Ctrl+W     - Close terminal",
            "",
            "Modes:",
            "  i          - Insert mode",
            "  :          - Command mode",
            "  v          - Visual mode",
            "  Esc        - Normal mode",
            "",
            "Press Esc to close help",
        ];

        let text = Paragraph::new(help_text.join("\n"))
            .block(block)
            .style(Style::default());

        frame.render_widget(text, area);
    }

    pub fn command_push(&mut self, c: char) {
        self.command_buffer.push(c);
    }

    pub fn command_backspace(&mut self) {
        self.command_buffer.pop();
    }

    pub fn get_command(&self) -> String {
        self.command_buffer.clone()
    }

    pub fn clear_command(&mut self) {
        self.command_buffer.clear();
    }

    pub fn show_error(&mut self, message: &str) {
        self.error_message = Some(message.to_string());
    }

    pub fn clear_error(&mut self) {
        self.error_message = None;
    }

    pub fn toggle_help(&mut self) {
        self.show_help = !self.show_help;
    }

    pub fn toggle_git_panel(&mut self) {
        self.show_git_panel = !self.show_git_panel;
    }

    pub fn toggle_file_explorer(&mut self) {
        self.show_file_explorer = !self.show_file_explorer;
    }

    pub fn show_worktree_info(&self, _workspace: &WorkspaceManager) {
        // TODO: Implement worktree info display
    }

    pub fn show_commit_interface(&self) {
        // TODO: Implement commit interface
    }

    pub fn show_config_editor(&self, _config: &AppConfig) {
        // TODO: Implement config editor
    }
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}