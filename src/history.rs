use crate::app::Config;
use rusqlite::{Connection, OpenFlags};
use std::path::Path;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS history (
    command TEXT PRIMARY KEY,
    at      INTEGER NOT NULL,
    ok      INTEGER NOT NULL
);
";

fn open(path: &Path) -> Result<Connection, String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| format!("创建历史目录失败: {error}"))?;
    }
    let connection =
        Connection::open(path).map_err(|error| format!("打开历史数据库失败: {error}"))?;
    connection
        .execute_batch(SCHEMA)
        .map_err(|error| format!("初始化历史数据库失败: {error}"))?;
    Ok(connection)
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|value| value.as_secs() as i64)
        .unwrap_or_default()
}

/// 失败的命令同样记录，`ok` 只存着，界面不区分。
pub fn upsert(path: &Path, command: &str, ok: bool) -> Result<(), String> {
    let connection = open(path)?;
    connection
        .execute(
            "INSERT INTO history (command, at, ok) VALUES (?1, ?2, ?3)
             ON CONFLICT(command) DO UPDATE SET at = excluded.at, ok = excluded.ok",
            rusqlite::params![command, now(), i64::from(ok)],
        )
        .map_err(|error| format!("写入历史失败: {error}"))?;
    Ok(())
}

/// 表按 `at` 升序返回，最旧在上；filter 命中的不返回，再取最后 `history_limit` 条。
pub fn load(path: &Path, config: &Config) -> Result<Vec<String>, String> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|error| format!("打开历史数据库失败: {error}"))?;
    let mut statement = connection
        .prepare("SELECT command FROM history ORDER BY at")
        .map_err(|error| format!("读取历史失败: {error}"))?;
    let mut rows = statement
        .query([])
        .map_err(|error| format!("读取历史失败: {error}"))?;

    let mut history = Vec::new();
    while let Some(row) = rows
        .next()
        .map_err(|error| format!("读取历史失败: {error}"))?
    {
        history.push(
            row.get::<_, String>(0)
                .map_err(|error| format!("读取历史失败: {error}"))?,
        );
    }

    history.retain(|command| !config.filtered(command));
    let limit = config.history_limit as usize;
    if history.len() > limit {
        history.drain(..history.len() - limit);
    }
    Ok(history)
}

/// 批量导入外部历史：整体一个事务，`at` 从当前时间往前铺开、按顺序递增，
/// 已有的同名命令被刷新成导入时的时间戳。返回写入的命令条数。
pub fn import(path: &Path, commands: &[String]) -> Result<usize, String> {
    let mut connection = open(path)?;
    let transaction = connection
        .transaction()
        .map_err(|error| format!("写入历史失败: {error}"))?;
    let mut at = now() - commands.len() as i64;
    for command in commands {
        at += 1;
        transaction
            .execute(
                "INSERT INTO history (command, at, ok) VALUES (?1, ?2, 1)
                 ON CONFLICT(command) DO UPDATE SET at = excluded.at, ok = excluded.ok",
                rusqlite::params![command, at],
            )
            .map_err(|error| format!("写入历史失败: {error}"))?;
    }
    transaction
        .commit()
        .map_err(|error| format!("写入历史失败: {error}"))?;
    Ok(commands.len())
}

