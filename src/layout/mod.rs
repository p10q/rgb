use crate::workspace::{TerminalId, WorkspaceManager};
use anyhow::Result;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub enum LayoutMode {
    Tiled(TileLayout),
    Floating,
    Tabbed,
    Stacked,
}

#[derive(Debug, Clone)]
pub enum TileLayout {
    Vertical,
    Horizontal,
    Grid { cols: usize },
    Spiral,
}

pub struct LayoutEngine {
    mode: LayoutMode,
    containers: Vec<Container>,
    focus_stack: Vec<ContainerId>,
    terminal_positions: HashMap<TerminalId, Rect>,
}

pub type ContainerId = usize;

#[derive(Debug, Clone)]
pub struct Container {
    pub id: ContainerId,
    pub content: ContainerContent,
    pub rect: Rect,
    pub resizable: bool,
    pub min_size: Size,
}

#[derive(Debug, Clone)]
pub enum ContainerContent {
    Terminal(TerminalId),
    Split {
        direction: Direction,
        children: Vec<ContainerId>,
        ratios: Vec<u16>,
    },
}

#[derive(Debug, Clone)]
pub struct Size {
    pub width: u16,
    pub height: u16,
}

impl LayoutEngine {
    pub fn new() -> Self {
        Self {
            mode: LayoutMode::Tiled(TileLayout::Grid { cols: 2 }),
            containers: Vec::new(),
            focus_stack: Vec::new(),
            terminal_positions: HashMap::new(),
        }
    }

    pub fn calculate_layout(
        &mut self,
        area: Rect,
        terminals: &[TerminalId],
    ) -> HashMap<TerminalId, Rect> {
        self.terminal_positions.clear();

        if terminals.is_empty() {
            return self.terminal_positions.clone();
        }

        match self.mode.clone() {
            LayoutMode::Tiled(tile_layout) => {
                self.calculate_tiled_layout(area, terminals, &tile_layout)
            }
            LayoutMode::Floating => self.calculate_floating_layout(area, terminals),
            LayoutMode::Tabbed => self.calculate_tabbed_layout(area, terminals),
            LayoutMode::Stacked => self.calculate_stacked_layout(area, terminals),
        }

        self.terminal_positions.clone()
    }

    fn calculate_tiled_layout(
        &mut self,
        area: Rect,
        terminals: &[TerminalId],
        tile_layout: &TileLayout,
    ) {
        match tile_layout {
            TileLayout::Vertical => {
                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints(vec![Constraint::Ratio(1, terminals.len() as u32); terminals.len()])
                    .split(area);

                for (i, terminal_id) in terminals.iter().enumerate() {
                    self.terminal_positions.insert(*terminal_id, chunks[i]);
                }
            }
            TileLayout::Horizontal => {
                let chunks = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints(vec![Constraint::Ratio(1, terminals.len() as u32); terminals.len()])
                    .split(area);

                for (i, terminal_id) in terminals.iter().enumerate() {
                    self.terminal_positions.insert(*terminal_id, chunks[i]);
                }
            }
            TileLayout::Grid { cols } => {
                let cols = *cols.min(&terminals.len()).max(&1);
                let rows = (terminals.len() + cols - 1) / cols;

                let row_chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints(vec![Constraint::Ratio(1, rows as u32); rows])
                    .split(area);

                let mut terminal_iter = terminals.iter();
                for row_chunk in row_chunks.iter().take(rows) {
                    let terminals_in_row = terminal_iter.len().min(cols);
                    let col_chunks = Layout::default()
                        .direction(Direction::Horizontal)
                        .constraints(vec![Constraint::Ratio(1, terminals_in_row as u32); terminals_in_row])
                        .split(*row_chunk);

                    for col_chunk in col_chunks.iter().take(terminals_in_row) {
                        if let Some(terminal_id) = terminal_iter.next() {
                            self.terminal_positions.insert(*terminal_id, *col_chunk);
                        }
                    }
                }
            }
            TileLayout::Spiral => {
                self.calculate_spiral_layout(area, terminals);
            }
        }
    }

    fn calculate_spiral_layout(&mut self, area: Rect, terminals: &[TerminalId]) {
        if terminals.is_empty() {
            return;
        }

        if terminals.len() == 1 {
            self.terminal_positions.insert(terminals[0], area);
            return;
        }

        let mut remaining = area;
        let mut direction = Direction::Horizontal;
        let mut terminals_iter = terminals.iter().peekable();

        while terminals_iter.peek().is_some() {
            let count = if terminals_iter.len() == 1 {
                1
            } else {
                2.min(terminals_iter.len())
            };

            let chunks = Layout::default()
                .direction(direction)
                .constraints(if count == 1 {
                    vec![Constraint::Percentage(100)]
                } else {
                    vec![Constraint::Percentage(50), Constraint::Percentage(50)]
                })
                .split(remaining);

            if let Some(terminal_id) = terminals_iter.next() {
                self.terminal_positions.insert(*terminal_id, chunks[0]);
            }

            if count > 1 {
                remaining = chunks[1];
                direction = match direction {
                    Direction::Horizontal => Direction::Vertical,
                    Direction::Vertical => Direction::Horizontal,
                };
            }
        }
    }

