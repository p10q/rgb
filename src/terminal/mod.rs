use anyhow::Result;
use crossterm::event::KeyEvent;
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::Arc;
use tokio::sync::Mutex;
use vte::{Params, Parser, Perform};

pub struct TerminalEmulator {
    master: Box<dyn MasterPty + Send>,
    parser: Parser,
    size: (u16, u16),
    output_buffer: Vec<u8>,
    display_buffer: Vec<Vec<char>>,
    cursor_pos: (u16, u16),
    active_files: Vec<String>,
}

impl TerminalEmulator {
    pub fn new(command: &str, working_dir: &Path, size: (u16, u16)) -> Result<Self> {
        let pty_system = native_pty_system();
        let pty_size = PtySize {
            rows: size.1,
            cols: size.0,
            pixel_width: 0,
            pixel_height: 0,
        };

        let pair = pty_system.openpty(pty_size)?;

        let mut cmd = CommandBuilder::new(command);
        cmd.cwd(working_dir);
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");

        let _child = pair.slave.spawn_command(cmd)?;

        // Initialize display buffer
        let mut display_buffer = Vec::with_capacity(size.1 as usize);
        for _ in 0..size.1 {
            display_buffer.push(vec![' '; size.0 as usize]);
        }

        Ok(TerminalEmulator {
            master: pair.master,
            parser: Parser::new(),
            size,
            output_buffer: Vec::new(),
            display_buffer,
            cursor_pos: (0, 0),
            active_files: Vec::new(),
        })
    }

    pub fn write(&mut self, data: &[u8]) -> Result<()> {
        self.master.take_writer()?.write_all(data)?;
        Ok(())
    }

    pub fn resize(&mut self, size: (u16, u16)) -> Result<()> {
        self.size = size;
        let pty_size = PtySize {
            rows: size.1,
            cols: size.0,
            pixel_width: 0,
            pixel_height: 0,
        };
        self.master.resize(pty_size)?;

        // Resize display buffer
        self.display_buffer.clear();
        for _ in 0..size.1 {
            self.display_buffer.push(vec![' '; size.0 as usize]);
        }

        Ok(())
    }

    pub fn update(&mut self) -> Result<()> {
        let mut reader = self.master.try_clone_reader()?;
        reader.set_non_blocking(true)?;

        let mut buffer = [0u8; 4096];
        match reader.read(&mut buffer) {
            Ok(n) if n > 0 => {
                self.output_buffer.extend_from_slice(&buffer[..n]);

                // Process through VTE parser
                for byte in &buffer[..n] {
                    self.parser.advance(self, *byte);
                }

                // Parse for file references
                self.parse_for_files(&buffer[..n]);
            }
            Ok(_) | Err(_) => {}
        }

        Ok(())
    }

    fn parse_for_files(&mut self, data: &[u8]) {
        if let Ok(text) = std::str::from_utf8(data) {
            // Look for file patterns
            let patterns = [
                r"([a-zA-Z0-9_/.-]+\.[a-zA-Z]+):(\d+)",
                r"(?i)(?:error|warning) in ([a-zA-Z0-9_/.-]+\.[a-zA-Z]+)",
                r"(?i)editing ([a-zA-Z0-9_/.-]+\.[a-zA-Z]+)",
            ];

            for pattern in &patterns {
                if let Ok(re) = regex::Regex::new(pattern) {
                    for cap in re.captures_iter(text) {
                        if let Some(file) = cap.get(1) {
                            let file_path = file.as_str().to_string();
                            if !self.active_files.contains(&file_path) {
                                self.active_files.push(file_path);
                            }
                        }
                    }
                }
            }
        }
    }

    pub fn handle_key_event(&mut self, key: KeyEvent) -> Result<()> {
        let bytes = convert_key_to_bytes(key);
        if !bytes.is_empty() {
            self.write(&bytes)?;
        }
        Ok(())
    }

    pub fn get_visible_content(&self) -> Vec<String> {
        self.display_buffer
            .iter()
            .map(|line| line.iter().collect::<String>())
            .collect()
    }

    pub fn get_cursor_position(&self) -> (u16, u16) {
        self.cursor_pos
    }

    pub fn scroll(&mut self, _lines: isize) {
        // TODO: Implement scrollback
    }

    pub fn get_active_files(&self) -> &[String] {
        &self.active_files
    }
}

// VTE Perform implementation for processing terminal sequences
impl Perform for TerminalEmulator {
    fn print(&mut self, c: char) {
        let (x, y) = self.cursor_pos;
        if (y as usize) < self.display_buffer.len() && (x as usize) < self.display_buffer[y as usize].len() {
            self.display_buffer[y as usize][x as usize] = c;
            self.cursor_pos.0 = (x + 1).min(self.size.0 - 1);
        }
    }

    fn execute(&mut self, byte: u8) {
        match byte {
            b'\n' => {
                self.cursor_pos.1 = (self.cursor_pos.1 + 1).min(self.size.1 - 1);
                self.cursor_pos.0 = 0;
            }
            b'\r' => {
                self.cursor_pos.0 = 0;
            }
            b'\t' => {
                self.cursor_pos.0 = ((self.cursor_pos.0 / 8) + 1) * 8;
                if self.cursor_pos.0 >= self.size.0 {
                    self.cursor_pos.0 = self.size.0 - 1;
                }
            }
            0x08 => {
                // Backspace
                if self.cursor_pos.0 > 0 {
                    self.cursor_pos.0 -= 1;
                }
            }
            _ => {}
        }
    }

    fn hook(&mut self, _params: &Params, _intermediates: &[u8], _ignore: bool, _c: char) {
        // Not implemented for basic terminal
    }

