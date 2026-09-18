use regex::Regex;
use std::collections::HashSet;
use std::process::{Command, Stdio};

/// Atuin's `history list` defaults to oldest first. Ask for newest first so
/// truncation and de-duplication keep the most recent occurrence of a command.
fn history_args() -> [&'static str; 6] {
    [
        "history",
        "list",
        "--cmd-only",
        "--print0",
        "--reverse",
        "false",
    ]
}

/// Atuin prints one NUL-terminated command per entry with `--print0`.
///
/// Duplicate commands are kept at their first position. Since `load_history`
/// requests newest-first output, that position is the most recent occurrence.
pub fn parse_history(raw: &[u8]) -> Vec<String> {
    let mut seen = HashSet::new();
    raw.split(|byte| *byte == 0)
        .filter_map(|item| std::str::from_utf8(item).ok())
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .filter(|item| seen.insert((*item).to_owned()))
        .map(str::to_owned)
        .collect()
}

pub fn load_history(limit: u32, filters: &[Regex]) -> Result<Vec<String>, String> {
    let output = Command::new("atuin")
        .args(history_args())
        .stdin(Stdio::null())
        .output()
        .map_err(|error| format!("无法启动 Atuin: {error}"))?;

    if !output.status.success() {
        let message = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(if message.is_empty() {
            "Atuin 返回错误。".to_owned()
        } else {
            format!("Atuin 返回错误: {message}")
        });
    }

    let mut history = parse_history(&output.stdout);
    filter_history(&mut history, filters);
    history.truncate(limit as usize);
    Ok(history)
}

fn filter_history(history: &mut Vec<String>, filters: &[Regex]) {
    history.retain(|command| !filters.iter().any(|regex| regex.is_match(command)));
}

fn escape_regex(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '\\' | '.' | '+' | '*' | '?' | '(' | ')' | '|' | '[' | ']' | '{' | '}' | '^' | '$' => {
                escaped.push('\\');
                escaped.push(character);
            }
            // Avoid terminating Atuin's r/.../ query at a slash in the command.
            '/' => escaped.push_str("\\x2f"),
            _ => escaped.push(character),
        }
    }
    escaped
}

fn delete_query(command: &str) -> String {
    // `atuin history list --cmd-only` trims command strings before printing.
    // Match the displayed command while tolerating stored leading/trailing whitespace.
    format!("r/^\\s*{}\\s*$/", escape_regex(command))
}

fn search_args(command: &str) -> Vec<String> {
    vec![
        "search".to_owned(),
        "--search-mode".to_owned(),
        "fulltext".to_owned(),
        "--cmd-only".to_owned(),
        "--print0".to_owned(),
        "--filter-mode".to_owned(),
        "global".to_owned(),
        "--".to_owned(),
        delete_query(command),
    ]
}

fn matching_commands(command: &str) -> Result<Vec<String>, String> {
    let output = Command::new("atuin")
        .args(search_args(command))
        .stdin(Stdio::null())
        .output()
        .map_err(|error| format!("无法启动 Atuin: {error}"))?;

    if !output.status.success() {
        // Atuin exits with 1 when nothing matches.
        if output.stdout.is_empty() {
            return Ok(Vec::new());
        }
        let message = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(if message.is_empty() {
            "Atuin 搜索历史失败。".to_owned()
        } else {
            format!("Atuin 搜索历史失败: {message}")
        });
    }

    Ok(parse_history(&output.stdout))
}

fn delete_args(command: &str) -> Vec<String> {
    vec![
        "search".to_owned(),
        "--delete".to_owned(),
        "--search-mode".to_owned(),
        "fulltext".to_owned(),
        "--filter-mode".to_owned(),
        "global".to_owned(),
        "--".to_owned(),
        delete_query(command),
    ]
}

