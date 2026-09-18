use crate::app::notes_path;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
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
    path: Option<PathBuf>,
}

impl NoteStore {
    pub fn load() -> Result<Self, String> {
        let path = notes_path();
        Self::load_from(&path)
    }

    pub fn load_from(path: &Path) -> Result<Self, String> {
        if !path.exists() {
            return Ok(Self {
                notes: Vec::new(),
                path: Some(path.to_owned()),
            });
        }

        let text = fs::read_to_string(path).map_err(|error| format!("读取备注失败: {error}"))?;
        let mut notes = Vec::new();
        let mut seen = HashMap::new();
        for (line_number, line) in text.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let note: Note = serde_json::from_str(line).map_err(|error| {
                format!("备注文件第 {} 行解析失败: {error}", line_number + 1)
            })?;
            match seen.get(&note.command) {
                Some(index) => notes[*index] = note,
                None => {
                    seen.insert(note.command.clone(), notes.len());
                    notes.push(note);
                }
            }
        }

        Ok(Self {
            notes,
            path: Some(path.to_owned()),
        })
    }

    pub fn get(&self, command: &str) -> Option<&str> {
        self.notes
            .iter()
            .find(|item| item.command == command)
            .map(|item| item.note.as_str())
    }

    pub fn set(&mut self, command: &str, note: &str) -> Result<(), String> {
        let now = now();
        if let Some(existing) = self
            .notes
            .iter_mut()
            .find(|item| item.command == command)
        {
            existing.note = note.to_owned();
            existing.updated_at = now;
        } else {
            self.notes.push(Note {
                command: command.to_owned(),
                note: note.to_owned(),
                updated_at: now,
            });
        }
        self.save()
    }

    pub fn save(&self) -> Result<(), String> {
        let Some(path) = &self.path else {
            return Err("备注路径未设置。".to_owned());
        };
        save_to(path, &self.notes)
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.notes.len()
    }
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
        let file = fs::File::create(&temp_path).map_err(|error| {
            format!("创建临时备注文件失败: {}: {error}", temp_path.display())
        })?;
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

    fn temp_file(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(name);
        let _ = fs::remove_file(&path);
        path
    }

    #[test]
    fn loads_updates_and_safely_serializes() {
        let path = temp_file("kc-notes-roundtrip.jsonl");
        let mut store = NoteStore::load_from(&path).unwrap();
        store.set("cargo test", "运行测试").unwrap();
        store.set("echo \"中文 \\ 路径\"", "包含特殊字符").unwrap();
        store.set("cargo test", "更新后的备注").unwrap();

        let reloaded = NoteStore::load_from(&path).unwrap();
        assert_eq!(reloaded.len(), 2);
        assert_eq!(reloaded.get("cargo test"), Some("更新后的备注"));
        assert_eq!(reloaded.get("echo \"中文 \\ 路径\""), Some("包含特殊字符"));
        fs::remove_file(&path).unwrap();
    }

    #[test]
    fn last_duplicate_wins() {
        let path = temp_file("kc-notes-duplicate.jsonl");
        fs::write(
            &path,
            "{\"command\":\"ls\",\"note\":\"old\",\"updated_at\":\"1\"}\n{\"command\":\"ls\",\"note\":\"new\",\"updated_at\":\"2\"}\n",
        )
        .unwrap();
        let store = NoteStore::load_from(&path).unwrap();
        assert_eq!(store.get("ls"), Some("new"));
        assert_eq!(store.len(), 1);
        fs::remove_file(&path).unwrap();
    }
}
