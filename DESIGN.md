# RGB (Rust Good Vibes) - Design Document
## AI Workbench Rust Port

**Author:** AI Assistant
**Date:** September 2025
**Version:** 1.0.0

## Executive Summary

RGB is a Rust-based terminal multiplexer and workspace manager that reimplements AIWorkbench's core functionality as a single terminal application. It provides multiple sub-terminals, git worktree management, intelligent file tracking, and automatic tiling/resizing within a unified TUI environment.

## Core Design Principles

1. **Single Process Architecture**: Everything runs in one terminal app with sub-terminals
2. **Performance First**: Leverage Rust's zero-cost abstractions and async/await
3. **Keyboard Driven**: Full keyboard navigation with vim-like bindings
4. **Git Native**: Deep integration with git worktrees and libgit2
5. **Extensible**: Plugin architecture for custom terminal backends

## Technology Stack

### Core Libraries

#### TUI Framework
- **[ratatui](https://github.com/ratatui/ratatui)** (v0.29+)
  - Modern TUI framework with extensive widget support
  - Immediate mode rendering with double buffering
  - Cross-platform terminal support
  - Active community and ecosystem

#### Terminal Emulation
- **[alacritty_terminal](https://crates.io/crates/alacritty_terminal)** (v0.24+)
  - Production-ready terminal emulator core
  - VT100/xterm compatibility
  - GPU-accelerated rendering support
  - PTY handling and process management

#### Async Runtime
- **[tokio](https://tokio.rs)** (v1.40+)
  - Industry-standard async runtime
  - Multi-threaded work stealing scheduler
  - Excellent ecosystem integration

#### Git Integration
- **[git2](https://github.com/rust-lang/git2-rs)** (v0.19+)
  - libgit2 bindings for Rust
  - Worktree management
  - Diff generation and patch application
  - Status monitoring

#### File System Monitoring
- **[notify](https://github.com/notify-rs/notify)** (v6.1+)
  - Cross-platform file system event monitoring
  - Debounced events
  - Recursive directory watching

#### Configuration
- **[config](https://github.com/mehcode/config-rs)** (v0.14+)
  - Layered configuration (defaults → system → user)
  - Multiple format support (TOML, YAML, JSON)
  - Environment variable overrides

#### Additional Dependencies
- **[crossterm](https://github.com/crossterm-rs/crossterm)** - Terminal manipulation
- **[clap](https://github.com/clap-rs/clap)** - CLI argument parsing
- **[serde](https://serde.rs)** - Serialization/deserialization
- **[anyhow](https://github.com/dtolnay/anyhow)** - Error handling
- **[tracing](https://github.com/tokio-rs/tracing)** - Structured logging
- **[directories](https://github.com/dirs-dev/directories-rs)** - Platform-specific paths

## Architecture Overview

```
┌─────────────────────────────────────────────────────────────┐
│                         RGB Application                      │
├─────────────────────────────────────────────────────────────┤
│                          UI Layer                            │
│  ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌──────────┐       │
│  │  Layout  │ │  Input   │ │  Render  │ │  Widgets │       │
│  │  Engine  │ │  Handler │ │  Engine  │ │  Library │       │
│  └──────────┘ └──────────┘ └──────────┘ └──────────┘       │
├─────────────────────────────────────────────────────────────┤
│                      Core Services                           │
│  ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌──────────┐       │
│  │Workspace │ │ Terminal │ │   Git    │ │   File   │       │
│  │ Manager  │ │ Emulator │ │ Manager  │ │  Monitor │       │
│  └──────────┘ └──────────┘ └──────────┘ └──────────┘       │
├─────────────────────────────────────────────────────────────┤
│                     System Integration                       │
│  ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌──────────┐       │
│  │   PTY    │ │  Process │ │  libgit2 │ │  notify  │       │
│  │  Handler │ │  Manager │ │  Bindings│ │   Events │       │
│  └──────────┘ └──────────┘ └──────────┘ └──────────┘       │
└─────────────────────────────────────────────────────────────┘
```

## Component Design

### 1. Application Core (`src/app.rs`)

```rust
pub struct RgbApp {
    workspace: WorkspaceManager,
    layout: LayoutEngine,
    input: InputHandler,
    renderer: Renderer,
    config: AppConfig,
    state: AppState,
}

pub enum AppState {
    Normal,
    Insert,
    Command,
    Visual,
}
```

### 2. Workspace Manager (`src/workspace/mod.rs`)

Manages the overall workspace including terminals, worktrees, and project state.

```rust
pub struct WorkspaceManager {
    terminals: Vec<TerminalSession>,
    active_terminal: Option<TerminalId>,
    project_dir: PathBuf,
    worktrees: HashMap<TerminalId, WorktreeInfo>,
    file_tracker: FileTracker,
    git_manager: GitManager,
}

pub struct TerminalSession {
    id: TerminalId,
    title: String,
    emulator: TerminalEmulator,
    pty: PtyHandle,
    process: Child,
    working_dir: PathBuf,
    active_files: HashSet<PathBuf>,
    layout_position: LayoutPosition,
}
```

### 3. Layout Engine (`src/layout/mod.rs`)

Handles automatic tiling, manual resizing, and drag-and-drop arrangement.

```rust
pub enum LayoutMode {
    Tiled(TileLayout),
    Floating,
    Tabbed,
    Stacked,
}

pub enum TileLayout {
    Vertical,
    Horizontal,
    Grid { cols: usize },
    Spiral,
}

pub struct LayoutEngine {
    mode: LayoutMode,
    containers: Vec<Container>,
    constraints: Vec<Constraint>,
    focus_stack: Vec<ContainerId>,
}

pub struct Container {
    id: ContainerId,
    content: ContainerContent,
    rect: Rect,
    resizable: bool,
    min_size: Size,
}
```

### 4. Terminal Emulator (`src/terminal/mod.rs`)

Wraps alacritty_terminal for VT100/xterm emulation.

```rust
pub struct TerminalEmulator {
    term: Term<EventProxy>,
    pty: PtyHandle,
    size: TermSize,
    scrollback: ScrollbackBuffer,
    selection: Option<Selection>,
}

impl TerminalEmulator {
    pub fn write(&mut self, data: &[u8]) -> Result<()>
    pub fn resize(&mut self, size: TermSize) -> Result<()>
    pub fn scroll(&mut self, lines: i32) -> Result<()>
    pub fn select(&mut self, start: Point, end: Point)
    pub fn copy_selection(&self) -> Option<String>
}
```

### 5. Git Integration (`src/git/mod.rs`)

Manages worktrees, monitors changes, and handles commits.

```rust
pub struct GitManager {
    repo: Repository,
    worktrees: HashMap<TerminalId, Worktree>,
    status_cache: Arc<RwLock<GitStatus>>,
    monitor: GitMonitor,
}

pub struct Worktree {
    path: PathBuf,
    branch: String,
    terminal_id: TerminalId,
    last_sync: Instant,
    merge_status: MergeStatus,
}

impl GitManager {
    pub async fn create_worktree(&mut self, terminal_id: TerminalId) -> Result<PathBuf>
    pub async fn sync_worktree(&mut self, terminal_id: TerminalId) -> Result<()>
    pub async fn commit(&mut self, message: &str, files: Vec<PathBuf>) -> Result<()>
    pub async fn get_diff(&self, terminal_id: TerminalId) -> Result<Vec<DiffHunk>>
}
```

### 6. File Monitoring (`src/monitor/mod.rs`)

Tracks file changes and detects conflicts between terminals.

```rust
pub struct FileMonitor {
    watcher: RecommendedWatcher,
    events: mpsc::Receiver<FileEvent>,
    tracked_files: Arc<RwLock<HashMap<PathBuf, Vec<TerminalId>>>>,
}

pub struct FileTracker {
    terminal_files: HashMap<TerminalId, HashSet<PathBuf>>,
    file_changes: Vec<FileChange>,
    conflict_detector: ConflictDetector,
}

pub struct ConflictDetector {
    overlaps: HashMap<PathBuf, Vec<TerminalId>>,
    resolution_strategy: ConflictResolution,
}
```

### 7. UI Components (`src/ui/mod.rs`)

Ratatui-based widgets and components.

```rust
pub struct TerminalWidget {
    terminal: Arc<Mutex<TerminalEmulator>>,
    viewport: Viewport,
    cursor: CursorStyle,
}

pub struct FileExplorerWidget {
    root: PathBuf,
    tree: FileTree,
    selected: Option<PathBuf>,
}

pub struct GitStatusWidget {
    changes: Vec<FileChange>,
    staged: HashSet<PathBuf>,
}

pub struct MinimapWidget {
    active_files: HashSet<PathBuf>,
    terminal_colors: HashMap<TerminalId, Color>,
}
```

## Key Features Implementation

### 1. Terminal Multiplexing

- **Split Creation**: Vertical/horizontal splits with configurable ratios
- **Tab Management**: Multiple workspaces with quick switching
- **Focus Navigation**: Vim-like navigation (hjkl) between panes
- **Resize Operations**: Mouse drag or keyboard commands
- **Zoom Mode**: Temporarily maximize a single terminal

### 2. Git Worktree Integration

- **Automatic Creation**: Each terminal gets its own worktree
- **Branch Management**: Automatic branch naming and tracking
- **Sync Operations**: Periodic sync with main branch
- **Conflict Resolution**: Visual merge conflict handling
- **Cleanup**: Automatic worktree removal on terminal close

### 3. File Tracking

- **Output Parsing**: Detect file references in terminal output
- **Change Detection**: Real-time file modification tracking
- **Overlap Warning**: Alert when multiple terminals edit same files
- **Visual Indicators**: Color-coded file ownership

### 4. Layout Management

- **Tiling Modes**: Automatic layouts (grid, spiral, binary)
- **Drag & Drop**: Mouse-based pane rearrangement
- **Persistent Layouts**: Save/load layout configurations
- **Responsive Sizing**: Maintain minimum sizes and proportions

## User Interface Design

### Main Layout

```
┌─────────────────────────────────────────────────────────────┐
│ [Project: my-app] [Branch: main] [Terminals: 4] [Git: ✓]   │
├────────────┬────────────────────────────────┬───────────────┤
│            │                                 │               │
│   Files    │      Terminal Grid             │   Git/Info    │
│            │  ┌──────────┬──────────┐       │               │
│ ▼ src/     │  │Terminal 1│Terminal 2│       │  Changes:     │
│   ▶ lib.rs │  │          │          │       │  M main.rs    │
│   ▶ main.rs│  ├──────────┼──────────┤       │  A test.rs    │
│ ▶ tests/   │  │Terminal 3│Terminal 4│       │               │
│            │  │          │          │       │  Timeline:    │
│            │  └──────────┴──────────┘       │  10:45 commit │
│            │                                 │  10:32 edit   │
├────────────┴────────────────────────────────┴───────────────┤
│ [:q Quit] [Tab Switch] [Ctrl+T New] [F1 Help] [Mode: NORMAL]│
└─────────────────────────────────────────────────────────────┘
```

### Key Bindings

#### Normal Mode
- `h/j/k/l` - Navigate between terminals
- `Tab` - Cycle through terminals
- `Ctrl+T` - New terminal
- `Ctrl+W` - Window commands (split, close, etc.)
- `Ctrl+G` - Git operations
- `F1-F10` - Quick terminal switch
- `:` - Command mode

#### Window Commands (Ctrl+W prefix)
- `v` - Vertical split
- `s` - Horizontal split
- `q` - Close current terminal
- `o` - Close all other terminals
- `=` - Equal sizing
- `</>` - Resize horizontally
- `+/-` - Resize vertically
- `H/J/K/L` - Move terminal to edge

#### Command Mode
- `:new <cmd>` - New terminal with command
- `:worktree` - Show worktree info
- `:commit` - Commit changes
- `:layout <name>` - Apply layout
- `:config` - Open configuration
- `:quit` - Exit application

## Configuration

### Config File (`~/.config/rgb/config.toml`)

```toml
[general]
project_dir = "~/projects"
max_terminals = 10
auto_save_layout = true
default_shell = "/bin/zsh"

[appearance]
theme = "dark"
font_size = 12
cursor_style = "block"
scrollback_lines = 10000

[keybindings]
new_terminal = "ctrl+t"
close_terminal = "ctrl+w"
switch_mode = "esc"

[layout]
default = "grid"
min_pane_size = { width = 40, height = 10 }
border_style = "rounded"

[git]
auto_worktree = true
sync_interval = 300  # seconds
commit_template = "feat: {message}\n\nCo-authored-by: RGB"

[terminals]
claude = { command = "claude", icon = "🤖" }
vim = { command = "vim", icon = "📝" }
custom = { command = "$SHELL", icon = ">" }
```

## Performance Considerations

1. **Terminal Rendering**:
   - Use dirty region tracking
   - Implement viewport culling
   - Cache rendered cells

2. **File Monitoring**:
   - Debounce file system events
   - Use inotify/FSEvents efficiently
   - Limit watch depth

3. **Git Operations**:
   - Cache status results
   - Async worktree operations
   - Incremental diff updates

4. **Memory Management**:
   - Limit scrollback buffer size
   - Compress inactive terminal state
   - Use Arc/Rc for shared data

## Security Considerations

1. **Process Isolation**: Each terminal runs in separate PTY
2. **Environment Sanitization**: Clean environment variables
3. **No Credential Storage**: Use system keychain integration
4. **Audit Logging**: Optional command history logging

## Testing Strategy

1. **Unit Tests**: Component-level testing
2. **Integration Tests**: Terminal emulation verification
3. **E2E Tests**: Full workflow testing with expect
4. **Performance Tests**: Benchmark critical paths
5. **Fuzz Testing**: Input handling robustness

## Development Phases

### Phase 1: Core Foundation (Week 1-2)
- [ ] Project setup and dependencies
- [ ] Basic TUI with ratatui
- [ ] Terminal emulator integration
- [ ] Simple layout engine

### Phase 2: Terminal Management (Week 3-4)
- [ ] Multiple terminal support
- [ ] PTY handling and process management
- [ ] Basic file tracking
- [ ] Terminal grid layout

### Phase 3: Git Integration (Week 5-6)
- [ ] Worktree management
- [ ] Status monitoring
- [ ] Diff visualization
- [ ] Commit interface

### Phase 4: Advanced Features (Week 7-8)
- [ ] Drag-and-drop layout
- [ ] File conflict detection
- [ ] Configuration system
- [ ] Keyboard shortcuts

### Phase 5: Polish & Performance (Week 9-10)
- [ ] Performance optimization
- [ ] Theme support
- [ ] Documentation
- [ ] Testing suite

## Migration Guide from AIWorkbench

### Feature Mapping

| AIWorkbench (Swift) | RGB (Rust) | Notes |
|-------------------|------------|-------|
| SwiftUI Views | Ratatui Widgets | Complete rewrite |
| SwiftTerm | alacritty_terminal | Different API |
| NSFileManager | notify-rs | Cross-platform |
| Git CLI calls | libgit2 | More efficient |
| Core Data | In-memory + serde | Simpler model |

### Data Migration
- Export workspace configuration as JSON
- Import terminal layouts and preferences
- Preserve git worktree associations

## Future Enhancements

1. **Plugin System**: WASM-based plugins for custom tools
2. **Remote Sessions**: SSH/mosh integration
3. **Collaborative Mode**: Shared terminal sessions
4. **AI Integration**: Built-in LLM assistance
5. **Recording/Playback**: Terminal session recording

## Conclusion

RGB represents a complete reimagining of AIWorkbench as a native Rust TUI application. By leveraging Rust's performance and safety guarantees along with modern TUI libraries, we can create a more efficient, portable, and feature-rich terminal workspace manager.

The single-process architecture with sub-terminals provides a unified experience while maintaining the isolation benefits of the original design through git worktrees. The keyboard-driven interface and extensive customization options will appeal to power users while remaining accessible to newcomers.