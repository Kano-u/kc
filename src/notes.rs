use crate::data_paths::DataPaths;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Note {
    pub command: String,
    pub note: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Default)]
pub struct NoteStore {
    notes: Vec<Note>,
    local_notes: Vec<Note>,
    path: Option<PathBuf>,
}

impl NoteStore {
    pub fn load() -> Result<Self, String> {
        let paths = DataPaths::load()?;
        Self::load_from_paths(&paths.read_notes()?, &paths.write_notes())
    }

    #[cfg(test)]
    pub fn load_from(path: &Path) -> Result<Self, String> {
        Self::load_from_paths(&[path.to_owned()], path)
    }

    pub fn load_from_paths(read_paths: &[PathBuf], write_path: &Path) -> Result<Self, String> {
        let mut notes: Vec<Note> = Vec::new();
        for path in read_paths {
            merge_file(&mut notes, path)?;
        }

        let mut local_notes = Vec::new();
        merge_file(&mut local_notes, write_path)?;
        merge_notes(&mut notes, &local_notes);

        Ok(Self {
            notes,
            local_notes,
            path: Some(write_path.to_owned()),
        })
    }

    pub fn get(&self, command: &str) -> Option<&str> {
        self.notes
            .iter()
            .find(|item| item.command == command)
            .map(|item| item.note.as_str())
    }

    pub fn set(&mut self, command: &str, note: &str) -> Result<(), String> {
        let updated = Note {
            command: command.to_owned(),
            note: note.to_owned(),
            updated_at: now(),
        };
        upsert_note(&mut self.local_notes, updated.clone());
        upsert_note(&mut self.notes, updated);
        self.save()
    }

    pub fn remove(&mut self, command: &str) -> Result<(), String> {
        remove_note(&mut self.local_notes, command);
        remove_note(&mut self.notes, command);
        self.save()
    }

    pub fn save(&self) -> Result<(), String> {
        let Some(path) = &self.path else {
            return Err("备注路径未设置。".to_owned());
        };
        save_to(path, &self.local_notes)
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.notes.len()
    }

    #[cfg(test)]
    pub fn path_for_test(&self) -> Option<&Path> {
        self.path.as_deref()
    }
}

fn merge_file(notes: &mut Vec<Note>, path: &Path) -> Result<(), String> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("读取备注失败: {error}")),
    };

    for (line_number, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let note: Note = serde_json::from_str(line).map_err(|error| {
            format!(
                "备注文件 {} 第 {} 行解析失败: {error}",
                path.display(),
                line_number + 1
            )
        })?;
        upsert_note(notes, note);
    }
    Ok(())
}

fn merge_notes(notes: &mut Vec<Note>, incoming: &[Note]) {
    for note in incoming {
        upsert_note(notes, note.clone());
    }
}

fn upsert_note(notes: &mut Vec<Note>, note: Note) {
    match notes.iter().position(|item| item.command == note.command) {
        Some(index) if notes[index].updated_at <= note.updated_at => notes[index] = note,
        Some(_) => {}
        None => notes.push(note),
    }
}

fn remove_note(notes: &mut Vec<Note>, command: &str) {
    notes.retain(|item| item.command != command);
}

fn now() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|value| value.as_secs().to_string())
        .unwrap_or_else(|_| "0".to_owned())
}

