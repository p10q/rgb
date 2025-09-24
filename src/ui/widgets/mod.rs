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

        // Render terminal content
        let emulator = self.emulator.read();
        let content = emulator.get_visible_content();

        // Resize terminal if needed
        drop(emulator);
        if inner_area.width > 0 && inner_area.height > 0 {
            let mut emulator = self.emulator.write();
            let _ = emulator.resize((inner_area.width, inner_area.height));
        }

        // Draw terminal content
        let emulator = self.emulator.read();
        for (y, line) in content.iter().enumerate() {
            if y >= inner_area.height as usize {
                break;
            }

            let y_pos = inner_area.y + y as u16;

            for (x, ch) in line.chars().enumerate() {
                if x >= inner_area.width as usize {
                    break;
                }

                let x_pos = inner_area.x + x as u16;

                // Set character in buffer
                if let Some(cell) = buf.cell_mut((x_pos, y_pos)) {
                    cell.set_char(ch);
                    cell.set_style(Style::default().fg(Color::White));
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