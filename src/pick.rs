use crate::app::Config;
use crate::history;
use crate::notes::NoteStore;
use crate::tui::{PickAction, PickResult};
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Clone, Copy, PartialEq, Eq)]
enum OutputFormat {
    Json,
    Nul,
}

pub fn main(args: &[String]) -> ExitCode {
    let mut query = String::new();
    let mut query_env: Option<String> = None;
    let mut result_file: Option<PathBuf> = None;
    let mut result_file_env: Option<String> = None;
    let mut format = OutputFormat::Json;
    let mut index = 0;

    while index < args.len() {
        match args[index].as_str() {
            "--query" => {
                index += 1;
                if index < args.len() {
                    query = args[index].clone();
                }
            }
            "--query-env" => {
                index += 1;
                if index < args.len() {
                    query_env = Some(args[index].clone());
                }
            }
            "--result-file" => {
                index += 1;
                if index < args.len() {
                    result_file = Some(PathBuf::from(&args[index]));
                }
            }
            "--result-file-env" => {
                index += 1;
                if index < args.len() {
                    result_file_env = Some(args[index].clone());
                }
            }
            "--format" => {
                index += 1;
                match args.get(index).map(String::as_str) {
                    Some("json") => format = OutputFormat::Json,
                    Some("nul") => format = OutputFormat::Nul,
                    _ => {
                        eprintln!("--format 只支持 json 或 nul。");
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
        .unwrap_or(query);
    let result_file = result_file.or_else(|| {
        result_file_env
            .and_then(|name| std::env::var(name).ok())
            .map(PathBuf::from)
    });

    let result = match run_pick(&query) {
        Ok(result) => result,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
    };

    let payload = encode(&result, format);
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

fn run_pick(query: &str) -> Result<PickResult, String> {
    let config = Config::load()?;
    let history = history::load(
        &crate::data_paths::history_db(),
        config.history_limit,
        &config.filters,
    )?;
    let mut notes = NoteStore::load()?;
    crate::tui::run(query, &history, &mut notes)
}

fn encode(result: &PickResult, format: OutputFormat) -> String {
    match format {
        OutputFormat::Json => {
            let mut json = serde_json::to_string(result).unwrap_or_default();
            json.push('\n');
            json
        }
        OutputFormat::Nul => {
            let action = match result.action {
                PickAction::Insert => "insert",
                PickAction::Execute => "execute",
                PickAction::Cancel => "cancel",
            };
            format!("{action}\0{}\0", result.command.as_deref().unwrap_or(""))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nul_encoding_preserves_special_characters() {
        let result = PickResult {
            action: PickAction::Insert,
            command: Some("echo \"中文\" \\ path\nnext".to_owned()),
        };
        let encoded = encode(&result, OutputFormat::Nul);
        let mut parts = encoded.split('\0');
        assert_eq!(parts.next(), Some("insert"));
        assert_eq!(parts.next(), Some("echo \"中文\" \\ path\nnext"));
    }

    #[test]
    fn json_encoding_has_action() {
        let result = PickResult {
            action: PickAction::Cancel,
            command: None,
        };
        assert_eq!(
            encode(&result, OutputFormat::Json),
            "{\"action\":\"cancel\"}\n"
        );
    }
}
