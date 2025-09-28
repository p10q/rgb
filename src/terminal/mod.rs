use alacritty_terminal::{
    event::{Event as AlacEvent, EventListener, WindowSize},
    event_loop::{EventLoop, EventLoopSender, Msg, Notifier},
    grid::{Dimensions, Scroll},
    index::{Column, Line, Point},
    sync::FairMutex,
    term::{Config, Term},
    tty::{self, Pty},
};
use anyhow::Result;
use crossterm::event::KeyEvent;
use ratatui::style::Color;
use std::{
    borrow::Cow,
    path::Path,
    sync::{Arc, Mutex},
};

struct TermSize {
    columns: usize,
    screen_lines: usize,
}

impl TermSize {
    fn new(columns: usize, screen_lines: usize) -> Self {
        Self { columns, screen_lines }
    }
}

impl Dimensions for TermSize {
    fn total_lines(&self) -> usize {
        self.screen_lines
    }

    fn screen_lines(&self) -> usize {
        self.screen_lines
    }

    fn columns(&self) -> usize {
        self.columns
    }
}

impl Dimensions for &TermSize {
    fn total_lines(&self) -> usize {
        self.screen_lines
    }

    fn screen_lines(&self) -> usize {
        self.screen_lines
    }

    fn columns(&self) -> usize {
        self.columns
    }
}

pub struct TerminalEmulator {
    term: Arc<FairMutex<Term<EventProxy>>>,
    sender: EventLoopSender,
    size: (u16, u16),
    active_files: Vec<String>,
    is_alive: Arc<Mutex<bool>>,
}

#[derive(Clone)]
struct EventProxy {
    is_alive: Arc<Mutex<bool>>,
}

impl EventListener for EventProxy {
    fn send_event(&self, event: AlacEvent) {
        // Only log non-noisy events
        match &event {
            AlacEvent::MouseCursorDirty | AlacEvent::ClipboardStore(_, _) | AlacEvent::ClipboardLoad(_, _) => {
                // Don't log these noisy events
            }
            _ => {
                tracing::debug!("EventProxy received event: {:?}", event);
            }
        }

        match event {
            AlacEvent::Exit => {
                tracing::info!("Terminal process exited!");
                *self.is_alive.lock().unwrap() = false;
            }
            AlacEvent::Title(title) => {
                tracing::info!("Terminal title changed: {}", title);
            }
            AlacEvent::ResetTitle => {
                tracing::debug!("Terminal title reset");
            }
            AlacEvent::ClipboardStore(_, _) => {
                // Silent
            }
            AlacEvent::ClipboardLoad(_, _) => {
                // Silent
            }
            AlacEvent::ColorRequest(_, _) => {
                tracing::trace!("Color request event");
            }
            AlacEvent::PtyWrite(data) => {
                let preview = if data.len() > 100 {
                    format!("{}...", &data[..100])
                } else {
                    data.clone()
                };
                tracing::info!("PTY write request: {} bytes, content: {:?}", data.len(), preview);
            }
            AlacEvent::MouseCursorDirty => {
                // Silent
            }
            AlacEvent::Bell => {
                tracing::debug!("Terminal bell!");
            }
            AlacEvent::ChildExit(_) => {
                tracing::info!("Child process exit event");
                *self.is_alive.lock().unwrap() = false;
            }
            AlacEvent::Wakeup => {
                tracing::trace!("Wakeup event");
            }
            AlacEvent::TextAreaSizeRequest(_) => {
                tracing::trace!("Text area size request");
            }
            AlacEvent::CursorBlinkingChange => {
                tracing::trace!("Cursor blinking change");
            }
        }
    }
}

