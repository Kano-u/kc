use crate::history::load_history;
use crate::notes::NoteStore;
use crate::tui;
use std::path::PathBuf;

fn home_dir() -> PathBuf {
    if let Some(home) = std::env::var_os("USERPROFILE") {
        return PathBuf::from(home);
    }
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home);
    }
    PathBuf::from(".")
}

fn config_dir() -> PathBuf {
    if cfg!(windows) {
        home_dir().join(".config").join("kc")
    } else if let Some(dir) = std::env::var_os("XDG_CONFIG_HOME") {
        PathBuf::from(dir).join("kc")
    } else {
        home_dir().join(".config").join("kc")
    }
}

pub fn data_dir() -> PathBuf {
    config_dir().join("data")
}

pub fn notes_path() -> PathBuf {
    data_dir().join("notes.jsonl")
}

pub fn config_path() -> PathBuf {
    config_dir().join("config.toml")
}

#[derive(Debug, Clone)]
pub struct Config {
    pub history_limit: u32,
}

impl Default for Config {
    fn default() -> Self {
        Self { history_limit: 5000 }
    }
}

impl Config {
    pub fn load() -> Self {
        let Ok(text) = std::fs::read_to_string(config_path()) else {
            return Self::default();
        };
        let value: toml::Value = match toml::from_str(&text) {
            Ok(value) => value,
            Err(error) => {
                eprintln!("配置文件解析失败，使用默认设置: {error}");
                return Self::default();
            }
        };
        Self {
            history_limit: value
                .get("history_limit")
                .and_then(|value| value.as_integer())
                .and_then(|value| u32::try_from(value).ok())
                .unwrap_or(5000),
        }
    }
}

pub fn run(query: Option<&str>) -> std::process::ExitCode {
    let config = Config::load();
    let query = query.unwrap_or_default();
    let history = match load_history(config.history_limit) {
        Ok(history) => history,
        Err(error) => {
            eprintln!("{error}");
            return std::process::ExitCode::FAILURE;
        }
    };
    let mut notes = match NoteStore::load() {
        Ok(notes) => notes,
        Err(error) => {
            eprintln!("{error}");
            return std::process::ExitCode::FAILURE;
        }
    };

    match tui::run(query, &history, &mut notes) {
        Ok(_) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            std::process::ExitCode::FAILURE
        }
    }
}
