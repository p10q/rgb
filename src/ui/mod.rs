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
use std::fs;
use std::path::Path;

pub struct Ui {
    command_buffer: String,
    error_message: Option<String>,
    show_help: bool,
    show_git_panel: bool,
    show_file_explorer: bool,
    file_explorer_selected: usize,  // Index of selected item in file explorer
    file_tree: Vec<FileTreeItem>,
    file_explorer_area: Option<Rect>,  // Track the file explorer area for mouse clicks
}

#[derive(Clone, Debug)]
struct FileTreeItem {
    name: String,
    is_dir: bool,
    is_expanded: bool,
    depth: usize,
    path: String,
}

impl Ui {
    pub fn new() -> Self {
        // Build initial file tree - start with root directory
        let mut file_tree = vec![
            FileTreeItem {
                name: "./".to_string(),
                is_dir: true,
                is_expanded: false,  // Start collapsed, expand on demand
                depth: 0,
                path: ".".to_string(),
            },
        ];

        // Try to load root directory contents initially
        let mut ui = Self {
            command_buffer: String::new(),
            error_message: None,
            show_help: false,
            show_git_panel: false,  // Hidden by default to save space
            show_file_explorer: true,  // Shown by default
            file_explorer_selected: 0,
            file_tree,
            file_explorer_area: None,
        };

        // Expand root directory to show initial contents
        ui.file_tree[0].is_expanded = true;
        ui.load_directory_contents(0);

        ui
    }

    pub fn draw(
        &mut self,
        frame: &mut Frame,
        workspace: &WorkspaceManager,
        layout: &mut LayoutEngine,
        state: &AppState,
    ) {
        tracing::trace!("UI::draw called");
        let size = frame.area();

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
            Span::styled("rgb-workspace", Style::default().fg(Color::Blue)),
            Span::raw("] "),
            Span::raw("[Terminals: "),
            Span::styled(
                terminal_count.to_string(),
                Style::default().fg(Color::DarkGray),
            ),
            Span::raw("] "),
            if let Some(id) = active_id {
                Span::styled(
                    format!("[Active: {}]", &id.to_string()[..8]),
                    Style::default().fg(Color::Magenta),
                )
            } else {
                Span::raw("")
            },
        ];

        let header = Paragraph::new(Line::from(header_text))
            .style(Style::default().bg(Color::Gray).fg(Color::Black));

