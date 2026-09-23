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

const USAGE: &str = "kc - Shell 历史记录笔记与搜索

用法:
  kc                      启动 TUI
  kc pick --query-env NAME --result-file-env NAME
                          为 shell 集成启动 TUI；结果写入文件（未给则为 JSON）
  kc record --command-env NAME
                          将命令记录到 NAME 环境变量中
  kc import powershell    将 PowerShell 历史记录导入数据库
  kc init powershell      打印 PowerShell 集成脚本
  kc --help               显示此帮助

环境变量:
  KC_CONFIG_DIR           配置/数据目录，必须设置
  KC_RECORD               命令成功时设为 1，否则为 0
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
