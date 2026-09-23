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
        Self::load_from(&paths.config())
    }

    /// 配置文件不存在是正常的（用默认值）；读不到、解析不了、字段非法都直接报错，
    /// 绝不静默回退到默认值。
    fn load_from(path: &std::path::Path) -> Result<Self, String> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(error) => {
                return Err(format!("读取配置文件 {} 失败: {error}", path.display()));
            }
        };
        let value: toml::Value = toml::from_str(&text)
            .map_err(|error| format!("配置文件 {} 解析失败: {error}", path.display()))?;

        let default = Self::default();
        let history_limit = match value.get("history_limit") {
            None => default.history_limit,
            Some(value) => value
                .as_integer()
                .and_then(|value| u32::try_from(value).ok())
                .ok_or_else(|| {
                    format!(
                        "配置文件 {} 的 history_limit 必须是非负整数。",
                        path.display()
                    )
                })?,
        };

        Ok(Self {
            history_limit,
            filters: parse_filters(value.get("filter"))
                .map_err(|error| format!("配置文件 {}: {error}", path.display()))?,
        })
    }

    /// filter 命中的命令不显示、不记录，也不导入。
    pub fn filtered(&self, command: &str) -> bool {
        self.filters.iter().any(|regex| regex.is_match(command))
    }
}

/// filter 缺失即空列表；但类型不对、含非字符串项、或正则非法都直接报错。
fn parse_filters(value: Option<&toml::Value>) -> Result<Vec<Regex>, String> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let values = value
        .as_array()
        .ok_or_else(|| "filter 必须是字符串数组。".to_owned())?;

    let mut filters = Vec::with_capacity(values.len());
    for value in values {
        let pattern = value
            .as_str()
            .ok_or_else(|| format!("filter 的每一项都必须是字符串，遇到 {value}。"))?;
        filters.push(
            Regex::new(pattern).map_err(|error| format!("无效的过滤正则 {pattern:?}: {error}"))?,
        );
    }
    Ok(filters)
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

    fn filters(value: &str) -> Result<Vec<Regex>, String> {
        let value: toml::Value = toml::from_str(value).unwrap();
        parse_filters(value.get("filter"))
    }

    fn temp_config(name: &str, contents: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("kc-config-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("com.config.toml");
        std::fs::write(&path, contents).unwrap();
        path
    }

    #[test]
    fn parses_regex_filter_list() {
        let filters = filters(r#"filter = ["^dir$", "secret"]"#).unwrap();
        assert_eq!(filters.len(), 2);
        assert!(filters[0].is_match("dir"));
        assert!(!filters[0].is_match("dirty"));
        assert!(filters[1].is_match("echo secret"));
    }

    #[test]
    fn missing_config_uses_defaults() {
        let path = std::env::temp_dir().join(format!(
            "kc-config-missing-{}/com.config.toml",
            std::process::id()
        ));
        let config = Config::load_from(&path).unwrap();
        assert_eq!(config.history_limit, 5000);
        assert!(config.filters.is_empty());
    }

    #[test]
    fn missing_filter_list_is_empty() {
        assert!(filters("history_limit = 10").unwrap().is_empty());
    }

    #[test]
    fn invalid_regex_is_an_error() {
        let error = filters(r#"filter = ["^dir$", "[invalid"]"#).unwrap_err();
        assert!(error.contains("无效的过滤正则"), "{error}");
    }

    #[test]
    fn non_string_filter_entry_is_an_error() {
        let error = filters("filter = [\"dir\", 3]").unwrap_err();
        assert!(error.contains("必须是字符串"), "{error}");
    }

    #[test]
    fn non_array_filter_is_an_error() {
        let error = filters(r#"filter = "dir""#).unwrap_err();
        assert!(error.contains("必须是字符串数组"), "{error}");
    }

    #[test]
    fn invalid_history_limit_is_an_error() {
        for contents in ["history_limit = -1", "history_limit = \"many\""] {
            let path = temp_config("limit", contents);
            let error = Config::load_from(&path).unwrap_err();
            assert!(error.contains("history_limit"), "{contents}: {error}");
            std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
        }
    }

    #[test]
    fn broken_toml_is_an_error() {
        let path = temp_config("broken", "history_limit = [");
        let error = Config::load_from(&path).unwrap_err();
        assert!(error.contains("解析失败"), "{error}");
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn reads_a_valid_config_file() {
        let path = temp_config("valid", "history_limit = 10\nfilter = [\"^dir$\"]\n");
        let config = Config::load_from(&path).unwrap();
        assert_eq!(config.history_limit, 10);
        assert!(config.filtered("dir"));
        assert!(!config.filtered("dirty"));
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn filtered_matches_any_pattern() {
        let value: toml::Value = toml::from_str(r#"filter = ["^dir$", "secret"]"#).unwrap();
        let config = Config {
            history_limit: 10,
            filters: parse_filters(value.get("filter")).unwrap(),
        };
        assert!(config.filtered("dir"));
        assert!(config.filtered("echo secret"));
        assert!(!config.filtered("dirty"));
        assert!(!Config::default().filtered("echo token=abc"));
    }
}