fn save_to(path: &Path, notes: &[Note]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| format!("创建数据目录失败: {error}"))?;
    }

    let temp_path = path.with_extension("jsonl.tmp");
    {
        let file = fs::File::create(&temp_path)
            .map_err(|error| format!("创建临时备注文件失败: {}: {error}", temp_path.display()))?;
        let mut writer = BufWriter::new(file);
        for note in notes {
            serde_json::to_writer(&mut writer, note)
                .map_err(|error| format!("序列化备注失败: {error}"))?;
            writeln!(writer).map_err(|error| format!("写入备注失败: {error}"))?;
        }
        writer
            .flush()
            .map_err(|error| format!("写入备注失败: {error}"))?;
    }

    fs::rename(&temp_path, path).map_err(|error| {
        let _ = fs::remove_file(&temp_path);
        format!("保存备注失败: {error}")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("kc-notes-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn loads_updates_and_safely_serializes() {
        let dir = temp_dir("roundtrip");
        let path = dir.join("com.notes.jsonl");
        let mut store = NoteStore::load_from(&path).unwrap();
        store.set("cargo test", "运行测试").unwrap();
        store.set("echo \"中文 \\ 路径\"", "包含特殊字符").unwrap();
        store.set("cargo test", "更新后的备注").unwrap();

        let reloaded = NoteStore::load_from(&path).unwrap();
        assert_eq!(reloaded.len(), 2);
        assert_eq!(reloaded.get("cargo test"), Some("更新后的备注"));
        assert_eq!(reloaded.get("echo \"中文 \\ 路径\""), Some("包含特殊字符"));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn last_duplicate_wins() {
        let dir = temp_dir("duplicate");
        let path = dir.join("com.notes.jsonl");
        fs::write(
            &path,
            "{\"command\":\"ls\",\"note\":\"old\",\"updated_at\":\"1\"}\n{\"command\":\"ls\",\"note\":\"new\",\"updated_at\":\"2\"}\n",
        )
        .unwrap();
        let store = NoteStore::load_from(&path).unwrap();
        assert_eq!(store.get("ls"), Some("new"));
        assert_eq!(store.len(), 1);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn merges_multiple_note_files_by_updated_at() {
        let dir = temp_dir("merge");
        let com = dir.join("com.notes.jsonl");
        let phone = dir.join("phone.notes.jsonl");
        fs::write(
            &com,
            "{\"command\":\"ls\",\"note\":\"com old\",\"updated_at\":\"1\"}\n{\"command\":\"dir\",\"note\":\"com only\",\"updated_at\":\"1\"}\n",
        )
        .unwrap();
        fs::write(
            &phone,
            "{\"command\":\"ls\",\"note\":\"phone new\",\"updated_at\":\"2\"}\n{\"command\":\"pwd\",\"note\":\"phone only\",\"updated_at\":\"1\"}\n",
        )
        .unwrap();

        let store =
            NoteStore::load_from_paths(&[com, phone], &dir.join("com.notes.jsonl")).unwrap();
        assert_eq!(store.get("ls"), Some("phone new"));
        assert_eq!(store.get("dir"), Some("com only"));
        assert_eq!(store.get("pwd"), Some("phone only"));
        assert_eq!(store.len(), 3);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn removes_a_note_and_persists_the_deletion() {
        let dir = temp_dir("remove");
        let path = dir.join("com.notes.jsonl");
        let mut store = NoteStore::load_from(&path).unwrap();
        store.set("cargo test", "运行测试").unwrap();
        store.set("cargo build", "构建").unwrap();

        store.remove("cargo test").unwrap();
        assert_eq!(store.get("cargo test"), None);
        assert_eq!(store.get("cargo build"), Some("构建"));
        assert_eq!(store.len(), 1);

        let reloaded = NoteStore::load_from(&path).unwrap();
        assert_eq!(reloaded.len(), 1);
        assert_eq!(reloaded.get("cargo test"), None);
        assert_eq!(reloaded.get("cargo build"), Some("构建"));
        assert!(!fs::read_to_string(&path).unwrap().contains("cargo test"));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn removing_a_missing_note_succeeds() {
        let dir = temp_dir("remove-missing");
        let path = dir.join("com.notes.jsonl");
        let mut store = NoteStore::load_from(&path).unwrap();

        store.remove("never noted").unwrap();
        assert_eq!(store.len(), 0);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn writes_only_current_host_notes() {
        let dir = temp_dir("local-write");
        let com = dir.join("com.notes.jsonl");
        let phone = dir.join("phone.notes.jsonl");
        fs::write(
            &phone,
            "{\"command\":\"pwd\",\"note\":\"phone only\",\"updated_at\":\"1\"}\n",
        )
        .unwrap();

        let mut store = NoteStore::load_from_paths(&[phone, com.clone()], &com).unwrap();
        store.set("cargo test", "local note").unwrap();

        let local = fs::read_to_string(&com).unwrap();
        assert!(local.contains("cargo test"));
        assert!(!local.contains("pwd"));
        fs::remove_dir_all(dir).unwrap();
    }
}
