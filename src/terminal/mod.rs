use anyhow::Result;
use crossterm::event::KeyEvent;
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use std::io::{Read, Write, ErrorKind};
use std::path::Path;

// For setting non-blocking mode on Unix
#[cfg(unix)]
use std::os::unix::io::AsRawFd;

pub struct TerminalEmulator {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    _child: Box<dyn Child + Send>,  // Keep the child process alive
    size: (u16, u16),
    parser: vt100::Parser,
    active_files: Vec<String>,
    has_pending_input: bool,  // Track if we've sent input that needs reading
    is_alive: bool,  // Track if terminal process is still running
}

use ratatui::style::Color;

// Helper function to set non-blocking mode on a file descriptor
#[cfg(unix)]
fn set_nonblocking(fd: std::os::unix::io::RawFd) -> Result<()> {
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFL, 0);
        if flags < 0 {
            return Err(anyhow::anyhow!("Failed to get file descriptor flags"));
        }

        if libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) < 0 {
            return Err(anyhow::anyhow!("Failed to set non-blocking mode"));
        }
    }

    Ok(())
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

        // Set the PTY to non-blocking mode
        #[cfg(unix)]
        if let Some(fd) = pair.master.as_raw_fd() {
            if let Err(e) = set_nonblocking(fd) {
                tracing::warn!("Failed to set PTY to non-blocking mode: {}", e);
            } else {
                tracing::debug!("PTY set to non-blocking mode");
            }
        }

        // Parse command - check if it contains spaces (arguments)
        let (program, args) = if command.contains(' ') {
            // Command has arguments, need to run through shell
            let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());
            (shell, vec!["-c".to_string(), command.to_string()])
        } else {
            // Simple command without arguments
            (command.to_string(), vec![])
        };

        let mut cmd = CommandBuilder::new(&program);
        for arg in args {
            cmd.arg(arg);
        }
        cmd.cwd(working_dir);
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");

        let child = pair.slave.spawn_command(cmd)?;

        tracing::info!("Terminal created with command: {} in dir: {:?}", command, working_dir);

        // Create vt100 parser
        let mut parser = vt100::Parser::default();
        parser.set_size(size.1, size.0);

        let writer = pair.master.take_writer()?;

        let emulator = TerminalEmulator {
            master: pair.master,
            writer,
            _child: child,  // Keep the child process alive
            size,
            parser,
            active_files: Vec::new(),
            has_pending_input: false,
            is_alive: true,
        };

        Ok(emulator)
    }

    pub fn write(&mut self, data: &[u8]) -> Result<()> {
        // Don't write to dead terminal
        if !self.is_alive {
            tracing::debug!("Refusing to write to dead terminal");
            return Ok(());
        }

        tracing::debug!("Writing {} bytes to terminal: {:?}", data.len(), String::from_utf8_lossy(data));
        self.writer.write_all(data)?;
        self.writer.flush()?;
        self.has_pending_input = true;  // Mark that we need to read output

        // In release builds, add a tiny yield to ensure PTY processes the input
        #[cfg(not(debug_assertions))]
        std::thread::yield_now();

        Ok(())
    }

    pub fn resize(&mut self, size: (u16, u16)) -> Result<()> {
        // Only resize if size actually changed
        if self.size == size {
            return Ok(());
        }

        tracing::debug!("Resizing terminal from {:?} to {:?}", self.size, size);

        self.size = size;
        let pty_size = PtySize {
            rows: size.1,
            cols: size.0,
            pixel_width: 0,
            pixel_height: 0,
        };
        self.master.resize(pty_size)?;

        // Resize vt100 parser
        self.parser.set_size(size.1, size.0);

        Ok(())
    }

    pub fn is_alive(&self) -> bool {
        self.is_alive
    }

    pub fn get_display_colors(&self) -> Vec<Vec<(Color, Color)>> {
        let mut colors = Vec::new();

        for row in 0..self.size.1 as usize {
            let mut row_colors = Vec::new();
            for col in 0..self.size.0 as usize {
                // vt100 doesn't expose cell colors in an easy way
                let fg = Color::Reset;
                let bg = Color::Reset;
                row_colors.push((fg, bg));
            }
            colors.push(row_colors);
        }

        colors
    }

    pub fn update(&mut self) -> Result<bool> {  // Returns true if there was output
        // Don't try to read from dead terminals
        if !self.is_alive {
            tracing::trace!("Skipping update for dead terminal");
            return Ok(false);
        }

        tracing::trace!("Terminal::update called, has_pending_input: {}", self.has_pending_input);

        let mut buffer = [0u8; 4096];
        let mut total_read = 0;

        tracing::debug!("Attempting to clone reader");
        let mut reader = match self.master.try_clone_reader() {
            Ok(r) => {
                tracing::debug!("Successfully cloned reader");
                r
            },
            Err(e) => {
                tracing::error!("Failed to clone reader: {}", e);
                return Err(anyhow::anyhow!("Failed to clone reader: {}", e));
            }
        };

        // If we're expecting output (just sent input), do more iterations
        let max_iterations = if self.has_pending_input {
            tracing::debug!("Has pending input, will do up to 10 read iterations");
            10  // More iterations when we expect output
        } else {
            tracing::trace!("No pending input, will do 1 read iteration");
            1   // Just check once for async output
        };

        if self.has_pending_input {
            self.has_pending_input = false;
        }
        let mut iterations = 0;

        loop {
            iterations += 1;
            if iterations > max_iterations {
                tracing::debug!("Reached max read iterations ({}), stopping", max_iterations);
                break;
            }

            tracing::trace!("Read iteration {}", iterations);

            match reader.read(&mut buffer) {
                Ok(0) => {
                    // EOF - terminal process has exited
                    if self.is_alive {
                        tracing::error!("TERMINAL DIED: Process has exited - marking as dead");
                        self.is_alive = false;

                        // Add exit message to screen
                        let exit_msg = "[Process exited - Press Ctrl+W to close]";
                        // Process the message through vt100 parser
                        self.parser.process(exit_msg.as_bytes());

                        // Return true to force a redraw showing the exit message
                        return Ok(true);
                    } else {
                        // Already dead, just return
                        tracing::trace!("Terminal already dead, skipping further reads");
                        return Ok(false);
                    }
                }
                Ok(n) => {
                    tracing::debug!("Read {} bytes on iteration {}", n, iterations);
                    total_read += n;

                    // Parse for file references first
                    self.parse_for_files(&buffer[..n]);

                    // Process through vt100 parser
                    self.parser.process(&buffer[..n]);
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock || e.kind() == ErrorKind::Interrupted => {
                    // No more data available (non-blocking read)
                    tracing::trace!("Got WouldBlock on iteration {}", iterations);
                    break;
                }
                Err(e) => {
                    tracing::warn!("PTY read error on iteration {}: {}", iterations, e);
                    break;
                }
            }
        }

        if total_read > 0 {
            tracing::debug!("Total read: {} bytes", total_read);
        }

        Ok(total_read > 0)
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
        // Don't process keys if terminal is dead
        if !self.is_alive {
            tracing::warn!("Ignoring key event for dead terminal: {:?}", key);
            return Ok(());
        }

        tracing::debug!("Handling key event: {:?}", key);
        let bytes = convert_key_to_bytes(key);
        if !bytes.is_empty() {
            tracing::debug!("Converted to {} bytes: {:?}", bytes.len(), bytes);
            self.write(&bytes)?;
        } else {
            tracing::debug!("Key event produced no bytes: {:?}", key);
        }
        Ok(())
    }

    pub fn get_visible_content(&self) -> Vec<String> {
        // Get the screen contents as a string and split by lines
        let screen_str = self.parser.screen().contents();
        let lines: Vec<String> = screen_str.lines().map(|s| s.to_string()).collect();

        // Pad or truncate to match terminal size
        let mut content = Vec::new();
        for i in 0..self.size.1 as usize {
            if i < lines.len() {
                let mut line = lines[i].clone();
                // Pad line to terminal width
                while line.len() < self.size.0 as usize {
                    line.push(' ');
                }
                content.push(line);
            } else {
                // Empty line
                content.push(" ".repeat(self.size.0 as usize));
            }
        }
        content
    }

    pub fn get_cursor_position(&self) -> (u16, u16) {
        let (row, col) = self.parser.screen().cursor_position();
        (col as u16, row as u16)
    }

    pub fn scroll(&mut self, _lines: isize) {
        // vt100 handles scrolling internally
    }

    pub fn get_active_files(&self) -> &[String] {
        &self.active_files
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