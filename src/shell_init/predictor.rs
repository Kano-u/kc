/// 预测器源码。方法体里只能出现 .NET 调用：它跑在 PSReadLine 的线程池线程上，
/// 那里没有 runspace，cmdlet 与类内方法调用都会失败（而且失败会被静默吞掉）。
/// 类型全部写全限定名，因为这段代码经 `Add-Type` 编译时没有 `using` 命名空间。
///
/// 它直接读 `history.db` 与 `*.notes.jsonl`，不再需要 `kc` 预先导出任何缓存：
/// SQLite 走 P/Invoke 到 Windows 自带的 `winsqlite3.dll` —— 这恰好绕开了
/// “线程池线程上没有 runspace”的限制，因为 P/Invoke 不经过 PowerShell。
///
/// 源码放在同目录的 `predictor.cs` 里而不是内嵌字符串：它是完整的一个程序，
/// 提成真实文件才有语法高亮与检查 —— 内嵌时写错只有跑 `kc init` 才暴露。
/// `include_str!` 在编译期读入，所以产物与内嵌时完全一样。
pub(super) const PREDICTOR: &str = include_str!("predictor.cs");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_source_is_free_of_carriage_returns() {
        // include_str! 原样读入字节，不像 raw string 那样被 rustc 归一化。
        // 一旦工作区把 predictor.cs 检出成 CRLF，CR 就会随 base64 进脚本。
        // C# 编译器容忍它，但产物字节会随检出环境变，所以这里直接拦住。
        assert!(
            !PREDICTOR.contains('\r'),
            "predictor.cs 含 CR：检查 .gitattributes 的 eol=lf 是否生效"
        );
        // 不查 ASCII：C# 的注释里有中文，它先经 base64 才进脚本，
        // 所以源码非 ASCII 是正常的。“脚本纯 ASCII”由 mod.rs 的测试保证。
    }

    #[test]
    fn the_predictor_only_calls_dotnet_from_the_prediction_path() {
        // 线程池线程上没有 runspace：cmdlet 与类内方法调用都会失败。
        for forbidden in ["Test-Path", "Get-Item", "Get-Content", "Write-Output"] {
            assert!(
                !PREDICTOR.contains(forbidden),
                "预测器不能用 cmdlet: {forbidden}"
            );
        }
        assert!(PREDICTOR.contains("System.IO.File.ReadAllLines"));
        assert!(PREDICTOR.contains("System.Text.Encoding.UTF8"));
    }

    #[test]
    fn the_predictor_reads_the_database_through_winsqlite3() {
        // 直接读 SQLite 是本次改动的全部要点：线程池线程上没有 runspace，
        // 只有 P/Invoke 能到达数据库，且不引入任何外部依赖。
        assert!(PREDICTOR.contains(r#"DllImport("winsqlite3.dll""#));
        assert!(PREDICTOR.contains("sqlite3_open_v2"));
        assert!(PREDICTOR.contains("SELECT command FROM history ORDER BY at DESC"));
        assert!(PREDICTOR.contains("sqlite3_busy_timeout"));
        assert!(PREDICTOR.contains("sqlite3_finalize"));
        assert!(PREDICTOR.contains("sqlite3_close"));
    }

    #[test]
    fn the_predictor_merges_every_notes_file_by_updated_at() {
        // 与 notes.rs 同规则：同名命令取 updated_at 较新的一条，
        // 且文件名排序使结果与枚举顺序无关。
        assert!(PREDICTOR.contains(r#"GetFiles(_notesDir, "*.notes.jsonl")"#));
        assert!(PREDICTOR.contains("System.Array.Sort(files, System.StringComparer.Ordinal)"));
        assert!(PREDICTOR.contains("System.String.CompareOrdinal(previous, updated) <= 0"));
        assert!(PREDICTOR.contains("System.Text.Json.JsonDocument.Parse"));
    }

    #[test]
    fn the_predictor_reloads_only_when_a_source_file_changes() {
        // 每按一键只比一次修改时间+大小；内容重读很贵（2 万条约 17 ms）。
        assert!(PREDICTOR.contains("string current = Stamp();"));
        assert!(PREDICTOR.contains("if (current != _stamp)"));
        assert!(PREDICTOR.contains("LastWriteTimeUtc.Ticks"));
        assert!(PREDICTOR.contains(".Append(info.Length)"));
    }

    #[test]
    fn the_predictor_skips_multiline_commands() {
        // PSReadLine 自己的历史建议也跳过含换行的命令，行内插不下它们。
        assert!(PREDICTOR.contains("command.IndexOf('\\n') != -1"));
        assert!(PREDICTOR.contains("command.IndexOf('\\r') != -1"));
    }

    #[test]
    fn the_predictor_filters_by_prefix_and_fills_the_tooltip() {
        assert!(PREDICTOR.contains("context.InputAst.Extent.Text"));
        assert!(PREDICTOR.contains(
            "entry.SuggestionText.StartsWith(prefix, System.StringComparison.OrdinalIgnoreCase)"
        ));
        assert!(PREDICTOR.contains("string.IsNullOrEmpty(note) ? null : note"));
        assert!(PREDICTOR.contains("_package.SuggestionEntries"));
        assert!(PREDICTOR.contains("_list.Clear()"));
    }

    #[test]
    fn the_predictor_keeps_the_query_order() {
        // 字典不保证迭代顺序，候选会变成随机 10 条；查询已按最新在上排好，必须按顺序取。
        assert!(
            !PREDICTOR.contains("Dictionary<string, string> _entries"),
            "候选不能用无序容器存"
        );
        assert!(PREDICTOR.contains("foreach (var entry in _entries)"));
    }

    #[test]
    fn the_predictor_seeds_the_package_because_empty_ones_are_rejected() {
        assert!(PREDICTOR.contains(
            r#"new System.Management.Automation.Subsystem.Prediction.PredictiveSuggestion("seed")"#
        ));
    }
}
