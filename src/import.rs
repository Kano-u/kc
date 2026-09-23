use crate::app::Config;
use crate::data_paths::history_db;
use crate::history;
use std::path::PathBuf;
use std::process::ExitCode;

/// 把 PowerShell（PSReadLine）已有的历史并入 kc 的历史库。
pub fn main(args: &[String]) -> ExitCode {
    match args {
        [shell] if shell == "powershell" => {}
        _ => {
            eprintln!("用法: kc import powershell");
            return ExitCode::from(2);
        }
    }

    match run() {
        Ok((path, count)) => {
            println!("已从 {} 导入 {count} 条命令。", path.display());
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("kc import: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(PathBuf, usize), String> {
    let path = powershell_history()?;
    let text = std::fs::read_to_string(&path)
        .map_err(|error| format!("读取历史文件 {} 失败: {error}", path.display()))?;

    // 配置读不到时 filter 未知，直接报错，不再按"不过滤"导入。
    let config = Config::load()?;

    let commands = parse(&text, &config);
    let count = history::import(&history_db()?, &commands)?;
    Ok((path, count))
}

/// PSReadLine 把历史存在固定的用户目录，Windows PowerShell 控制台宿主是 ConsoleHost。
fn powershell_history() -> Result<PathBuf, String> {
    let appdata = std::env::var_os("APPDATA")
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "环境变量 APPDATA 未设置，无法定位 PowerShell 历史。".to_owned())?;
    Ok(PathBuf::from(appdata)
        .join("Microsoft")
        .join("Windows")
        .join("PowerShell")
        .join("PSReadLine")
        .join("ConsoleHost_history.txt"))
}

/// PSReadLine 的历史文件一行一条命令；行尾的反引号表示续行，读回时还原成换行符。
/// 空的续行、全空白行都不是命令；命中的 filter 不入库。
/// 去重交给历史表的主键，这里不做。
fn parse(text: &str, config: &Config) -> Vec<String> {
    let mut commands = Vec::new();
    let mut pending = String::new();
    for line in text.lines() {
        if let Some(rest) = line.strip_suffix('`') {
            pending.push_str(rest);
            pending.push('\n');
        } else if pending.is_empty() {
            push(&mut commands, line, config);
        } else {
            pending.push_str(line);
            push(&mut commands, &pending, config);
            pending.clear();
        }
    }
    commands
}

fn push(commands: &mut Vec<String>, command: &str, config: &Config) {
    let command = command.trim();
    if !command.is_empty() && !config.filtered(command) {
        commands.push(command.to_owned());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(filters: &[&str]) -> Config {
        Config {
            history_limit: 5000,
            filters: filters
                .iter()
                .map(|pattern| regex::Regex::new(pattern).unwrap())
                .collect(),
        }
    }

    #[test]
    fn reads_one_command_per_line() {
        let commands = parse("dir\nls\ncargo test\n", &Config::default());
        assert_eq!(commands, vec!["dir", "ls", "cargo test"]);
    }

    #[test]
    fn joins_continuation_lines_and_trims() {
        // 文件里的多行命令把换行写成行尾反引号，命令自身的反引号则写成两个。
        let text = "runtime/python.exe api.py ``\n  -a 0.0.0.0 -p 9881\n";
        assert_eq!(
            parse(text, &Config::default()),
            vec!["runtime/python.exe api.py `\n  -a 0.0.0.0 -p 9881"]
        );
    }

    #[test]
    fn drops_blank_lines_and_a_trailing_continuation() {
        assert_eq!(
            parse("dir\n\n   \nls\n", &Config::default()),
            vec!["dir", "ls"]
        );
        assert_eq!(parse("dir\nls `\n", &Config::default()), vec!["dir"]);
    }

    #[test]
    fn applies_the_filter() {
        let commands = parse("dir\nsecret\nls\n", &config(&["^dir$", "secret"]));
        assert_eq!(commands, vec!["ls"]);
    }

    #[test]
    fn keeps_duplicates_for_the_database_to_collapse() {
        assert_eq!(parse("a\nb\na\n", &Config::default()), vec!["a", "b", "a"]);
    }

    #[test]
    fn accepts_only_powershell() {
        assert_eq!(main(&[]), ExitCode::from(2));
        assert_eq!(main(&["bash".to_owned()]), ExitCode::from(2));
        assert_eq!(
            main(&["powershell".to_owned(), "x".to_owned()]),
            ExitCode::from(2)
        );
    }
}
