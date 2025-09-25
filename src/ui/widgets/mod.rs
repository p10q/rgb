use crate::terminal::TerminalEmulator;
use parking_lot::RwLock;
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Widget},
};
use std::sync::Arc;

pub struct TerminalWidget {
    emulator: Arc<RwLock<TerminalEmulator>>,
    active: bool,
    show_cursor: bool,
}

impl TerminalWidget {
    pub fn new(emulator: Arc<RwLock<TerminalEmulator>>) -> Self {
        Self {
            emulator,
            active: false,
            show_cursor: true,
        }
    }

    pub fn active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }

    pub fn show_cursor(mut self, show: bool) -> Self {
        self.show_cursor = show;
        self
    }
}

impl Widget for TerminalWidget {
    fn render(self, area: Rect, buf: &mut Buffer) {
        tracing::debug!("TerminalWidget::render called with area: {:?}", area);

        // Create border
        let border_style = if self.active {
            Style::default().fg(Color::Yellow)
        } else {
            Style::default().fg(Color::DarkGray)
        };

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(border_style)
            .title(if self.active { "Active Terminal" } else { "Terminal" });

        let inner_area = block.inner(area);
        block.render(area, buf);

        tracing::debug!("Inner area for terminal content: {:?}", inner_area);

        // Resize terminal if needed
        if inner_area.width > 0 && inner_area.height > 0 {
            {
                let mut emulator = self.emulator.write();
                match emulator.resize((inner_area.width, inner_area.height)) {
                    Ok(_) => {},
                    Err(e) => tracing::error!("Failed to resize terminal: {}", e),
                }
            } // Drop write lock here
        }

        // Get terminal content AFTER resize
        let emulator = self.emulator.read();
        let content = emulator.get_visible_content();

        tracing::debug!("Got {} lines of content from terminal", content.len());

        // Debug: Log first few lines of content
        let non_empty_lines: Vec<_> = content.iter()
            .enumerate()
            .filter(|(_, line)| !line.trim().is_empty())
            .collect();

        if non_empty_lines.is_empty() {
            tracing::warn!("No non-empty lines in terminal content!");
        } else {
            tracing::info!("Rendering {} non-empty lines:", non_empty_lines.len());
            for (idx, line) in non_empty_lines.iter().take(5) {
                tracing::info!("  Line {}: {:?}", idx, line.trim());
            }
        }

        // Clear the area first with background
        for y in 0..inner_area.height {
            for x in 0..inner_area.width {
                let x_pos = inner_area.x + x;
                let y_pos = inner_area.y + y;
                if let Some(cell) = buf.cell_mut((x_pos, y_pos)) {
                    cell.set_char(' ');
                    cell.set_style(Style::default().bg(Color::Black));
                }
            }
        }

        // Now draw the content
        for (y, line) in content.iter().enumerate() {
            if y >= inner_area.height as usize {
                break;
            }

            let y_pos = inner_area.y + y as u16;

            // Draw the entire line at once, handling empty chars
            for (x, ch) in line.chars().enumerate() {
                if x >= inner_area.width as usize {
                    break;
                }

                let x_pos = inner_area.x + x as u16;

                // Set character in buffer (including spaces)
                if let Some(cell) = buf.cell_mut((x_pos, y_pos)) {
                    // Make spaces visible with a different background
                    if ch == ' ' {
                        cell.set_char(' ');
                        cell.set_style(Style::default().fg(Color::White).bg(Color::Black));
                    } else {
                        cell.set_char(ch);
                        cell.set_style(Style::default().fg(Color::Green).bg(Color::Black));
                    }
                }
            }
        }

        // Add a test string to make sure rendering works at all
        if inner_area.width > 10 && inner_area.height > 0 {
            let test_msg = "DEBUG: Terminal Widget Active";
            for (i, ch) in test_msg.chars().enumerate() {
                if i < inner_area.width as usize {
                    if let Some(cell) = buf.cell_mut((inner_area.x + i as u16, inner_area.y)) {
                        cell.set_char(ch);
                        cell.set_style(Style::default().fg(Color::Red).bg(Color::Black));
                    }
                }
            }
        }

        // Draw cursor if active and show_cursor is true
        if self.active && self.show_cursor {
            let (cursor_x, cursor_y) = emulator.get_cursor_position();
            let cursor_x = inner_area.x + cursor_x.min(inner_area.width.saturating_sub(1));
            let cursor_y = inner_area.y + cursor_y.min(inner_area.height.saturating_sub(1));

            if let Some(cell) = buf.cell_mut((cursor_x, cursor_y)) {
                cell.set_style(
                    cell.style()
                        .add_modifier(Modifier::REVERSED)
                        .bg(Color::White),
                );
            }
        }
    }
}