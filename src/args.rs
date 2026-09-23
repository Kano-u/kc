use std::collections::HashMap;

/// 所有子命令唯一的参数解析入口：只接受 `--name value` 形式。
///
/// 未知参数、缺少值、同一参数重复出现都算用法错误。各子命令因此不再
/// 各写一套解析循环，语法不会互相漂移。
pub fn parse(args: &[String], allowed: &[&str]) -> Result<HashMap<String, String>, String> {
    let mut values = HashMap::new();
    let mut index = 0;
    while index < args.len() {
        let name = args[index].as_str();
        if !allowed.contains(&name) {
            return Err(format!("未知参数 {name}。"));
        }
        let value = args
            .get(index + 1)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| format!("{name} 缺少值。"))?;
        if values.insert(name.to_owned(), value.clone()).is_some() {
            return Err(format!("{name} 重复出现。"));
        }
        index += 2;
    }
    Ok(values)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALLOWED: &[&str] = &["--query-env", "--result-file-env"];

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn reads_flags_in_any_order() {
        let values = parse(
            &args(&["--result-file-env", "FILE", "--query-env", "QUERY"]),
            ALLOWED,
        )
        .unwrap();
        assert_eq!(values.get("--query-env").map(String::as_str), Some("QUERY"));
        assert_eq!(
            values.get("--result-file-env").map(String::as_str),
            Some("FILE")
        );
    }

    #[test]
    fn absent_flags_are_simply_missing() {
        assert!(parse(&args(&[]), ALLOWED).unwrap().is_empty());
    }

    #[test]
    fn rejects_an_empty_value() {
        let error = parse(&args(&["--query-env", ""]), ALLOWED).unwrap_err();
        assert!(error.contains("缺少值"), "{error}");
    }

    #[test]
    fn rejects_unknown_flags() {
        let error = parse(&args(&["--query", "dir"]), ALLOWED).unwrap_err();
        assert!(error.contains("未知参数"), "{error}");
    }

    #[test]
    fn rejects_a_flag_without_a_value() {
        let error = parse(&args(&["--query-env"]), ALLOWED).unwrap_err();
        assert!(error.contains("缺少值"), "{error}");
    }

    #[test]
    fn rejects_a_repeated_flag() {
        let error = parse(&args(&["--query-env", "A", "--query-env", "B"]), ALLOWED).unwrap_err();
        assert!(error.contains("重复出现"), "{error}");
    }
}
