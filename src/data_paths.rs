use std::path::{Path, PathBuf};

/// 历史数据库固定在用户目录，与同步目录无关，也不参与同步。
pub fn history_db() -> PathBuf {
    crate::app::home_dir().join(".kc").join("history.db")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataPaths {
    pub config: PathBuf,
    pub read_notes: Vec<PathBuf>,
    pub write_notes: PathBuf,
}

pub fn resolve(config_dir: &Path) -> Result<DataPaths, String> {
    if !config_dir.is_dir() {
        return Err(format!("配置目录不存在: {}", config_dir.display()));
    }

    let hosts = files_with_extension(config_dir, "host")?;
    let host = match hosts.as_slice() {
        [host] => host,
        [] => {
            return Err(format!("未在 {} 中找到 .host 文件。", config_dir.display()));
        }
        _ => {
            let names = hosts
                .iter()
                .filter_map(|path| path.file_name())
                .map(|name| name.to_string_lossy())
                .collect::<Vec<_>>()
                .join(", ");
            return Err(format!(
                "在 {} 中找到多个 .host 文件: {names}",
                config_dir.display()
            ));
        }
    };

    let base = host
        .file_stem()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("无法解析主机标记: {}", host.display()))?;

    let write_notes = config_dir.join(format!("{base}.notes.jsonl"));
    let mut read_notes = notes_files(config_dir)?;
    read_notes.retain(|path| path != &write_notes);
    read_notes.push(write_notes.clone());

    Ok(DataPaths {
        config: config_dir.join(format!("{base}.config.toml")),
        read_notes,
        write_notes,
    })
}

fn files_with_extension(config_dir: &Path, extension: &str) -> Result<Vec<PathBuf>, String> {
    let entries =
        std::fs::read_dir(config_dir).map_err(|error| format!("读取配置目录失败: {error}"))?;
    let mut paths = Vec::new();
    for entry in entries {
        let path = entry
            .map_err(|error| format!("读取配置目录失败: {error}"))?
            .path();
        if path.is_file()
            && path
                .extension()
                .is_some_and(|value| value.eq_ignore_ascii_case(extension))
        {
            paths.push(path);
        }
    }
    paths.sort();
    Ok(paths)
}

fn notes_files(config_dir: &Path) -> Result<Vec<PathBuf>, String> {
    let entries =
        std::fs::read_dir(config_dir).map_err(|error| format!("读取配置目录失败: {error}"))?;
    let mut paths = Vec::new();
    for entry in entries {
        let path = entry
            .map_err(|error| format!("读取配置目录失败: {error}"))?
            .path();
        if path.is_file()
            && path
                .file_name()
                .and_then(|value| value.to_str())
                .is_some_and(|name| name.to_ascii_lowercase().ends_with(".notes.jsonl"))
        {
            paths.push(path);
        }
    }
    paths.sort();
    Ok(paths)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("kc-data-paths-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn history_database_lives_outside_the_sync_directory() {
        let path = history_db();
        assert!(path.ends_with(Path::new(".kc").join("history.db")));
        std::env::set_var("KC_CONFIG_DIR", temp_dir("history-isolated"));
        assert_eq!(history_db(), path);
        std::env::remove_var("KC_CONFIG_DIR");
    }

    #[test]
    fn requires_a_host_file() {
        let dir = temp_dir("missing-host");
        let error = resolve(&dir).unwrap_err();
        assert!(error.contains("未在"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn rejects_multiple_host_files() {
        let dir = temp_dir("multiple-hosts");
        std::fs::write(dir.join("com.host"), "").unwrap();
        std::fs::write(dir.join("phone.host"), "").unwrap();
        let error = resolve(&dir).unwrap_err();
        assert!(error.contains("多个 .host"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn host_file_selects_config_and_collects_all_notes() {
        let dir = temp_dir("host");
        std::fs::write(dir.join("com.host"), "").unwrap();
        std::fs::write(dir.join("com.notes.jsonl"), "").unwrap();
        std::fs::write(dir.join("phone.notes.jsonl"), "").unwrap();

        let paths = resolve(&dir).unwrap();
        assert_eq!(paths.config, dir.join("com.config.toml"));
        assert_eq!(paths.write_notes, dir.join("com.notes.jsonl"));
        assert_eq!(
            paths.read_notes,
            vec![dir.join("phone.notes.jsonl"), dir.join("com.notes.jsonl")]
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn adds_the_current_write_file_to_the_read_list() {
        let dir = temp_dir("write-file");
        std::fs::write(dir.join("com.host"), "").unwrap();
        std::fs::write(dir.join("phone.notes.jsonl"), "").unwrap();

        let paths = resolve(&dir).unwrap();
        assert_eq!(
            paths.read_notes,
            vec![dir.join("phone.notes.jsonl"), dir.join("com.notes.jsonl")]
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