impl TerminalEmulator {
    pub fn new(command: &str, working_dir: &Path, size: (u16, u16)) -> Result<Self> {
        let window_size = WindowSize {
            num_lines: size.1,
            num_cols: size.0,
            cell_width: 1,
            cell_height: 1,
        };

        // Parse command - use default shell if empty
        let (shell, args) = if command.is_empty() {
            // DEBUG: Try running a simple command that definitely produces output
            let test_simple = true;  // Set to true to test with simple command

            if test_simple {
                // Run a simple echo command for testing
                ("/bin/echo".to_string(), vec!["RGB Terminal Test Output".to_string()])
            } else {
                let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());
                // Force interactive mode for shells
                let args = if shell.ends_with("zsh") {
                    vec!["-i".to_string()]  // Interactive mode for zsh
                } else if shell.ends_with("bash") {
                    vec!["-i".to_string()]  // Interactive mode for bash
                } else {
                    vec![]
                };
                (shell, args)
            }
        } else if command.contains(' ') {
            // Has arguments, use shell to execute the command
            let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());
            (shell, vec!["-c".to_string(), command.to_string()])
        } else {
            // Single command without args
            (command.to_string(), vec![])
        };

        // Set up environment variables for proper terminal operation
        let mut env = std::collections::HashMap::new();
        env.insert("TERM".to_string(), "xterm-256color".to_string());

        // Force interactive shell behavior
        env.insert("PS1".to_string(), "$ ".to_string());  // Simple prompt
        env.insert("COLUMNS".to_string(), size.0.to_string());
        env.insert("LINES".to_string(), size.1.to_string());

        // Preserve important environment variables
        if let Ok(path) = std::env::var("PATH") {
            env.insert("PATH".to_string(), path);
        }
        if let Ok(home) = std::env::var("HOME") {
            env.insert("HOME".to_string(), home);
        }
        if let Ok(user) = std::env::var("USER") {
            env.insert("USER".to_string(), user);
        }

        let options = tty::Options {
            shell: Some(tty::Shell::new(shell.clone(), args.clone())),
            working_directory: Some(working_dir.to_path_buf()),
            hold: false,
            env,
        };

        tracing::info!("Creating PTY with shell: {} args: {:?} in dir: {:?}",
            shell, args, working_dir);

        // Debug: Log if we detected the shell type
        tracing::debug!("Shell ends with 'zsh': {}, ends with 'bash': {}",
            shell.ends_with("zsh"), shell.ends_with("bash"));

        let pty = tty::new(&options, window_size, 0)?;
        tracing::info!("PTY created successfully - child PID: {:?}", pty.child().id());

        let is_alive = Arc::new(Mutex::new(true));

        let event_proxy = EventProxy {
            is_alive: is_alive.clone(),
        };

        let config = Config::default();
        let term_size = TermSize::new(size.0 as usize, size.1 as usize);
        let term = Term::new(config, &term_size, event_proxy.clone());
        let term = Arc::new(FairMutex::new(term));
        tracing::debug!("Terminal emulator created");

        let event_loop = EventLoop::new(
            Arc::clone(&term),
            event_proxy,
            pty,
            false,  // hold
            false,  // ref_test
        )?;
        tracing::debug!("Event loop created");

        let sender = event_loop.channel();

        // Spawn event loop - let it manage its own lifecycle
        let _io_thread = event_loop.spawn();
        tracing::info!("Event loop spawned - terminal should be running now");

        // Give the event loop a moment to start
        std::thread::sleep(std::time::Duration::from_millis(100));

        // Send multiple attempts to trigger shell prompt
        tracing::info!("Sending initial commands to trigger shell prompt");

        // Try different approaches to get the shell to respond
        let _ = sender.send(Msg::Input(Cow::Borrowed(b"\r")));
        std::thread::sleep(std::time::Duration::from_millis(50));

        // Send a simple echo command
        let _ = sender.send(Msg::Input(Cow::Borrowed(b"echo test\r")));
        std::thread::sleep(std::time::Duration::from_millis(50));

        // Try sending a space and backspace to trigger redraw
        let _ = sender.send(Msg::Input(Cow::Borrowed(b" \x08")));

        tracing::info!("Terminal fully initialized - shell should be running");

        Ok(Self {
            term,
            sender,
            size,
            active_files: Vec::new(),
            is_alive,
        })
    }

    pub fn write(&mut self, data: &[u8]) -> Result<()> {
        if !self.is_alive() {
            tracing::debug!("Refusing to write to dead terminal");
            return Ok(());
        }

        tracing::info!("Writing {} bytes to terminal: {:?}",
            data.len(),
            String::from_utf8_lossy(&data[..data.len().min(50)]));
        let _ = self.sender.send(Msg::Input(Cow::Owned(data.to_vec())));
        Ok(())
    }

    pub fn resize(&mut self, size: (u16, u16)) -> Result<()> {
        if self.size == size {
            return Ok(());
        }

        tracing::debug!("Resizing terminal from {:?} to {:?}", self.size, size);
        self.size = size;

        let window_size = WindowSize {
            num_lines: size.1,
            num_cols: size.0,
            cell_width: 1,
            cell_height: 1,
        };

        let term_size = TermSize::new(size.0 as usize, size.1 as usize);
        self.term.lock().resize(&term_size);
        let _ = self.sender.send(Msg::Resize(window_size));

        Ok(())
    }

    pub fn is_alive(&self) -> bool {
        *self.is_alive.lock().unwrap()
    }

    pub fn update(&mut self) -> Result<bool> {
        if !self.is_alive() {
            tracing::trace!("Skipping update for dead terminal");

            // Check if we need to add exit message
            let mut term = self.term.lock();
            let last_line = Line(self.size.1 as i32 - 1);
            let msg = "[Process exited - Press Ctrl+W to close]";
            for (i, ch) in msg.chars().enumerate() {
                if i < self.size.0 as usize {
                    let point = Point::new(last_line, Column(i));
                    term.grid_mut()[point].c = ch;
                }
            }

            return Ok(false);
        }

        // The event loop handles reading from PTY automatically
        // Check if we have any content
        let term = self.term.lock();
        let grid = term.grid();
        let mut has_content = false;

        // Quick check for any non-space content in the first few lines
        for line_idx in 0..self.size.1.min(5) {
            for col in 0..self.size.0.min(80) {
                // Account for display offset
                let grid_line = Line(line_idx as i32) - grid.display_offset() as i32;
                let point = Point::new(grid_line, Column(col as usize));
                let cell = &grid[point];

                if cell.c != ' ' && cell.c != '\0' {
                    has_content = true;
                    if line_idx < 2 {
                        tracing::trace!("Found content at line {}, col {}: '{}'", line_idx, col, cell.c);
                    }
                    break;
                }
            }
            if has_content {
                break;
            }
        }

        if !has_content {
            tracing::debug!("Terminal update: No content visible yet (display_offset: {})",
                grid.display_offset());
        } else {
            tracing::trace!("Terminal update: Content is present");
        }

        drop(term);

        // Force a UI update to show any new terminal content
        Ok(true)  // Return true to trigger redraw
    }

    pub fn handle_key_event(&mut self, key: KeyEvent) -> Result<()> {
        if !self.is_alive() {
            tracing::warn!("Ignoring key event for dead terminal: {:?}", key);
            return Ok(());
        }

        tracing::info!("Handling key event: {:?}", key);
        let bytes = convert_key_to_bytes(key);
        tracing::trace!("Converted key to {} bytes: {:?}", bytes.len(), bytes);
        if !bytes.is_empty() {
            self.write(&bytes)?;
        } else {
            tracing::warn!("Key event produced no bytes: {:?}", key);
        }
        Ok(())
    }

    pub fn get_visible_content(&self) -> Vec<String> {
        let term = self.term.lock();
        let mut content = Vec::new();

        let grid = term.grid();

        // Get the display offset to handle scrollback
        let display_offset = grid.display_offset();

        // Log grid info for debugging
        tracing::trace!("Grid info - display_offset: {}, screen_lines: {}",
            display_offset, grid.screen_lines());

        for line_idx in 0..self.size.1 {
            let mut line_str = String::new();

            // Calculate the actual line in the grid, accounting for display offset
            let grid_line = Line(line_idx as i32) - display_offset as i32;

            for col in 0..self.size.0 {
                let point = Point::new(grid_line, Column(col as usize));
                let cell = &grid[point];
                line_str.push(cell.c);
            }
            content.push(line_str);
        }

        // Debug: Check if we have any non-empty content
        let non_empty = content.iter().any(|line| !line.trim().is_empty());
        if !non_empty {
            tracing::debug!("Warning: No visible content in terminal grid");

            // Try to check raw grid content
            let cursor = grid.cursor.point;
            tracing::debug!("Cursor position: line={}, col={}", cursor.line.0, cursor.column.0);
        }

        content
    }

    pub fn get_display_colors(&self) -> Vec<Vec<(Color, Color)>> {
        let term = self.term.lock();
        let mut colors = Vec::new();

        let grid = term.grid();

        // Get the display offset to handle scrollback - same as get_visible_content
        let display_offset = grid.display_offset();

        for line_idx in 0..self.size.1 {
            let mut row_colors = Vec::new();

            // Calculate the actual line in the grid, accounting for display offset
            let grid_line = Line(line_idx as i32) - display_offset as i32;

            for col in 0..self.size.0 {
                let point = Point::new(grid_line, Column(col as usize));
                let cell = &grid[point];

                let fg = convert_alacritty_color(cell.fg);
                let bg = convert_alacritty_color(cell.bg);
                row_colors.push((fg, bg));
            }
            colors.push(row_colors);
        }

        colors
    }

    pub fn get_cursor_position(&self) -> (u16, u16) {
        let term = self.term.lock();
        let cursor = term.grid().cursor.point;
        (cursor.column.0 as u16, cursor.line.0 as u16)
    }

    pub fn scroll(&mut self, lines: isize) {
        let mut term = self.term.lock();
        let scroll = Scroll::Delta(lines as i32);
        term.scroll_display(scroll);
    }

    pub fn get_active_files(&self) -> &[String] {
        &self.active_files
    }

    pub fn shutdown(&mut self) {
        // Mark as not alive first
        *self.is_alive.lock().unwrap() = false;

        // Signal the event loop to shutdown
        // The event loop will handle its own cleanup
        let _ = self.sender.send(Msg::Shutdown);
    }
}

