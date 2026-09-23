use crate::data_paths::DataPaths;
use crate::history;
use crate::notes::NoteStore;
use crate::tui;
use regex::Regex;

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
        let paths = DataPaths::load()?;
        Ok(Self::load_from(&paths.config()))
    }

    fn load_from(path: &std::path::Path) -> Self {
        let default = Self::default();
        let Ok(text) = std::fs::read_to_string(path) else {
            return default;
        };
        let value: toml::Value = match toml::from_str(&text) {
            Ok(value) => value,
            Err(error) => {
                eprintln!("配置文件解析失败，使用默认设置: {error}");
                return default;
            }
        };
        Self {
            history_limit: value
                .get("history_limit")
                .and_then(|value| value.as_integer())
                .and_then(|value| u32::try_from(value).ok())
                .unwrap_or(default.history_limit),
            filters: parse_filters(value.get("filter")),
        }
    }

    /// filter 命中的命令不显示、不记录，也不导入。
    pub fn filtered(&self, command: &str) -> bool {
        self.filters.iter().any(|regex| regex.is_match(command))
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
    match run_main(query.unwrap_or_default()) {
        Ok(_) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            std::process::ExitCode::FAILURE
        }
    }
}

/// 裸 `kc` 与 `kc pick` 唯一的数据加载入口，两者只在结果如何输出上不同。
pub fn run_main(query: &str) -> Result<crate::tui::PickResult, String> {
    let config = Config::load()?;
    let history = history::load(&crate::data_paths::history_db()?, &config)?;
    let mut notes = NoteStore::load()?;
    tui::run(query, &history, &mut notes)
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
    fn filtered_matches_any_pattern() {
        let value: toml::Value = toml::from_str(r#"filter = ["^dir$", "secret"]"#).unwrap();
        let config = Config {
            history_limit: 10,
            filters: parse_filters(value.get("filter")),
        };
        assert!(config.filtered("dir"));
        assert!(config.filtered("echo secret"));
        assert!(!config.filtered("dirty"));
        assert!(!Config::default().filtered("echo token=abc"));
    }
}
