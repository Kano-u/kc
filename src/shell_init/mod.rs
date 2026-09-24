use crate::args;
use crate::data_paths::{config_dir, history_db};
use std::path::Path;
use std::process::ExitCode;

mod powershell;
mod predictor;
mod zsh;

use powershell::POWERSHELL;
use predictor::PREDICTOR;
use zsh::ZSH;

const USAGE: &str = "kc init --shell powershell|zsh";
const ALLOWED: &[&str] = &["--shell"];

/// 历史数据库与备注目录以 base64 形式内嵌：脚本本体必须保持纯 ASCII，
/// 而 KC_CONFIG_DIR 可能含非 ASCII（如 `D:\0\32_文档\kc_data`）。
const HISTORY_PATH: &str = "@KC_HISTORY_DB@";
const NOTES_DIR: &str = "@KC_NOTES_DIR@";
/// C# 源码以 base64 形式内嵌，同样是 ASCII。
const PREDICTOR_URI: &str = "@KC_PREDICTOR_URI@";

pub fn main(args: &[String]) -> ExitCode {
    let values = match args::parse(args, ALLOWED) {
        Ok(values) => values,
        Err(error) => {
            eprintln!("{error}\n用法: {USAGE}");
            return ExitCode::from(2);
        }
    };
    let script = match values.get("--shell").map(String::as_str) {
        Some("powershell") => rendered_powershell(),
        Some("zsh") => Ok(rendered_zsh().to_owned()),
        _ => {
            eprintln!("只支持 --shell powershell 或 zsh。\n用法: {USAGE}");
            return ExitCode::from(2);
        }
    };
    match script {
        Ok(script) => {
            print!("{script}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("kc init: {error}");
            ExitCode::FAILURE
        }
    }
}

fn rendered_powershell() -> Result<String, String> {
    let uri = format!(
        "data:text/plain;charset=utf-8;base64,{}",
        base64(PREDICTOR.as_bytes())
    );
    Ok(script(&history_db()?, &config_dir()?, &uri))
}

/// zsh 版本不碰预测器，也就没有 base64 与路径占位符要替换：整份脚本是一个常量。
fn rendered_zsh() -> &'static str {
    ZSH
}

/// 路径经 base64 内嵌，在脚本里解码。直接写路径会带进非 ASCII 字节，
/// 而脚本要过管道解码（用控制台代码页），非 UTF-8 代码页上会被解坏。
fn script(history_db: &Path, notes_dir: &Path, uri: &str) -> String {
    let script = POWERSHELL
        .replace(HISTORY_PATH, &base64(history_db.to_string_lossy().as_bytes()))
        .replace(NOTES_DIR, &base64(notes_dir.to_string_lossy().as_bytes()));
    script.replace(PREDICTOR_URI, uri)
}

/// 标准 base64（RFC 4648，带 `=` 填充）。预测器源码是内嵌常量，
/// 这里避免为一次编码引入依赖。
fn base64(input: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut output = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let (a, b, c) = (
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        );
        let packed = (u32::from(a) << 16) | (u32::from(b) << 8) | u32::from(c);
        for index in 0..4 {
            if index <= chunk.len() {
                let value = (packed >> (18 - 6 * index)) & 0b0011_1111;
                output.push(char::from(ALPHABET[value as usize]));
            } else {
                output.push('=');
            }
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 把脚本里内嵌的 C# 源码解出来，供测试直接检查。
    fn embedded_predictor(script: &str) -> String {
        let start = script.find("base64,").expect("找不到内嵌源码") + "base64,".len();
        let encoded = script[start..]
            .split('\'')
            .next()
            .expect("内嵌源码没有结束引号");
        decode(encoded)
    }

    fn decode(input: &str) -> String {
        let mut bytes = Vec::new();
        let mut buffer = 0u32;
        let mut bits = 0;
        for byte in input.bytes().filter(|byte| *byte != b'=') {
            let value = match byte {
                b'A'..=b'Z' => byte - b'A',
                b'a'..=b'z' => byte - b'a' + 26,
                b'0'..=b'9' => byte - b'0' + 52,
                b'+' => 62,
                b'/' => 63,
                other => panic!("非法 base64 字符: {}", other as char),
            };
            buffer = (buffer << 6) | u32::from(value);
            bits += 6;
            if bits >= 8 {
                bits -= 8;
                bytes.push((buffer >> bits) as u8);
            }
        }
        String::from_utf8(bytes).expect("内嵌源码不是 UTF-8")
    }

    #[test]
    fn stays_ascii_so_any_console_codepage_can_decode_it() {
        assert!(
            POWERSHELL.is_ascii(),
            "脚本含非 ASCII 字节，管道解码会破坏它"
        );
        assert!(PREDICTOR_URI.is_ascii());
        let output = script(
            Path::new(r"C:\Users\k\.kc\history.db"),
            Path::new(r"C:\Users\k\kc"),
            "data:,",
        );
        assert!(output.is_ascii(), "生成脚本含非 ASCII 字节");
    }

    #[test]
    fn writes_the_history_and_notes_paths_as_decoded_base64() {
        let output = script(
            Path::new(r"C:\Users\k\.kc\history.db"),
            Path::new(r"C:\Users\k\kc"),
            "data:,",
        );
        assert!(output.contains("$global:KcHistoryDb = $KcUtf8.GetString("));
        assert!(output.contains("$global:KcNotesDir = $KcUtf8.GetString("));
        assert!(!output.contains(HISTORY_PATH), "占位符没有被替换");
        assert!(!output.contains(NOTES_DIR), "占位符没有被替换");
        // 路径本身不能出现在脚本里：非 ASCII 路径会破坏纯 ASCII 约束。
        assert!(!output.contains(r"C:\Users\k\.kc\history.db"));
    }

    #[test]
    fn encodes_a_non_ascii_notes_directory_without_breaking_ascii() {
        // KC_CONFIG_DIR 可能含中文（如 D:\0\32_文档\kc_data），
        // 直接写进脚本会引入非 ASCII 字节。
        let non_ascii = Path::new("D:\\0\\32_文档\\kc_data");
        let output = script(Path::new("h"), non_ascii, "data:,");
        assert!(output.is_ascii(), "含非 ASCII 路径时脚本仍必须是 ASCII");
        assert!(!output.contains("文档"));
    }

    #[test]
    fn carries_the_predictor_as_base64_rather_than_as_source() {
        let uri = format!(
            "data:text/plain;charset=utf-8;base64,{}",
            base64(PREDICTOR.as_bytes())
        );
        let output = script(Path::new("p"), Path::new("n"), &uri);
        assert!(!output.contains(PREDICTOR_URI), "占位符没有被替换");
        assert!(
            !output.contains("public sealed class"),
            "源码应当以 base64 形式内嵌"
        );
        assert_eq!(embedded_predictor(&output), PREDICTOR);
    }

    #[test]
    fn base64_matches_the_known_vectors() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foob"), "Zm9vYg==");
        assert_eq!(base64(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn rejects_unknown_shells() {
        assert_eq!(main(&[]), ExitCode::from(2));
        assert_eq!(
            main(&["--shell".to_owned(), "bash".to_owned()]),
            ExitCode::from(2)
        );
        assert_eq!(main(&["powershell".to_owned()]), ExitCode::from(2));
        assert_eq!(main(&["--shell".to_owned()]), ExitCode::from(2));
    }
}