impl Drop for TerminalEmulator {
    fn drop(&mut self) {
        // Only send shutdown if still alive to avoid duplicate shutdowns
        if *self.is_alive.lock().unwrap() {
            *self.is_alive.lock().unwrap() = false;
            let _ = self.sender.send(Msg::Shutdown);
        }
    }
}

fn convert_alacritty_color(color: alacritty_terminal::vte::ansi::Color) -> Color {
    use alacritty_terminal::vte::ansi::{Color as AlacColor, NamedColor};

    match color {
        AlacColor::Named(named) => match named {
            NamedColor::Black => Color::Black,
            NamedColor::Red => Color::Red,
            NamedColor::Green => Color::Green,
            NamedColor::Yellow => Color::Yellow,
            NamedColor::Blue => Color::Blue,
            NamedColor::Magenta => Color::Magenta,
            NamedColor::Cyan => Color::Cyan,
            NamedColor::White | NamedColor::Foreground => Color::White,
            NamedColor::BrightBlack => Color::DarkGray,
            NamedColor::BrightRed => Color::LightRed,
            NamedColor::BrightGreen => Color::LightGreen,
            NamedColor::BrightYellow => Color::LightYellow,
            NamedColor::BrightBlue => Color::LightBlue,
            NamedColor::BrightMagenta => Color::LightMagenta,
            NamedColor::BrightCyan => Color::LightCyan,
            NamedColor::BrightWhite | NamedColor::BrightForeground => Color::White,
            _ => Color::Reset,
        },
        AlacColor::Spec(rgb) => {
            Color::Rgb(rgb.r, rgb.g, rgb.b)
        },
        AlacColor::Indexed(idx) => {
            match idx {
                0 => Color::Black,
                1 => Color::Red,
                2 => Color::Green,
                3 => Color::Yellow,
                4 => Color::Blue,
                5 => Color::Magenta,
                6 => Color::Cyan,
                7 => Color::Gray,
                8 => Color::DarkGray,
                9 => Color::LightRed,
                10 => Color::LightGreen,
                11 => Color::LightYellow,
                12 => Color::LightBlue,
                13 => Color::LightMagenta,
                14 => Color::LightCyan,
                15 => Color::White,
                16..=231 => {
                    // 216 color cube
                    let idx = idx - 16;
                    let r = ((idx / 36) * 51) as u8;
                    let g = (((idx % 36) / 6) * 51) as u8;
                    let b = ((idx % 6) * 51) as u8;
                    Color::Rgb(r, g, b)
                },
                232..=255 => {
                    // Grayscale
                    let gray = ((idx - 232) * 10 + 8) as u8;
                    Color::Rgb(gray, gray, gray)
                },
                _ => Color::Reset,
            }
        },
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
            } else if c == ' ' {
                vec![0]  // Ctrl+Space
            } else if c == '\\' {
                vec![28]  // Ctrl+\
            } else if c == ']' {
                vec![29]  // Ctrl+]
            } else if c == '^' {
                vec![30]  // Ctrl+^
            } else if c == '_' {
                vec![31]  // Ctrl+_
            } else {
                vec![]
            }
        }
        (KeyCode::Char(c), KeyModifiers::ALT) => {
            let mut bytes = vec![0x1b];  // ESC prefix for Alt
            bytes.extend(c.to_string().into_bytes());
            bytes
        }
        (KeyCode::Enter, _) => vec![b'\r'],
        (KeyCode::Backspace, _) => vec![0x7f],
        (KeyCode::Left, KeyModifiers::NONE) => vec![0x1b, b'[', b'D'],
        (KeyCode::Right, KeyModifiers::NONE) => vec![0x1b, b'[', b'C'],
        (KeyCode::Up, KeyModifiers::NONE) => vec![0x1b, b'[', b'A'],
        (KeyCode::Down, KeyModifiers::NONE) => vec![0x1b, b'[', b'B'],
        (KeyCode::Left, KeyModifiers::ALT) => vec![0x1b, 0x1b, b'[', b'D'],
        (KeyCode::Right, KeyModifiers::ALT) => vec![0x1b, 0x1b, b'[', b'C'],
        (KeyCode::Up, KeyModifiers::ALT) => vec![0x1b, 0x1b, b'[', b'A'],
        (KeyCode::Down, KeyModifiers::ALT) => vec![0x1b, 0x1b, b'[', b'B'],
        (KeyCode::Home, _) => vec![0x1b, b'[', b'H'],
        (KeyCode::End, _) => vec![0x1b, b'[', b'F'],
        (KeyCode::PageUp, _) => vec![0x1b, b'[', b'5', b'~'],
        (KeyCode::PageDown, _) => vec![0x1b, b'[', b'6', b'~'],
        (KeyCode::Tab, KeyModifiers::NONE) => vec![b'\t'],
        (KeyCode::Tab, KeyModifiers::SHIFT) => vec![0x1b, b'[', b'Z'],  // Backtab
        (KeyCode::Delete, _) => vec![0x1b, b'[', b'3', b'~'],
        (KeyCode::Insert, _) => vec![0x1b, b'[', b'2', b'~'],
        (KeyCode::F(n), _) => match n {
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
            11 => vec![0x1b, b'[', b'2', b'3', b'~'],
            12 => vec![0x1b, b'[', b'2', b'4', b'~'],
            _ => vec![],
        },
        (KeyCode::Esc, _) => vec![0x1b],
        _ => vec![],
    }
}