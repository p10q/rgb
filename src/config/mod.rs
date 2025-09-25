use anyhow::Result;
use config::{Config, ConfigError, Environment, File};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct AppConfig {
    pub general: GeneralConfig,
    pub appearance: AppearanceConfig,
    pub keybindings: KeybindingsConfig,
    pub layout: LayoutConfig,
    pub git: GitConfig,
    pub terminals: HashMap<String, TerminalConfig>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct GeneralConfig {
    pub project_dir: Option<PathBuf>,
    pub max_terminals: usize,
    pub auto_save_layout: bool,
    pub default_shell: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct AppearanceConfig {
    pub theme: String,
    pub font_size: u16,
    pub cursor_style: CursorStyle,
    pub scrollback_lines: usize,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub enum CursorStyle {
    Block,
    Line,
    Underline,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct KeybindingsConfig {
    pub new_terminal: String,
    pub close_terminal: String,
    pub switch_mode: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct LayoutConfig {
    pub default: String,
    pub min_pane_size: Size,
    pub border_style: BorderStyle,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Size {
    pub width: u16,
    pub height: u16,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub enum BorderStyle {
    Rounded,
    Double,
    Thick,
    Plain,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct GitConfig {
    pub auto_worktree: bool,
    pub sync_interval: u64,
    pub commit_template: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct TerminalConfig {
    pub command: String,
    pub icon: String,
    pub environment: HashMap<String, String>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            general: GeneralConfig {
                project_dir: None,
                max_terminals: 10,
                auto_save_layout: true,
                default_shell: std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string()),
            },
            appearance: AppearanceConfig {
                theme: "dark".to_string(),
                font_size: 12,
                cursor_style: CursorStyle::Block,
                scrollback_lines: 10000,
            },
            keybindings: KeybindingsConfig {
                new_terminal: "ctrl+t".to_string(),
                close_terminal: "ctrl+w".to_string(),
                switch_mode: "esc".to_string(),
            },
            layout: LayoutConfig {
                default: "grid".to_string(),
                min_pane_size: Size {
                    width: 40,
                    height: 10,
                },
                border_style: BorderStyle::Rounded,
            },
            git: GitConfig {
                auto_worktree: true,
                sync_interval: 300,
                commit_template: "feat: {message}\n\nCo-authored-by: RGB".to_string(),
            },
            terminals: default_terminals(),
        }
    }
}

fn default_terminals() -> HashMap<String, TerminalConfig> {
    let mut terminals = HashMap::new();

    terminals.insert(
        "claude".to_string(),
        TerminalConfig {
            command: "claude".to_string(),
            icon: "🤖".to_string(),
            environment: HashMap::new(),
        },
    );

    terminals.insert(
        "vim".to_string(),
        TerminalConfig {
            command: "vim".to_string(),
            icon: "📝".to_string(),
            environment: HashMap::new(),
        },
    );

    terminals.insert(
        "shell".to_string(),
        TerminalConfig {
            command: std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string()),
            icon: ">".to_string(),
            environment: HashMap::new(),
        },
    );

    terminals
}

pub fn load_config(config_path: Option<PathBuf>) -> Result<AppConfig> {
    // If no config path is specified and no configs exist, just use defaults
    let has_config = config_path.is_some() ||
        dirs::home_dir().map(|h| h.join(".config").join("rgb").join("config.toml").exists()).unwrap_or(false);

    if !has_config {
        return Ok(AppConfig::default());
    }

    let mut builder = Config::builder();

    // Start with defaults
    builder = builder.add_source(Config::try_from(&AppConfig::default())?);

    // Add system config if it exists
    if let Some(proj_dirs) = ProjectDirs::from("com", "rgb", "rgb") {
        let system_config = proj_dirs.config_dir().join("config.toml");
        if system_config.exists() {
            builder = builder.add_source(File::from(system_config));
        }
    }

    // Add user config if it exists
    if let Some(home) = dirs::home_dir() {
        let user_config = home.join(".config").join("rgb").join("config.toml");
        if user_config.exists() {
            builder = builder.add_source(File::from(user_config));
        }
    }

    // Add specified config file
    if let Some(path) = config_path {
        builder = builder.add_source(File::from(path));
    }

    // Add environment variables with RGB_ prefix
    builder = builder.add_source(Environment::with_prefix("RGB").separator("_"));

    let config = builder.build()?;
    Ok(config.try_deserialize()?)
}

pub fn save_config(config: &AppConfig, path: Option<PathBuf>) -> Result<()> {
    let config_path = path.unwrap_or_else(|| {
        dirs::home_dir()
            .unwrap()
            .join(".config")
            .join("rgb")
            .join("config.toml")
    });

    // Ensure directory exists
    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let toml_string = toml::to_string_pretty(config)?;
    std::fs::write(config_path, toml_string)?;

    Ok(())
}