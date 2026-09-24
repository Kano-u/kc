use crate::app::Config;
use crate::args;
use crate::data_paths::history_db;
use crate::history;
use std::process::ExitCode;

const USAGE: &str = "kc record --command-env NAME";
const ALLOWED: &[&str] = &["--command-env"];

/// 由 PowerShell 的 prompt 钩子调用：把刚执行的命令写进历史库。
///
/// 这里永远返回成功，错误只打到 stderr —— 记录历史绝不能干扰用户的提示符。
pub fn main(args: &[String]) -> ExitCode {
    if let Err(error) = record(args) {
        eprintln!("kc record: {error}");
    }
    ExitCode::SUCCESS
}

fn record(args: &[String]) -> Result<(), String> {
    let values = args::parse(args, ALLOWED)?;
    let name = values
        .get("--command-env")
        .ok_or_else(|| format!("缺少 --command-env。\n用法: {USAGE}"))?;
    let Some(command) = std::env::var(name).ok().as_deref().and_then(clean) else {
        return Ok(());
    };

    // 配置坏掉时绝不"不过滤照记"：宁可这条不记，也不能把本该被 filter 挡住的命令写进库。
    let config = Config::load()?;
    history::upsert(&history_db()?, &command, succeeded(), &config)?;

    // 预览直接在读库时发生，不再有派生缓存需要刷新。
    Ok(())
}

/// 去掉首尾空白（含多行命令的行尾换行），保留内嵌换行；全空白视为空命令。
fn clean(raw: &str) -> Option<String> {
    let command = raw.trim();
    (!command.is_empty()).then(|| command.to_owned())
}

/// `KC_RECORD=1` 记为成功，缺失或其它值都记为失败。
fn succeeded() -> bool {
    std::env::var("KC_RECORD").is_ok_and(|value| value == "1")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_missing_unknown_and_extra_arguments() {
        // 参数解析统一由 args 模块负责，这里只确认 record 接住了它的错误。
        assert!(record(&[]).is_err());
        assert!(record(&["--command-env".to_owned()]).is_err());
        assert!(record(&["-c".to_owned(), "KC_COMMAND".to_owned()]).is_err());
        assert!(record(&["--command-env".to_owned(), "".to_owned()]).is_err());
    }

    #[test]
    fn trims_outer_whitespace_and_keeps_inner_newlines() {
        assert_eq!(clean("  dir  "), Some("dir".to_owned()));
        assert_eq!(
            clean("\n  dir |\n  more\n"),
            Some("dir |\n  more".to_owned())
        );
    }

    #[test]
    fn blank_commands_are_not_recorded() {
        assert_eq!(clean(""), None);
        assert_eq!(clean("   \r\n\t "), None);
    }

    #[test]
    fn success_comes_from_the_kc_record_variable() {
        // 必须在同一测试内串行改环境变量，避免和别的测试互相干扰。
        std::env::set_var("KC_RECORD", "1");
        assert!(succeeded());
        std::env::set_var("KC_RECORD", "0");
        assert!(!succeeded());
        std::env::remove_var("KC_RECORD");
        assert!(!succeeded());
    }
}
