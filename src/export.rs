use crate::args;
use crate::data_paths::{history_db, preview_cache};
use crate::history;
use crate::notes::NoteStore;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const USAGE: &str = "kc export";
const ALLOWED: &[&str] = &[];

pub fn main(args: &[String]) -> ExitCode {
    if let Err(error) = args::parse(args, ALLOWED) {
        eprintln!("{error}\n用法: {USAGE}");
        return ExitCode::from(2);
    }
    match refresh() {
        Ok((path, count)) => {
            println!("已导出 {count} 条命令到 {}。", path.display());
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("kc export: {error}");
            ExitCode::FAILURE
        }
    }
}

/// 把历史与备注写成预测器要读的缓存文件，返回路径与导出的条数。
///
/// `kc record` 每次写库成功后也会调用它，所以这里不做任何输出。
pub fn refresh() -> Result<(PathBuf, usize), String> {
    let history = history::load(&history_db()?, u32::MAX)?;
    let notes = NoteStore::load()?;
    refresh_from(&history, &notes)
}

/// 用调用方手里的历史快照落盘：不重开数据库，也不重解析备注文件。
/// TUI 退出时走这条路 —— 它手上的列表就是删除与改备注之后的权威状态。
pub fn refresh_from(history: &[String], notes: &NoteStore) -> Result<(PathBuf, usize), String> {
    let lines = render(history, notes);
    let path = preview_cache()?;
    write_cache(&path, &lines)?;
    Ok((path, lines.len()))
}

/// 一行一条，`命令\t备注`。命令原样保留 —— 接受建议时插入的就是这里的原文，
/// 所以含换行或 TAB 的命令整条跳过，绝不压平（压平等于改写成另一条命令）。
///
/// 输入是 `history::load` 的顺序（最旧在上），输出反过来：预测器只取前 10 条，
/// 顺序直接决定拿到哪 10 条，只有最近的命令值得预览。
fn render(history: &[String], notes: &NoteStore) -> Vec<String> {
    let mut lines = Vec::with_capacity(history.len());
    for command in history.iter().rev() {
        if command.contains(['\n', '\r', '\t']) {
            continue;
        }
        let note = notes.get(command).map(flatten).unwrap_or_default();
        lines.push(format!("{command}\t{note}"));
    }
    lines
}

/// 备注是纯展示字段，换行与 TAB 会破坏一行一条的格式，写成空格。
fn flatten(note: &str) -> String {
    note.chars()
        .map(|value| match value {
            '\n' | '\r' | '\t' => ' ',
            other => other,
        })
        .collect()
}

/// 临时文件 + `rename`：预测器随时可能在读，不能让它看到半个文件。
fn write_cache(path: &Path, lines: &[String]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| format!("创建数据目录失败: {error}"))?;
    }

    let temp_path = path.with_extension("tsv.tmp");
    {
        let file = std::fs::File::create(&temp_path)
            .map_err(|error| format!("创建临时预览文件失败: {}: {error}", temp_path.display()))?;
        let mut writer = BufWriter::new(file);
        for line in lines {
            writeln!(writer, "{line}").map_err(|error| format!("写入预览缓存失败: {error}"))?;
        }
        writer
            .flush()
            .map_err(|error| format!("写入预览缓存失败: {error}"))?;
    }

    std::fs::rename(&temp_path, path).map_err(|error| {
        let _ = std::fs::remove_file(&temp_path);
        format!("保存预览缓存失败: {error}")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("kc-export-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn notes(path: &Path, entries: &[(&str, &str)]) -> NoteStore {
        let mut store = NoteStore::load_from(path).unwrap();
        for (command, note) in entries {
            store.set(command, note).unwrap();
        }
        store
    }

    fn commands(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn keeps_commands_verbatim_and_pairs_notes() {
        let dir = temp_dir("render");
        let store = notes(&dir.join("com.notes.jsonl"), &[("cargo test", "运行测试")]);
        let lines = render(
            &commands(&["echo \"中文 \\ 路径\"", "cargo test", "git status"]),
            &store,
        );
        assert_eq!(
            lines,
            vec![
                "git status\t",
                "cargo test\t运行测试",
                "echo \"中文 \\ 路径\"\t",
            ]
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn skips_commands_that_are_not_single_line() {
        let dir = temp_dir("skip");
        let store = notes(&dir.join("com.notes.jsonl"), &[]);
        let lines = render(
            &commands(&["dir |\n  more", "ls\r\n", "a\tb", "ls"]),
            &store,
        );
        assert_eq!(lines, vec!["ls\t"]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn writes_the_most_recent_commands_first() {
        // 预测器只取前 10 条：文件里排在后面的命令等于不存在。
        let dir = temp_dir("order");
        let store = notes(&dir.join("com.notes.jsonl"), &[]);
        let lines = render(&commands(&["oldest", "older", "newest"]), &store);
        assert_eq!(lines, vec!["newest\t", "older\t", "oldest\t"]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn flattens_tabs_and_newlines_in_notes() {
        let dir = temp_dir("flatten");
        let store = notes(&dir.join("com.notes.jsonl"), &[("ls", "前\t中\n后")]);
        assert_eq!(render(&commands(&["ls"]), &store), vec!["ls\t前 中 后"]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn writes_utf8_lines_and_creates_the_directory() {
        let dir = temp_dir("write");
        let path = dir.join("nested").join("preview.tsv");
        write_cache(&path, &["ls\t列表".to_owned(), "pwd\t".to_owned()]).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "ls\t列表\npwd\t\n");
        assert!(!path.with_extension("tsv.tmp").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