pub fn delete_history(command: &str) -> Result<(), String> {
    let matches = matching_commands(command)?;
    if matches.is_empty() {
        return Err(format!("Atuin 中未找到与“{command}”匹配的历史记录。"));
    }

    let output = Command::new("atuin")
        .args(delete_args(command))
        .stdin(Stdio::null())
        .output()
        .map_err(|error| format!("无法启动 Atuin: {error}"))?;

    if !output.status.success() {
        let message = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(if message.is_empty() {
            "Atuin 删除历史失败。".to_owned()
        } else {
            format!("Atuin 删除历史失败: {message}")
        });
    }

    let remaining = matching_commands(command)?;
    if remaining.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "Atuin 仍有 {} 条与“{command}”匹配的历史记录。",
            remaining.len()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_history_newest_first() {
        assert_eq!(
            history_args(),
            [
                "history",
                "list",
                "--cmd-only",
                "--print0",
                "--reverse",
                "false"
            ]
        );
    }

    #[test]
    fn parses_and_deduplicates_history() {
        let raw = b"cargo test\0git status\0cargo test\0npm run dev\0\0";
        assert_eq!(
            parse_history(raw),
            vec!["cargo test", "git status", "npm run dev"]
        );
    }

    #[test]
    fn keeps_the_newest_duplicate_first() {
        let raw = b"latest dir\0older dir\0latest dir\0old command\0";
        assert_eq!(
            parse_history(raw),
            vec!["latest dir", "older dir", "old command"]
        );
    }

    #[test]
    fn trims_and_skips_empty_entries() {
        assert!(parse_history(b"  \0\0").is_empty());
    }

    #[test]
    fn filters_commands_matching_configured_regexes() {
        let mut history = vec![
            "git status".to_owned(),
            "cargo test".to_owned(),
            "cargo test --all".to_owned(),
            "ls".to_owned(),
        ];
        let filters = vec![Regex::new("^cargo test$").unwrap()];
        filter_history(&mut history, &filters);
        assert_eq!(history, vec!["git status", "cargo test --all", "ls"]);
    }

    #[test]
    fn unanchored_regex_filters_substrings() {
        let mut history = vec!["git status".to_owned(), "echo secret".to_owned()];
        let filters = vec![Regex::new("secret").unwrap()];
        filter_history(&mut history, &filters);
        assert_eq!(history, vec!["git status"]);
    }

    #[test]
    fn empty_filter_list_keeps_everything() {
        let mut history = vec!["dir".to_owned()];
        filter_history(&mut history, &[]);
        assert_eq!(history, vec!["dir"]);
    }

    #[test]
    fn escapes_regex_metacharacters_for_exact_delete() {
        assert_eq!(
            delete_query("echo [$HOME] /tmp"),
            "r/^\\s*echo \\[\\$HOME\\] \\x2ftmp\\s*$/".to_owned()
        );
        assert_eq!(delete_query("cd .."), "r/^\\s*cd \\.\\.\\s*$/".to_owned());
    }

    #[test]
    fn delete_args_use_exact_regex_search() {
        assert_eq!(
            delete_args("dir"),
            vec![
                "search".to_owned(),
                "--delete".to_owned(),
                "--search-mode".to_owned(),
                "fulltext".to_owned(),
                "--filter-mode".to_owned(),
                "global".to_owned(),
                "--".to_owned(),
                "r/^\\s*dir\\s*$/".to_owned(),
            ]
        );
    }

    #[test]
    fn search_args_use_the_same_exact_regex_as_delete() {
        assert_eq!(
            search_args("cd .."),
            vec![
                "search".to_owned(),
                "--search-mode".to_owned(),
                "fulltext".to_owned(),
                "--cmd-only".to_owned(),
                "--print0".to_owned(),
                "--filter-mode".to_owned(),
                "global".to_owned(),
                "--".to_owned(),
                "r/^\\s*cd \\.\\.\\s*$/".to_owned(),
            ]
        );
    }
}

#[cfg(all(test, unix))]
mod unix_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn invokes_atuin_and_reads_nul_output() {
        let dir = std::env::temp_dir().join(format!("kc-fake-atuin-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let fake = dir.join("atuin");
        fs::write(&fake, "#!/bin/sh\nprintf 'one\\0two\\0'").unwrap();
        let mut permissions = fs::metadata(&fake).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&fake, permissions).unwrap();

        let old_path = std::env::var_os("PATH");
        let mut paths = vec![dir.clone()];
        if let Some(old_path) = &old_path {
            paths.extend(std::env::split_paths(old_path));
        }
        std::env::set_var("PATH", std::env::join_paths(paths).unwrap());
        let result = load_history(10, &[]).unwrap();
        match old_path {
            Some(value) => std::env::set_var("PATH", value),
            None => std::env::remove_var("PATH"),
        }
        fs::remove_dir_all(&dir).unwrap();
        assert_eq!(result, vec!["one", "two"]);
    }
}
