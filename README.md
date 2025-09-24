# RGB (Rust Good Vibes)

A terminal multiplexer and workspace manager - Rust port of AIWorkbench.

## Project Structure

```
rgb/
├── DESIGN.md           # Comprehensive design document
├── Cargo.toml          # Dependencies and project configuration
├── src/
│   ├── main.rs         # Application entry point
│   ├── app/            # Core application logic
│   ├── config/         # Configuration management
│   ├── git/            # Git worktree integration
│   ├── layout/         # Terminal layout engine (tiling, floating, etc.)
│   ├── monitor/        # File system monitoring
│   ├── terminal/       # Terminal emulation
│   ├── ui/             # Ratatui-based UI components
│   └── workspace/      # Workspace and terminal session management
```

## Current Status

✅ **Completed:**
- Comprehensive design document (DESIGN.md)
- Project structure and module organization
- Core dependency configuration
- Basic architecture implementation:
  - Application lifecycle management
  - Configuration system
  - Workspace management framework
  - Layout engine with multiple tiling modes
  - Git integration structure
  - File monitoring system
  - UI framework with Ratatui

🚧 **In Progress:**
- Terminal emulation integration (simplifying from alacritty_terminal to portable-pty)
- Fixing compilation errors and API compatibility

📋 **Next Steps:**
1. Replace alacritty_terminal with simpler PTY handling
2. Implement basic terminal rendering
3. Add git worktree creation/management
4. Complete file tracking system
5. Implement drag-and-drop terminal arrangement

## Key Features (Planned)

- **Multiple Terminal Sessions**: Up to 10 concurrent terminals
- **Git Worktrees**: Each terminal gets its own isolated git worktree
- **Smart Layouts**: Grid, spiral, vertical, horizontal, floating, tabbed
- **File Conflict Detection**: Warns when multiple terminals edit same files
- **Keyboard Driven**: Vim-like keybindings for navigation
- **Real-time Git Status**: See changes as they happen
- **Customizable**: TOML-based configuration

## Building

```bash
# Check compilation (currently has errors to fix)
cargo check

# Build (once compilation errors are resolved)
cargo build --release

# Run
./target/release/rgb [project-directory]
```

## Configuration

Configuration file at `~/.config/rgb/config.toml`:

```toml
[general]
max_terminals = 10
default_shell = "/bin/zsh"

[layout]
default = "grid"

[git]
auto_worktree = true
```

## Architecture Highlights

- **Single Process**: Everything runs in one terminal app with sub-terminals
- **Async/Await**: Built on Tokio for efficient concurrency
- **Immediate Mode UI**: Ratatui for responsive terminal UI
- **libgit2**: Native git operations without shelling out

See [DESIGN.md](DESIGN.md) for complete architectural details.

## Dependencies

- `ratatui` - Terminal UI framework
- `crossterm` - Cross-platform terminal manipulation
- `portable-pty` - PTY handling
- `tokio` - Async runtime
- `git2` - Git operations
- `notify` - File system monitoring

## License

MIT