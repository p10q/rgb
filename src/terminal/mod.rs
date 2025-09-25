use anyhow::Result;
use crossterm::event::KeyEvent;
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use std::io::{Read, Write, ErrorKind};
use std::path::Path;
use vte::{Params, Parser, Perform};

// For setting non-blocking mode on Unix
#[cfg(unix)]
use std::os::unix::io::AsRawFd;

pub struct TerminalEmulator {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    _child: Box<dyn Child + Send>,  // Keep the child process alive
    size: (u16, u16),
    output_buffer: Vec<u8>,
    display_buffer: Vec<Vec<char>>,
    cursor_pos: (u16, u16),
    active_files: Vec<String>,
    has_pending_input: bool,  // Track if we've sent input that needs reading
}

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
                tracing::info!("PTY set to non-blocking mode");
            }
        }

        let mut cmd = CommandBuilder::new(command);
        cmd.cwd(working_dir);
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");

        // For shells, don't add -i flag here, let the shell decide
        // The shell will detect it's connected to a TTY and become interactive

        let child = pair.slave.spawn_command(cmd)?;

        tracing::info!("Terminal created with command: {} in dir: {:?}", command, working_dir);

        // Initialize display buffer
        let mut display_buffer = Vec::with_capacity(size.1 as usize);
        for _ in 0..size.1 {
            display_buffer.push(vec![' '; size.0 as usize]);
        }

        let writer = pair.master.take_writer()?;

        let emulator = TerminalEmulator {
            master: pair.master,
            writer,
            _child: child,  // Keep the child process alive
            size,
            output_buffer: Vec::new(),
            display_buffer,
            cursor_pos: (0, 0),
            active_files: Vec::new(),
            has_pending_input: false,
        };

        Ok(emulator)
    }

    pub fn write(&mut self, data: &[u8]) -> Result<()> {
        tracing::debug!("Writing {} bytes to terminal: {:?}", data.len(), String::from_utf8_lossy(data));
        self.writer.write_all(data)?;
        self.writer.flush()?;
        self.has_pending_input = true;  // Mark that we need to read output
        // Don't call update() here to avoid potential recursion
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

        // Resize display buffer, preserving existing content where possible
        let old_buffer = std::mem::take(&mut self.display_buffer);
        self.display_buffer = Vec::with_capacity(size.1 as usize);

        for y in 0..size.1 as usize {
            let mut new_line = vec![' '; size.0 as usize];

            // Copy over existing content if available
            if y < old_buffer.len() {
                let old_line = &old_buffer[y];
                for x in 0..std::cmp::min(size.0 as usize, old_line.len()) {
                    new_line[x] = old_line[x];
                }
            }

            self.display_buffer.push(new_line);
        }

        // Adjust cursor position if needed
        self.cursor_pos.0 = self.cursor_pos.0.min(size.0 - 1);
        self.cursor_pos.1 = self.cursor_pos.1.min(size.1 - 1);

        Ok(())
    }

    pub fn update(&mut self) -> Result<()> {
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
        // Otherwise just do 1 iteration to check for any async output
        let max_iterations = if self.has_pending_input {
            tracing::debug!("Has pending input, will do up to 10 read iterations");
            10  // More iterations when we expect output
        } else if self.output_buffer.is_empty() {
            tracing::debug!("Empty buffer, will do up to 10 read iterations");
            10  // First time, read more
        } else {
            tracing::trace!("No pending input, will do 1 read iteration");
            1   // Just check once for async output
        };

        if self.has_pending_input {
            self.has_pending_input = false;
        }
        let mut iterations = 0;

        // Read all available data (with a limit to prevent infinite loops)
        tracing::debug!("Starting read loop with max {} iterations", max_iterations);

        // First, do a single read to see if there's any data
        let mut has_data = false;

        loop {
            iterations += 1;
            if iterations > max_iterations {
                tracing::debug!("Reached max read iterations ({}), stopping", max_iterations);
                break;
            }

            tracing::trace!("Read iteration {}", iterations);

            // After 6 successful reads in any update, stop
            if iterations > 6 && has_data {
                tracing::debug!("Stopping after {} reads to avoid blocking", iterations - 1);
                break;
            }

            match reader.read(&mut buffer) {
                Ok(0) => {
                    // EOF
                    tracing::debug!("Read returned 0 bytes (EOF)");
                    if total_read == 0 && iterations == 1 && self.has_pending_input {
                        // Give it one more try with a very small delay
                        std::thread::sleep(std::time::Duration::from_millis(5));
                        continue;
                    }
                    if total_read == 0 {
                        tracing::trace!("No data from PTY (EOF)");
                    }
                    break;
                }
                Ok(n) => {
                    tracing::debug!("Read {} bytes on iteration {}", n, iterations);
                    has_data = true;
                    total_read += n;
                    self.output_buffer.extend_from_slice(&buffer[..n]);

                    // Only log in trace mode to avoid spam
                    if tracing::enabled!(tracing::Level::TRACE) {
                        let text = String::from_utf8_lossy(&buffer[..n]);
                        tracing::trace!("Read {} bytes: {:?}", n, text);
                    }

                    // Log first few bytes for debugging
                    let preview = &buffer[..n.min(50)];
                    let preview_str = String::from_utf8_lossy(preview);
                    tracing::debug!("Read data preview ({}b): {:?}", n, preview_str);

                    // Parse for file references first
                    self.parse_for_files(&buffer[..n]);

                    // Process through VTE parser
                    let mut parser = Parser::new();
                    for byte in &buffer[..n] {
                        parser.advance(self, *byte);
                    }
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

        // Only log buffer state once at the end if we read something
        if total_read > 0 && tracing::enabled!(tracing::Level::DEBUG) {
            let non_empty_lines: Vec<_> = self.display_buffer.iter()
                .enumerate()
                .filter(|(_, line)| line.iter().any(|&c| c != ' '))
                .take(3)
                .map(|(i, line)| (i + 1, line.iter().collect::<String>()))
                .collect();

            if !non_empty_lines.is_empty() {
                tracing::debug!("Terminal has {} lines of content", non_empty_lines.len());
            }
        }

        if total_read > 0 {
            tracing::debug!("Total read: {} bytes, cursor at {:?}", total_read, self.cursor_pos);
        }

        tracing::debug!("Terminal update complete, read {} total bytes", total_read);
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
        let content: Vec<String> = self.display_buffer
            .iter()
            .map(|line| line.iter().collect::<String>())
            .collect();

        // Debug log non-empty lines
        let non_empty_count = content.iter().filter(|line| !line.trim().is_empty()).count();
        if non_empty_count > 0 {
            tracing::trace!("Display buffer has {} non-empty lines out of {}", non_empty_count, content.len());

            // Log line 4 specifically if it exists
            if content.len() > 4 {
                let line4 = &content[4];
                if !line4.trim().is_empty() {
                    tracing::debug!("Line 4 content (len {}): {:?}", line4.len(), line4);
                }
            }
        }

        content
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
        tracing::debug!("VTE print '{}' at cursor ({}, {}), buffer size ({} x {})",
            c, x, y, self.size.0, self.size.1);

        if (y as usize) < self.display_buffer.len() && (x as usize) < self.display_buffer[y as usize].len() {
            self.display_buffer[y as usize][x as usize] = c;
            self.cursor_pos.0 = x + 1;

            // Handle line wrap
            if self.cursor_pos.0 >= self.size.0 {
                self.cursor_pos.0 = 0;
                self.cursor_pos.1 = (self.cursor_pos.1 + 1).min(self.size.1 - 1);
            }

            tracing::debug!("Updated buffer at ({}, {}), new cursor at ({}, {})",
                x, y, self.cursor_pos.0, self.cursor_pos.1);
        } else {
            tracing::warn!("Print out of bounds: cursor ({}, {}) for buffer size ({} x {})",
                x, y, self.display_buffer[0].len(), self.display_buffer.len());
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
                let row = *params.iter().next().and_then(|p| p.first()).unwrap_or(&1) as u16;
                let col = *params.iter().nth(1).and_then(|p| p.first()).unwrap_or(&1) as u16;
                let new_pos = (col.saturating_sub(1), row.saturating_sub(1));
                tracing::debug!("CSI cursor position: row={}, col={}, setting cursor to {:?}",
                    row, col, new_pos);
                self.cursor_pos = new_pos;
            }
            'A' => {
                // Cursor up
                let n = *params.iter().next().and_then(|p| p.first()).unwrap_or(&1) as u16;
                self.cursor_pos.1 = self.cursor_pos.1.saturating_sub(n);
            }
            'B' => {
                // Cursor down
                let n = *params.iter().next().and_then(|p| p.first()).unwrap_or(&1) as u16;
                self.cursor_pos.1 = (self.cursor_pos.1 + n).min(self.size.1 - 1);
            }
            'C' => {
                // Cursor forward
                let n = *params.iter().next().and_then(|p| p.first()).unwrap_or(&1) as u16;
                self.cursor_pos.0 = (self.cursor_pos.0 + n).min(self.size.0 - 1);
            }
            'D' => {
                // Cursor backward
                let n = *params.iter().next().and_then(|p| p.first()).unwrap_or(&1) as u16;
                self.cursor_pos.0 = self.cursor_pos.0.saturating_sub(n);
            }
            'J' => {
                // Clear screen
                let mode = *params.iter().next().and_then(|p| p.first()).unwrap_or(&0);
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
                    1 => {
                        // Clear from beginning to cursor
                        let (x, y) = self.cursor_pos;
                        for row in 0..=y as usize {
                            let end = if row == y as usize { x as usize + 1 } else { self.display_buffer[row].len() };
                            for col in 0..end {
                                if row < self.display_buffer.len() && col < self.display_buffer[row].len() {
                                    self.display_buffer[row][col] = ' ';
                                }
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
                        // Reset cursor to top-left
                        self.cursor_pos = (0, 0);
                    }
                    _ => {}
                }
                tracing::trace!("Clear screen mode {}", mode);
            }
            'K' => {
                // Clear line
                let mode = *params.iter().next().and_then(|p| p.first()).unwrap_or(&0);
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