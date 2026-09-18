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

pub fn load_history(limit: u32) -> Result<Vec<String>, String> {
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
    history.truncate(limit as usize);
    Ok(history)
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
        let result = load_history(10).unwrap();
        match old_path {
            Some(value) => std::env::set_var("PATH", value),
            None => std::env::remove_var("PATH"),
        }
        fs::remove_dir_all(&dir).unwrap();
        assert_eq!(result, vec!["one", "two"]);
    }
}
