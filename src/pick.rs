use crate::tui::PickResult;
use std::path::PathBuf;
use std::process::ExitCode;

/// shell 集成入口：查询串与结果路径都从环境变量读，避免命令行转义问题。
pub fn main(args: &[String]) -> ExitCode {
    let mut query_env: Option<String> = None;
    let mut result_file_env: Option<String> = None;
    let mut index = 0;

    while index < args.len() {
        match args[index].as_str() {
            "--query-env" => {
                index += 1;
                match args.get(index) {
                    Some(name) => query_env = Some(name.clone()),
                    None => {
                        eprintln!("--query-env 缺少变量名。");
                        return ExitCode::from(2);
                    }
                }
            }
            "--result-file-env" => {
                index += 1;
                match args.get(index) {
                    Some(name) => result_file_env = Some(name.clone()),
                    None => {
                        eprintln!("--result-file-env 缺少变量名。");
                        return ExitCode::from(2);
                    }
                }
            }
            _ => {
                eprintln!("kc pick 收到未知参数。");
                return ExitCode::from(2);
            }
        }
        index += 1;
    }

    let query = query_env
        .and_then(|name| std::env::var(name).ok())
        .unwrap_or_default();
    let result_file = result_file_env
        .and_then(|name| std::env::var(name).ok())
        .map(PathBuf::from);

    let result = match crate::app::run_main(&query) {
        Ok(result) => result,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
    };

    let payload = encode(&result);
    if let Some(path) = result_file {
        if let Err(error) = std::fs::write(path, payload) {
            eprintln!("写入结果失败: {error}");
            return ExitCode::FAILURE;
        }
    } else {
        print!("{payload}");
    }
    ExitCode::SUCCESS
}

fn encode(result: &PickResult) -> String {
    let mut json = serde_json::to_string(result).expect("PickResult 序列化不应失败");
    json.push('\n');
    json
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::PickAction;

    #[test]
    fn json_encoding_has_action() {
        let result = PickResult {
            action: PickAction::Cancel,
            command: None,
        };
        assert_eq!(encode(&result), "{\"action\":\"cancel\"}\n");
    }

    #[test]
    fn json_encoding_keeps_multiline_commands() {
        let result = PickResult {
            action: PickAction::Insert,
            command: Some("echo \"中文\" \\ path\nnext".to_owned()),
        };
        let value: serde_json::Value = serde_json::from_str(&encode(&result)).unwrap();
        assert_eq!(
            value.get("command").and_then(serde_json::Value::as_str),
            Some("echo \"中文\" \\ path\nnext")
        );
    }

    #[test]
    fn rejects_unknown_arguments_and_missing_values() {
        assert_eq!(main(&["--query-env".to_owned()]), ExitCode::from(2));
        assert_eq!(main(&["--result-file-env".to_owned()]), ExitCode::from(2));
        assert_eq!(
            main(&["--query".to_owned(), "dir".to_owned()]),
            ExitCode::from(2)
        );
        assert_eq!(
            main(&["--format".to_owned(), "json".to_owned()]),
            ExitCode::from(2)
        );
    }
}