/// 物理删除，文件里不留痕迹。
pub fn delete(path: &Path, command: &str) -> Result<(), String> {
    let connection = open(path)?;
    connection
        .execute("DELETE FROM history WHERE command = ?1", [command])
        .map_err(|error| format!("删除历史失败: {error}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use regex::Regex;

    fn temp_db(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("kc-history-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join("history.db")
    }

    /// 直接写库以控制时间戳：`upsert` 用的是当前秒，同一秒内多条无法断言先后。
    fn seed(path: &std::path::Path, rows: &[(&str, i64, i64)]) {
        let connection = open(path).unwrap();
        for (command, at, ok) in rows {
            connection
                .execute(
                    "INSERT INTO history (command, at, ok) VALUES (?1, ?2, ?3)",
                    rusqlite::params![command, at, ok],
                )
                .unwrap();
        }
    }

    fn config(limit: u32, filters: &[&str]) -> Config {
        Config {
            history_limit: limit,
            filters: filters
                .iter()
                .map(|pattern| Regex::new(pattern).unwrap())
                .collect(),
        }
    }

    fn field(path: &std::path::Path, command: &str) -> (i64, i64) {
        let connection = open(path).unwrap();
        connection
            .query_row(
                "SELECT at, ok FROM history WHERE command = ?1",
                [command],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap()
    }

    #[test]
    fn loads_oldest_first() {
        let path = temp_db("order");
        seed(
            &path,
            &[("cargo test", 30, 1), ("dir", 10, 1), ("cd ..", 20, 0)],
        );
        assert_eq!(
            load(&path, &config(10, &[])).unwrap(),
            vec!["dir", "cd ..", "cargo test"]
        );
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn upsert_deduplicates_and_refreshes_the_row() {
        let path = temp_db("upsert");
        seed(&path, &[("dir", 1, 1)]);
        upsert(&path, "dir", false).unwrap();
        upsert(&path, "ls", true).unwrap();

        let history = load(&path, &config(10, &[])).unwrap();
        assert_eq!(history.len(), 2, "重复命令不应新增行: {history:?}");
        let (at, ok) = field(&path, "dir");
        assert!(at > 1, "upsert 应刷新时间戳: {at}");
        assert_eq!(ok, 0, "upsert 应覆盖成功/失败标记");
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn keeps_the_newest_entries_within_the_limit() {
        let path = temp_db("limit");
        seed(&path, &[("one", 1, 1), ("two", 2, 1), ("three", 3, 1)]);
        assert_eq!(load(&path, &config(2, &[])).unwrap(), vec!["two", "three"]);
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn filters_matching_commands_out_of_the_list() {
        let path = temp_db("filter");
        seed(
            &path,
            &[
                ("git status", 1, 1),
                ("echo secret", 2, 1),
                ("cargo test", 3, 1),
            ],
        );

        let filters = ["secret", "^cargo"];
        assert_eq!(
            load(&path, &config(10, &filters)).unwrap(),
            vec!["git status"]
        );
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn import_keeps_the_given_order_and_deduplicates() {
        let path = temp_db("import");
        let commands: Vec<String> = ["dir", "ls", "dir"]
            .iter()
            .map(|command| (*command).to_owned())
            .collect();

        assert_eq!(import(&path, &commands).unwrap(), 3);
        let history = load(&path, &config(10, &[])).unwrap();
        assert_eq!(
            history,
            vec!["ls", "dir"],
            "重复命令只留一行、时间取最后: {history:?}"
        );
        let (at, ok) = field(&path, "dir");
        assert!(at > field(&path, "ls").0, "后出现的重复命令时间戳应更新");
        assert_eq!(ok, 1);
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn import_puts_entries_at_the_end_of_the_history() {
        let path = temp_db("import-order");
        seed(&path, &[("old", 1, 1)]);

        import(&path, &["first".to_owned(), "second".to_owned()]).unwrap();
        assert_eq!(
            load(&path, &config(10, &[])).unwrap(),
            vec!["old", "first", "second"]
        );
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn delete_removes_the_row() {
        let path = temp_db("delete");
        seed(&path, &[("dir", 1, 1), ("ls", 2, 1)]);

        delete(&path, "dir").unwrap();
        assert_eq!(load(&path, &config(10, &[])).unwrap(), vec!["ls"]);
        // 幂等：再删一次不报错。
        delete(&path, "dir").unwrap();
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn missing_database_is_an_empty_history() {
        let dir = std::env::temp_dir().join(format!("kc-history-missing-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(load(&dir.join("history.db"), &config(10, &[]))
            .unwrap()
            .is_empty());
    }
}