    fn put(&mut self, _byte: u8) {
        // Not implemented for basic terminal
    }

    fn unhook(&mut self) {
        // Not implemented for basic terminal
    }

    fn osc_dispatch(&mut self, _params: &[&[u8]], _bell_terminated: bool) {
        // Not implemented for basic terminal
    }

    fn csi_dispatch(&mut self, params: &Params, _intermediates: &[u8], _ignore: bool, c: char) {
        match c {
            'H' | 'f' => {
                // Cursor position
                let row = params.iter().next().and_then(|p| p[0]).unwrap_or(1) as u16;
                let col = params.iter().nth(1).and_then(|p| p[0]).unwrap_or(1) as u16;
                self.cursor_pos = (col.saturating_sub(1), row.saturating_sub(1));
            }
            'A' => {
                // Cursor up
                let n = params.iter().next().and_then(|p| p[0]).unwrap_or(1) as u16;
                self.cursor_pos.1 = self.cursor_pos.1.saturating_sub(n);
            }
            'B' => {
                // Cursor down
                let n = params.iter().next().and_then(|p| p[0]).unwrap_or(1) as u16;
                self.cursor_pos.1 = (self.cursor_pos.1 + n).min(self.size.1 - 1);
            }
            'C' => {
                // Cursor forward
                let n = params.iter().next().and_then(|p| p[0]).unwrap_or(1) as u16;
                self.cursor_pos.0 = (self.cursor_pos.0 + n).min(self.size.0 - 1);
            }
            'D' => {
                // Cursor backward
                let n = params.iter().next().and_then(|p| p[0]).unwrap_or(1) as u16;
                self.cursor_pos.0 = self.cursor_pos.0.saturating_sub(n);
            }
            'J' => {
                // Clear screen
                let mode = params.iter().next().and_then(|p| p[0]).unwrap_or(0);
                match mode {
                    0 => {
                        // Clear from cursor to end
                        let (x, y) = self.cursor_pos;
                        for row in y as usize..self.display_buffer.len() {
                            let start = if row == y as usize { x as usize } else { 0 };
                            for col in start..self.display_buffer[row].len() {
                                self.display_buffer[row][col] = ' ';
                            }
                        }
                    }
                    2 => {
                        // Clear entire screen
                        for row in &mut self.display_buffer {
                            for col in row {
                                *col = ' ';
                            }
                        }
                    }
                    _ => {}
                }
            }
            'K' => {
                // Clear line
                let mode = params.iter().next().and_then(|p| p[0]).unwrap_or(0);
                let y = self.cursor_pos.1 as usize;
                if y < self.display_buffer.len() {
                    match mode {
                        0 => {
                            // Clear from cursor to end of line
                            for col in self.cursor_pos.0 as usize..self.display_buffer[y].len() {
                                self.display_buffer[y][col] = ' ';
                            }
                        }
                        2 => {
                            // Clear entire line
                            for col in &mut self.display_buffer[y] {
                                *col = ' ';
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    fn esc_dispatch(&mut self, _intermediates: &[u8], _ignore: bool, _byte: u8) {
        // Not implemented for basic terminal
    }
}

fn convert_key_to_bytes(key: KeyEvent) -> Vec<u8> {
    use crossterm::event::{KeyCode, KeyModifiers};

    match (key.code, key.modifiers) {
        (KeyCode::Char(c), KeyModifiers::NONE) => c.to_string().into_bytes(),
        (KeyCode::Char(c), KeyModifiers::CONTROL) => {
            if c >= 'a' && c <= 'z' {
                vec![(c as u8) - b'a' + 1]
            } else if c >= 'A' && c <= 'Z' {
                vec![(c as u8) - b'A' + 1]
            } else {
                vec![]
            }
        }
        (KeyCode::Enter, _) => vec![b'\r'],
        (KeyCode::Backspace, _) => vec![0x7f],
        (KeyCode::Left, _) => vec![0x1b, b'[', b'D'],
        (KeyCode::Right, _) => vec![0x1b, b'[', b'C'],
        (KeyCode::Up, _) => vec![0x1b, b'[', b'A'],
        (KeyCode::Down, _) => vec![0x1b, b'[', b'B'],
        (KeyCode::Home, _) => vec![0x1b, b'[', b'H'],
        (KeyCode::End, _) => vec![0x1b, b'[', b'F'],
        (KeyCode::PageUp, _) => vec![0x1b, b'[', b'5', b'~'],
        (KeyCode::PageDown, _) => vec![0x1b, b'[', b'6', b'~'],
        (KeyCode::Tab, _) => vec![b'\t'],
        (KeyCode::Delete, _) => vec![0x1b, b'[', b'3', b'~'],
        (KeyCode::Insert, _) => vec![0x1b, b'[', b'2', b'~'],
        (KeyCode::F(n), _) => {
            match n {
                1 => vec![0x1b, b'O', b'P'],
                2 => vec![0x1b, b'O', b'Q'],
                3 => vec![0x1b, b'O', b'R'],
                4 => vec![0x1b, b'O', b'S'],
                5 => vec![0x1b, b'[', b'1', b'5', b'~'],
                6 => vec![0x1b, b'[', b'1', b'7', b'~'],
                7 => vec![0x1b, b'[', b'1', b'8', b'~'],
                8 => vec![0x1b, b'[', b'1', b'9', b'~'],
                9 => vec![0x1b, b'[', b'2', b'0', b'~'],
                10 => vec![0x1b, b'[', b'2', b'1', b'~'],
                _ => vec![],
            }
        }
        (KeyCode::Esc, _) => vec![0x1b],
        _ => vec![],
    }
}