        frame.render_widget(header, area);
    }

    fn draw_terminals(
        &self,
        frame: &mut Frame,
        area: Rect,
        workspace: &WorkspaceManager,
        layout: &mut LayoutEngine,
    ) {
        tracing::trace!("draw_terminals called with area: {:?}", area);

        let terminals = workspace.terminals();
        tracing::trace!("Found {} terminals", terminals.len());

        let terminal_ids: Vec<TerminalId> = terminals.iter().map(|t| t.id).collect();

        // Calculate layout for terminals
        let terminal_rects = layout.calculate_layout(area, &terminal_ids);
        tracing::trace!("Layout calculated {} rectangles", terminal_rects.len());

        // Draw each terminal
        for (terminal_id, rect) in terminal_rects {
            tracing::trace!("Drawing terminal {:?} in rect {:?}", terminal_id, rect);

            if let Some(emulator) = workspace.get_terminal_emulator(terminal_id) {
                let is_active = workspace.active_terminal_id() == Some(terminal_id);
                tracing::trace!("Terminal is_active: {}", is_active);

                // Create terminal widget
                let terminal_widget = widgets::TerminalWidget::new(emulator.clone())
                    .active(is_active);

                frame.render_widget(terminal_widget, rect);
                tracing::trace!("Widget rendered for terminal {:?}", terminal_id);
            } else {
                tracing::warn!("No emulator found for terminal {:?}", terminal_id);
            }
        }
    }

    fn draw_file_explorer(&mut self, frame: &mut Frame, area: Rect, _workspace: &WorkspaceManager) {
        // Store the area for mouse click handling
        self.file_explorer_area = Some(area);
        // First, fill the entire area with a light background
        let bg_block = Block::default()
            .style(Style::default().bg(Color::White));
        frame.render_widget(bg_block, area);

        let block = Block::default()
            .title("Files [j/k:nav, Enter:open/expand, h/l:collapse/expand]")
            .borders(Borders::ALL)
            .style(Style::default()
                .fg(Color::Black)
                .bg(Color::White));

        // Build visible items from file tree
        let mut items = Vec::new();
        for (idx, item) in self.file_tree.iter().enumerate() {
            let indent = "  ".repeat(item.depth);
            let icon = if item.is_dir {
                if item.is_expanded { "▼" } else { "▶" }
            } else {
                "•"
            };

            let style = if idx == self.file_explorer_selected {
                Style::default()
                    .fg(Color::Blue)
                    .bg(Color::LightBlue)
                    .add_modifier(Modifier::BOLD)
            } else if item.is_dir {
                Style::default()
                    .fg(Color::Blue)
                    .bg(Color::White)
            } else {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::White)
            };

            items.push(ListItem::new(format!("{}{} {}", indent, icon, item.name)).style(style));
        }

        let list = List::new(items)
            .block(block)
            .style(Style::default()
                .fg(Color::Black)
                .bg(Color::White))
            .highlight_style(
                Style::default()
                    .add_modifier(Modifier::BOLD)
                    .bg(Color::Gray),
            )
            .highlight_symbol("> ");

        frame.render_widget(list, area);
    }

    fn draw_git_panel(&self, frame: &mut Frame, area: Rect, _workspace: &WorkspaceManager) {
        let block = Block::default()
            .title("Git")
            .borders(Borders::ALL)
            .style(Style::default().fg(Color::Black).bg(Color::White));

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
            Span::styled("Alt+I", Style::default().fg(Color::Blue)),
            Span::raw(" Insert] ["),
            Span::styled("jj/jk", Style::default().fg(Color::Blue)),
            Span::raw(" Normal] ["),
            Span::styled("Ctrl+T", Style::default().fg(Color::Blue)),
            Span::raw(" New] ["),
            Span::styled("Ctrl+F", Style::default().fg(Color::Blue)),
            Span::raw(" Files] ["),
            Span::styled("?", Style::default().fg(Color::Blue)),
            Span::raw(" Help] [Mode: "),
            Span::styled(mode_text, Style::default().fg(Color::Magenta)),
            Span::raw("]"),
        ];

        let footer = Paragraph::new(Line::from(footer_text))
            .style(Style::default().bg(Color::Gray).fg(Color::Black));

        frame.render_widget(footer, area);
    }

    fn draw_command_line(&self, frame: &mut Frame, _size: Rect) {
        let area = centered_rect(60, 3, frame.area());

        let block = Block::default()
            .title("Command")
            .borders(Borders::ALL)
            .style(Style::default().fg(Color::Blue).bg(Color::White));

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
        let area = centered_rect(60, 25, frame.area());

        let block = Block::default()
            .title("Help (Press ? or Esc to close)")
            .borders(Borders::ALL)
            .style(Style::default().fg(Color::Blue).bg(Color::White));

        let help_text = vec![
            "Navigation:",
            "  h/j/k/l    - Move between terminals/files",
            "  Tab        - Next terminal",
            "  Shift+Tab  - Previous terminal",
            "",
            "Terminal Management:",
            "  Ctrl+T     - New terminal",
            "  Ctrl+W     - Close terminal/exit files",
            "  Ctrl+Q     - Quit application",
            "",
            "File Explorer:",
            "  Ctrl+F     - Toggle focus to/from files",
            "  Ctrl+E     - Toggle file explorer visibility",
            "  j/k        - Navigate files (when focused)",
            "  h/l        - Collapse/expand folders",
            "  Enter      - Open file in new terminal",
            "",
            "Modes:",
            "  Alt+I      - Insert mode (type in terminal)",
            "  jj or jk   - Quick exit to Normal mode (vim-style)",
            "  Ctrl+[     - Exit to Normal mode (works like Esc)",
            "  Alt+F      - Also exits to Normal mode",
            "  :          - Command mode",
            "  v          - Visual mode",
            "  ?          - Toggle this help",
            "",
            "Note: Use Alt+I instead of 'i' to avoid conflicts",
            "with terminal programs like vim.",
            "",
            "Press ? or Esc to close help",
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

    pub fn is_help_visible(&self) -> bool {
        self.show_help
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

    pub fn file_explorer_move_up(&mut self) {
        if self.file_explorer_selected > 0 {
            self.file_explorer_selected -= 1;
        }
    }

    pub fn file_explorer_move_down(&mut self) {
        if self.file_explorer_selected < self.file_tree.len() - 1 {
            self.file_explorer_selected += 1;
        }
    }

    pub fn file_explorer_toggle_expand(&mut self) {
        if self.file_explorer_selected < self.file_tree.len() {
            let selected_idx = self.file_explorer_selected;
            let item = self.file_tree[selected_idx].clone();

            if item.is_dir {
                let new_state = !item.is_expanded;
                self.file_tree[selected_idx].is_expanded = new_state;

                if new_state {
                    // Expanding - load directory contents
                    self.load_directory_contents(selected_idx);
                } else {
                    // Collapsing - remove child items
                    self.collapse_directory(selected_idx);
                }
            }
        }
    }

    fn load_directory_contents(&mut self, dir_idx: usize) {
        let dir_item = &self.file_tree[dir_idx];
        let dir_path = &dir_item.path;
        let dir_depth = dir_item.depth;

        // Read directory contents
        if let Ok(entries) = fs::read_dir(dir_path) {
            let mut items_to_insert = Vec::new();

            // Collect and sort entries
            let mut entries: Vec<_> = entries.filter_map(|e| e.ok()).collect();
            entries.sort_by_key(|e| {
                let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
                let name = e.file_name();
                (!is_dir, name)  // Directories first, then files
            });

            for entry in entries {
                let path = entry.path();
                let name = entry.file_name().to_string_lossy().to_string();
                let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);

                // Skip hidden files starting with .
                if name.starts_with('.') {
                    continue;
                }

                items_to_insert.push(FileTreeItem {
                    name: if is_dir { format!("{}/", name) } else { name },
                    is_dir,
                    is_expanded: false,
                    depth: dir_depth + 1,
                    path: path.to_string_lossy().to_string(),
                });
            }

            // Insert items after the parent directory
            let insert_pos = dir_idx + 1;
            for (i, item) in items_to_insert.into_iter().enumerate() {
                self.file_tree.insert(insert_pos + i, item);
            }
        }
    }

    fn collapse_directory(&mut self, dir_idx: usize) {
        let dir_depth = self.file_tree[dir_idx].depth;

        // Remove all items with depth > dir_depth that come after dir_idx
        let mut i = dir_idx + 1;
        while i < self.file_tree.len() {
            if self.file_tree[i].depth > dir_depth {
                self.file_tree.remove(i);
            } else {
                break;  // Reached a sibling or parent level item
            }
        }
    }

    pub fn file_explorer_open(&mut self) -> Option<String> {
        if self.file_explorer_selected < self.file_tree.len() {
            let item = &self.file_tree[self.file_explorer_selected];
            if !item.is_dir {
                return Some(item.path.clone());
            } else {
                // Toggle expansion for directories
                self.file_explorer_toggle_expand();
            }
        }
        None
    }

    pub fn get_file_explorer_area(&self) -> Option<Rect> {
        self.file_explorer_area
    }

    pub fn handle_file_explorer_click(&mut self, x: u16, y: u16) {
        if let Some(area) = self.file_explorer_area {
            // Calculate which item was clicked based on y position
            let relative_y = y.saturating_sub(area.y + 1);  // +1 for the border
            let clicked_index = relative_y as usize;

            // Check if click is within the visible items
            if clicked_index < self.file_tree.len() {
                self.file_explorer_selected = clicked_index;
                // Double-click logic could be added here to open files
            }
        }
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