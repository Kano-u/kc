use crate::history::load_history;
use crate::notes::NoteStore;
use crate::tui;
use regex::Regex;
use std::path::PathBuf;

fn env_dir(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

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
    if let Some(dir) = env_dir("KC_CONFIG_DIR") {
        return dir;
    }
    if cfg!(windows) {
        home_dir().join(".config").join("kc")
    } else if let Some(dir) = std::env::var_os("XDG_CONFIG_HOME") {
        PathBuf::from(dir).join("kc")
    } else {
        home_dir().join(".config").join("kc")
    }
}

pub fn data_paths() -> Result<crate::data_paths::DataPaths, String> {
    crate::data_paths::resolve(&config_dir())
}

#[derive(Debug, Clone)]
pub struct Config {
    pub history_limit: u32,
    pub filters: Vec<Regex>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            history_limit: 5000,
            filters: Vec::new(),
        }
    }
}

impl Config {
    pub fn load() -> Result<Self, String> {
        let paths = data_paths()?;
        Ok(Self::load_from(&paths.config))
    }

    fn load_from(path: &std::path::Path) -> Self {
        let Ok(text) = std::fs::read_to_string(path) else {
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
            filters: parse_filters(value.get("filter")),
        }
    }
}

fn parse_filters(value: Option<&toml::Value>) -> Vec<Regex> {
    let Some(values) = value.and_then(toml::Value::as_array) else {
        return Vec::new();
    };

    values
        .iter()
        .filter_map(|value| value.as_str())
        .filter_map(|pattern| match Regex::new(pattern) {
            Ok(regex) => Some(regex),
            Err(error) => {
                eprintln!("忽略无效的过滤正则 {pattern:?}: {error}");
                None
            }
        })
        .collect()
}

pub fn run(query: Option<&str>) -> std::process::ExitCode {
    let config = match Config::load() {
        Ok(config) => config,
        Err(error) => {
            eprintln!("{error}");
            return std::process::ExitCode::FAILURE;
        }
    };
    let query = query.unwrap_or_default();
    let history = match load_history(config.history_limit, &config.filters) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_regex_filter_list() {
        let value: toml::Value =
            toml::from_str(r#"filter = ["^dir$", "secret", "[invalid"]"#).unwrap();
        let filters = parse_filters(value.get("filter"));
        assert_eq!(filters.len(), 2);
        assert!(filters[0].is_match("dir"));
        assert!(!filters[0].is_match("dirty"));
        assert!(filters[1].is_match("echo secret"));
    }

    #[test]
    fn missing_filter_list_is_empty() {
        let value: toml::Value = toml::from_str("history_limit = 10").unwrap();
        assert!(parse_filters(value.get("filter")).is_empty());
    }

    #[test]
    fn empty_environment_paths_are_ignored() {
        assert_eq!(env_dir("KC_TEST_UNSET_PATH"), None);
        std::env::set_var("KC_TEST_EMPTY_PATH", "");
        assert_eq!(env_dir("KC_TEST_EMPTY_PATH"), None);
        std::env::remove_var("KC_TEST_EMPTY_PATH");
    }

    #[test]
    fn environment_path_is_used_verbatim() {
        let path = std::env::temp_dir().join("kc-environment-path-test");
        std::env::set_var("KC_TEST_CONFIG_DIR", &path);
        assert_eq!(env_dir("KC_TEST_CONFIG_DIR"), Some(path));
        std::env::remove_var("KC_TEST_CONFIG_DIR");
    }
}
