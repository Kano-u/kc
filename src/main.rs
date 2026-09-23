mod app;
mod data_paths;
mod history;
mod import;
mod notes;
mod pick;
mod record;
mod shell_init;
mod tui;

use std::process::ExitCode;

const USAGE: &str = "kc - Shell history notes and search

Usage:
  kc                      Start the TUI
  kc pick --query QUERY   Start TUI for shell integration; emit one JSON result
  kc record --command-env NAME
                          Record the command in the NAME environment variable
  kc import powershell    Import the PowerShell history into the database
  kc init powershell      Print the PowerShell integration
  kc --help               Show this help

Environment:
  KC_CONFIG_DIR           Override the kc config/data directory
  KC_RECORD               Set to 1 when the command succeeded, otherwise 0
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None => app::run(None),
        Some("--help") | Some("-h") => {
            print!("{USAGE}");
            ExitCode::SUCCESS
        }
        Some("init") => shell_init::print(args.get(1).map(String::as_str)),
        Some("import") => import::main(&args[1..]),
        Some("pick") => pick::main(&args[1..]),
        Some("record") => record::main(&args[1..]),
        Some(_) => {
            eprintln!("未知命令。使用 kc --help 查看用法。");
            ExitCode::from(2)
        }
    }
}