    fn calculate_floating_layout(&mut self, area: Rect, terminals: &[TerminalId]) {
        // Simple cascade for now
        let offset = 2;
        for (i, terminal_id) in terminals.iter().enumerate() {
            let x_offset = (i as u16 * offset) % (area.width / 4);
            let y_offset = (i as u16 * offset) % (area.height / 4);

            let rect = Rect {
                x: area.x + x_offset,
                y: area.y + y_offset,
                width: area.width.saturating_sub(x_offset * 2).max(40),
                height: area.height.saturating_sub(y_offset * 2).max(10),
            };

            self.terminal_positions.insert(*terminal_id, rect);
        }
    }

    fn calculate_tabbed_layout(&mut self, area: Rect, terminals: &[TerminalId]) {
        // All terminals get the full area, UI will handle tab switching
        for terminal_id in terminals {
            self.terminal_positions.insert(*terminal_id, area);
        }
    }

    fn calculate_stacked_layout(&mut self, area: Rect, terminals: &[TerminalId]) {
        // Similar to tabbed but with title bars visible
        let title_height = 2;
        let stacked_height = title_height * terminals.len().saturating_sub(1) as u16;

        if let Some(active) = terminals.last() {
            let content_area = Rect {
                x: area.x,
                y: area.y + stacked_height,
                width: area.width,
                height: area.height.saturating_sub(stacked_height),
            };
            self.terminal_positions.insert(*active, content_area);
        }

        // Other terminals are hidden but positioned
        for terminal_id in terminals.iter().take(terminals.len().saturating_sub(1)) {
            self.terminal_positions.insert(*terminal_id, Rect {
                x: area.x,
                y: area.y,
                width: area.width,
                height: 0,
            });
        }
    }

    pub fn set_mode(&mut self, mode: LayoutMode) {
        self.mode = mode;
    }

    pub fn apply_layout(&mut self, layout_name: &str) -> Result<()> {
        let mode = match layout_name {
            "vertical" => LayoutMode::Tiled(TileLayout::Vertical),
            "horizontal" => LayoutMode::Tiled(TileLayout::Horizontal),
            "grid" => LayoutMode::Tiled(TileLayout::Grid { cols: 2 }),
            "spiral" => LayoutMode::Tiled(TileLayout::Spiral),
            "floating" => LayoutMode::Floating,
            "tabbed" => LayoutMode::Tabbed,
            "stacked" => LayoutMode::Stacked,
            _ => anyhow::bail!("Unknown layout: {}", layout_name),
        };

        self.mode = mode;
        Ok(())
    }

    pub fn focus_left(&mut self, workspace: &mut WorkspaceManager) {
        self.focus_direction(workspace, FocusDirection::Left);
    }

    pub fn focus_right(&mut self, workspace: &mut WorkspaceManager) {
        self.focus_direction(workspace, FocusDirection::Right);
    }

    pub fn focus_up(&mut self, workspace: &mut WorkspaceManager) {
        self.focus_direction(workspace, FocusDirection::Up);
    }

    pub fn focus_down(&mut self, workspace: &mut WorkspaceManager) {
        self.focus_direction(workspace, FocusDirection::Down);
    }

    fn focus_direction(&mut self, workspace: &mut WorkspaceManager, direction: FocusDirection) {
        if let Some(current_id) = workspace.active_terminal_id() {
            if let Some(current_rect) = self.terminal_positions.get(&current_id) {
                let best_terminal = self.find_best_terminal_in_direction(
                    current_id,
                    *current_rect,
                    direction,
                );

                if let Some(new_id) = best_terminal {
                    workspace.set_active_terminal(new_id);
                }
            }
        }
    }

    fn find_best_terminal_in_direction(
        &self,
        current_id: TerminalId,
        current_rect: Rect,
        direction: FocusDirection,
    ) -> Option<TerminalId> {
        let current_center = rect_center(&current_rect);
        let mut best_terminal = None;
        let mut best_distance = f32::MAX;

        for (terminal_id, rect) in &self.terminal_positions {
            if *terminal_id == current_id {
                continue;
            }

            let other_center = rect_center(rect);
            if !is_in_direction(&current_center, &other_center, direction) {
                continue;
            }

            let distance = euclidean_distance(&current_center, &other_center);
            if distance < best_distance {
                best_distance = distance;
                best_terminal = Some(*terminal_id);
            }
        }

        best_terminal
    }

    pub fn get_terminal_rect(&self, id: TerminalId) -> Option<Rect> {
        self.terminal_positions.get(&id).copied()
    }
}

#[derive(Debug, Clone, Copy)]
enum FocusDirection {
    Left,
    Right,
    Up,
    Down,
}

fn rect_center(rect: &Rect) -> (f32, f32) {
    (
        rect.x as f32 + rect.width as f32 / 2.0,
        rect.y as f32 + rect.height as f32 / 2.0,
    )
}

fn is_in_direction(from: &(f32, f32), to: &(f32, f32), direction: FocusDirection) -> bool {
    match direction {
        FocusDirection::Left => to.0 < from.0,
        FocusDirection::Right => to.0 > from.0,
        FocusDirection::Up => to.1 < from.1,
        FocusDirection::Down => to.1 > from.1,
    }
}

fn euclidean_distance(a: &(f32, f32), b: &(f32, f32)) -> f32 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}