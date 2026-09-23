use std::path::{Path, PathBuf};

/// 历史数据库固定在用户目录，与同步目录无关，也不参与同步。
pub fn history_db() -> Result<PathBuf, String> {
    Ok(home_dir()?.join(".kc").join("history.db"))
}

/// 命令预览缓存，与历史数据库同目录：同为本机数据，不参与同步。
/// 只给 PowerShell 预测器读，路径固定，shell 侧不需要环境变量。
pub fn preview_cache() -> Result<PathBuf, String> {
    Ok(home_dir()?.join(".kc").join("preview.tsv"))
}

/// Windows 用 `USERPROFILE`，Termux 等用 `HOME`；两者都没有就报错，
/// 绝不能退化到当前目录，否则历史与备注会写进任意工作目录。
fn home_dir() -> Result<PathBuf, String> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| "USERPROFILE 与 HOME 都未设置，无法定位用户目录。".to_owned())
}

/// 配置与数据目录必须由 `KC_CONFIG_DIR` 指定，不再猜测默认位置。
fn config_dir() -> Result<PathBuf, String> {
    config_dir_from(std::env::var_os("KC_CONFIG_DIR").as_deref())
}

fn config_dir_from(value: Option<&std::ffi::OsStr>) -> Result<PathBuf, String> {
    value
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| "环境变量 KC_CONFIG_DIR 未设置，无法定位配置目录。".to_owned())
}

/// 主机名决定配置文件与备注写入文件，两者都从目录与主机名派生。
#[derive(Debug, Clone)]
pub struct DataPaths {
    dir: PathBuf,
    host: String,
}

impl DataPaths {
    pub fn load() -> Result<Self, String> {
        resolve(&config_dir()?)
    }

    pub fn config(&self) -> PathBuf {
        self.dir.join(format!("{}.config.toml", self.host))
    }

    pub fn write_notes(&self) -> PathBuf {
        self.dir.join(format!("{}.notes.jsonl", self.host))
    }

    /// 其他主机的备注文件；当前主机自己的备注由 `NoteStore` 单独加载，
    /// 以便保存时只写回自己这份。同名备注按 `updated_at` 取较新的记录。
    pub fn read_notes(&self) -> Result<Vec<PathBuf>, String> {
        let write = self.write_notes();
        let mut paths = files(&self.dir, |path| {
            path.file_name()
                .and_then(|value| value.to_str())
                .is_some_and(|name| name.ends_with(".notes.jsonl"))
        })?;
        paths.retain(|path| path != &write);
        Ok(paths)
    }
}

fn resolve(config_dir: &Path) -> Result<DataPaths, String> {
    let hosts = files(config_dir, |path| {
        path.extension().is_some_and(|value| value == "host")
    })?;
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

    let host = host
        .file_stem()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("无法解析主机标记: {}", host.display()))?;

    Ok(DataPaths {
        dir: config_dir.to_owned(),
        host: host.to_owned(),
    })
}

fn files(config_dir: &Path, matches: impl Fn(&Path) -> bool) -> Result<Vec<PathBuf>, String> {
    let entries =
        std::fs::read_dir(config_dir).map_err(|error| format!("读取配置目录失败: {error}"))?;
    let mut paths = Vec::new();
    for entry in entries {
        let path = entry
            .map_err(|error| format!("读取配置目录失败: {error}"))?
            .path();
        if path.is_file() && matches(&path) {
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
    fn requires_kc_config_dir_instead_of_guessing_a_default() {
        let error = config_dir_from(None).unwrap_err();
        assert!(error.contains("KC_CONFIG_DIR"), "{error}");
        assert!(config_dir_from(Some(std::ffi::OsStr::new(""))).is_err());
        assert_eq!(
            config_dir_from(Some(std::ffi::OsStr::new("/tmp/kc"))).unwrap(),
            PathBuf::from("/tmp/kc")
        );
    }

    #[test]
    fn history_database_lives_outside_the_sync_directory() {
        let path = history_db().unwrap();
        assert!(path.ends_with(Path::new(".kc").join("history.db")));
        let config = temp_dir("history-isolated");
        std::env::set_var("KC_CONFIG_DIR", &config);
        assert_eq!(history_db().unwrap(), path);
        assert!(!path.starts_with(&config), "{}", path.display());
        std::env::remove_var("KC_CONFIG_DIR");
        std::fs::remove_dir_all(config).unwrap();
    }

    #[test]
    fn preview_cache_lives_next_to_the_history_database() {
        let path = preview_cache().unwrap();
        assert!(path.ends_with(Path::new(".kc").join("preview.tsv")));
        assert_eq!(path.parent(), history_db().unwrap().parent());
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
        assert_eq!(paths.config(), dir.join("com.config.toml"));
        assert_eq!(paths.write_notes(), dir.join("com.notes.jsonl"));
        assert_eq!(
            paths.read_notes().unwrap(),
            vec![dir.join("phone.notes.jsonl")]
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn skips_the_current_write_file_in_the_read_list() {
        let dir = temp_dir("write-file");
        std::fs::write(dir.join("com.host"), "").unwrap();
        std::fs::write(dir.join("phone.notes.jsonl"), "").unwrap();

        let paths = resolve(&dir).unwrap();
        assert_eq!(
            paths.read_notes().unwrap(),
            vec![dir.join("phone.notes.jsonl")]
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
