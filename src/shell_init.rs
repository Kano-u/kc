use crate::args;
use crate::data_paths::preview_cache;
use std::path::Path;
use std::process::ExitCode;

const USAGE: &str = "kc init --shell powershell";
const ALLOWED: &[&str] = &["--shell"];

/// 缓存文件的绝对路径在运行时替换进来：脚本本体是编译期常量，必须保持纯 ASCII。
const PREVIEW_PATH: &str = "@KC_PREVIEW_PATH@";
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
    match values.get("--shell").map(String::as_str) {
        Some("powershell") => match rendered() {
            Ok(script) => {
                print!("{script}");
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("kc init: {error}");
                ExitCode::FAILURE
            }
        },
        _ => {
            eprintln!("只支持 --shell powershell。\n用法: {USAGE}");
            ExitCode::from(2)
        }
    }
}

fn rendered() -> Result<String, String> {
    let uri = format!(
        "data:text/plain;charset=utf-8;base64,{}",
        base64(PREDICTOR.as_bytes())
    );
    Ok(script(&preview_cache()?, &uri))
}

/// 路径写进 PowerShell 单引号字符串：反斜杠不被解释，路径里的单引号写成两个。
fn script(path: &Path, uri: &str) -> String {
    let literal = path.to_string_lossy().replace('\'', "''");
    let script = POWERSHELL.replace(PREVIEW_PATH, &literal);
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

/// 这份脚本必须保持纯 ASCII：它经 `kc init --shell powershell | Invoke-Expression`
/// 进入 PowerShell，而管道的解码用的是控制台代码页。非 ASCII 字节在
/// 代码页不是 UTF-8 的终端（如 936）会被解坏，脚本随即语法错误。
const POWERSHELL: &str = r#"# Keep this block as the last statement of the profile.
# It defines global:prompt, so any prompt defined after it would win
# and commands would silently stop reaching the history database.

$global:KcPreviewPath = '@KC_PREVIEW_PATH@'

# The predictor is compiled C#, not a PowerShell class, and the source travels as a
# base64 data URI so this script stays pure ASCII. Two PSReadLine facts force this:
# it runs the predictor on a thread-pool thread where no runspace exists, so a
# PowerShell class cannot execute there at all; and it allows the predictor only
# 20 ms, which interpreted script cannot meet. Compiling costs ~450 ms, so the
# assembly is cached next to the cache file and keyed by the PowerShell version.
$KcPredictorSource = [System.Text.Encoding]::UTF8.GetString(
    [System.Convert]::FromBase64String(('@KC_PREDICTOR_URI@' -split ',', 2)[1]))
$KcPreviewDll = Join-Path (Split-Path -Parent $global:KcPreviewPath) `
    "KcPreviewPredictor-$($PSVersionTable.PSVersion).dll"

if (-not (Test-Path -LiteralPath $KcPreviewDll)) {
    # Add-Type refuses to overwrite an existing assembly, so build beside it and move.
    $KcPreviewTemp = "$KcPreviewDll.tmp"
    Remove-Item -LiteralPath $KcPreviewTemp -ErrorAction SilentlyContinue
    Add-Type -TypeDefinition $KcPredictorSource -OutputAssembly $KcPreviewTemp -OutputType Library
    Move-Item -LiteralPath $KcPreviewTemp -Destination $KcPreviewDll -Force
}

# Re-running kc init in the same session would otherwise fail: the type is loaded already.
if (-not ("KcPreviewPredictor" -as [type])) {
    Add-Type -Path $KcPreviewDll
}

$global:KcPreviewPredictor = [KcPreviewPredictor]::new($global:KcPreviewPath)
[System.Management.Automation.Subsystem.SubsystemManager]::RegisterSubsystem(
    [System.Management.Automation.Subsystem.SubsystemKind]::CommandPredictor,
    $global:KcPreviewPredictor
)

# Plugin, not the combined history-and-plugin source: PSReadLine drops plugin suggestions
# that are byte-identical to a history entry, which would hide kc's own candidates.
Set-PSReadLineOption -PredictionSource Plugin
Set-PSReadLineOption -PredictionViewStyle ListView

Remove-Variable KcPredictorSource, KcPreviewDll, KcPreviewTemp -ErrorAction SilentlyContinue

$global:KcLastId = -1

function global:prompt {
    # $? must be captured on the first line; any statement before it overwrites it.
    $ok = $?
    $entry = Get-History -Count 1

    # One Id is recorded once: an empty Enter redraws the prompt without a new entry.
    if ($null -ne $entry -and $entry.Id -ne $global:KcLastId) {
        $global:KcLastId = $entry.Id
        $env:KC_RECORD = if ($ok) { "1" } else { "0" }
        $env:KC_COMMAND = $entry.CommandLine
        kc record --command-env KC_COMMAND
        $env:KC_RECORD = $null
        $env:KC_COMMAND = $null
    }

    "PS $($executionContext.SessionState.Path.CurrentLocation)$('>' * ($nestedPromptLevel + 1)) "
}

function global:Invoke-KcPick {
    $previousOutputEncoding = [System.Console]::OutputEncoding
    $resultFile = New-TemporaryFile

    try {
        [System.Console]::OutputEncoding = [System.Text.Encoding]::UTF8

        $query = $null
        [Microsoft.PowerShell.PSConsoleReadLine]::GetBufferState([ref]$query, [ref]$null)

        $env:KC_QUERY = $query
        $env:KC_RESULT_FILE = $resultFile
        Start-Process -PassThru -NoNewWindow -FilePath kc -ArgumentList @(
            "pick", "--result-file-env", "KC_RESULT_FILE", "--query-env", "KC_QUERY"
        ) | Wait-Process
        $result = (Get-Content -Raw $resultFile -Encoding UTF8 | Out-String).Trim()

        [Microsoft.PowerShell.PSConsoleReadLine]::InvokePrompt()
        if ($result -eq "") {
            return
        }

        $choice = $result | ConvertFrom-Json
        if ($choice.action -eq "cancel") {
            return
        }

        [Microsoft.PowerShell.PSConsoleReadLine]::RevertLine()
        [Microsoft.PowerShell.PSConsoleReadLine]::Insert($choice.command)
        if ($choice.action -eq "execute") {
            [Microsoft.PowerShell.PSConsoleReadLine]::AcceptLine()
        }
    }
    finally {
        [System.Console]::OutputEncoding = $previousOutputEncoding
        $env:KC_QUERY = $null
        $env:KC_RESULT_FILE = $null
        Remove-Item $resultFile -ErrorAction SilentlyContinue
    }
}

Set-PSReadLineKeyHandler -Chord UpArrow -BriefDescription "Runs kc history picker" -ScriptBlock {
    $line = $null
    [Microsoft.PowerShell.PSConsoleReadLine]::GetBufferState([ref]$line, [ref]$null)

    if ($line.Contains("`n")) {
        [Microsoft.PowerShell.PSConsoleReadLine]::PreviousLine()
    }
    elseif ([Microsoft.PowerShell.PSConsoleReadLine]::HasPredictionSelection()) {
        # A kc candidate is selected in the prediction list, so 'UpArrow' navigates that
        # list -- exactly what the PSReadLine default does. Opening the picker here would
        # throw the selection away on the keystroke meant to move within it.
        [Microsoft.PowerShell.PSConsoleReadLine]::PreviousSuggestion()
    }
    else {
        Invoke-KcPick
    }
}
"#;

/// 预测器源码。方法体里只能出现 .NET 调用：它跑在 PSReadLine 的线程池线程上，
/// 那里没有 runspace，cmdlet 与类内方法调用都会失败（而且失败会被静默吞掉）。
/// 类型全部写全限定名，因为这段代码经 `Add-Type` 编译时没有 `using` 命名空间。
const PREDICTOR: &str = r#"public sealed class KcPreviewPredictor : System.Management.Automation.Subsystem.Prediction.ICommandPredictor
{
    private readonly string _path;
    // 顺序即优先级：缓存文件里最新的命令排在最上面，所以按顺序取前 10 条就是最近用过的 10 条。
    // 不能用字典 —— 字典不保证迭代顺序，候选会退化成随机 10 条。
    private readonly System.Collections.Generic.List<System.Management.Automation.Subsystem.Prediction.PredictiveSuggestion> _entries =
        new System.Collections.Generic.List<System.Management.Automation.Subsystem.Prediction.PredictiveSuggestion>();
    private System.DateTime _stamp = System.DateTime.MinValue;
    private readonly System.Collections.Generic.List<System.Management.Automation.Subsystem.Prediction.PredictiveSuggestion> _list;
    private readonly System.Management.Automation.Subsystem.Prediction.SuggestionPackage _package;

    public System.Guid Id { get; } = System.Guid.NewGuid();
    public string Name => "kc";
    public string Description => "kc history preview";
    public System.Collections.Generic.Dictionary<string, string> FunctionsToDefine => null;

    public KcPreviewPredictor(string path)
    {
        _path = path;
        // SuggestionPackage refuses an empty list, so it is built once around a seed
        // entry and every call reuses the list inside it, emptied first.
        var seed = new System.Collections.Generic.List<System.Management.Automation.Subsystem.Prediction.PredictiveSuggestion>();
        seed.Add(new System.Management.Automation.Subsystem.Prediction.PredictiveSuggestion("seed"));
        _package = new System.Management.Automation.Subsystem.Prediction.SuggestionPackage(seed);
        _list = _package.SuggestionEntries;
        _list.Clear();
    }

    private void Reload()
    {
        _entries.Clear();
        _stamp = System.DateTime.MinValue;
        if (!System.IO.File.Exists(_path))
        {
            return;
        }
        _stamp = System.IO.File.GetLastWriteTimeUtc(_path);
        // The notes are UTF-8; the default encoding would mangle them.
        foreach (var line in System.IO.File.ReadAllLines(_path, System.Text.Encoding.UTF8))
        {
            int split = line.IndexOf('\t');
            if (split <= 0)
            {
                continue;
            }
            // The note is display-only and rides in ToolTip; it never reaches the
            // command line. An empty note becomes null, not an empty string.
            string note = line.Substring(split + 1);
            _entries.Add(new System.Management.Automation.Subsystem.Prediction.PredictiveSuggestion(
                line.Substring(0, split), note.Length == 0 ? null : note));
        }
    }

    public System.Management.Automation.Subsystem.Prediction.SuggestionPackage GetSuggestion(
        System.Management.Automation.Subsystem.Prediction.PredictionClient client,
        System.Management.Automation.Subsystem.Prediction.PredictionContext context,
        System.Threading.CancellationToken cancellationToken)
    {
        _list.Clear();

        // One stat per keystroke: the file is only re-read when kc rewrote it.
        if (System.IO.File.Exists(_path))
        {
            System.DateTime current = System.IO.File.GetLastWriteTimeUtc(_path);
            if (current != _stamp)
            {
                Reload();
            }
        }

        string prefix = context.InputAst.Extent.Text;
        if (!string.IsNullOrEmpty(prefix))
        {
            foreach (var entry in _entries)
            {
                if (entry.SuggestionText.StartsWith(prefix, System.StringComparison.OrdinalIgnoreCase))
                {
                    _list.Add(entry);
                    if (_list.Count >= 10)
                    {
                        break;
                    }
                }
            }
        }

        return _package;
    }

    public bool CanAcceptFeedback(
        System.Management.Automation.Subsystem.Prediction.PredictionClient client,
        System.Management.Automation.Subsystem.Prediction.PredictorFeedbackKind kind)
    {
        return false;
    }

    public void OnSuggestionDisplayed(
        System.Management.Automation.Subsystem.Prediction.PredictionClient client,
        uint session, int count)
    {
    }

    public void OnSuggestionAccepted(
        System.Management.Automation.Subsystem.Prediction.PredictionClient client,
        uint session, string acceptedSuggestion)
    {
    }

    public void OnCommandLineAccepted(
        System.Management.Automation.Subsystem.Prediction.PredictionClient client,
        System.Collections.Generic.IReadOnlyList<string> history)
    {
    }

    public void OnCommandLineExecuted(
        System.Management.Automation.Subsystem.Prediction.PredictionClient client,
        string commandLine, bool success)
    {
    }
}
"#;

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
        let output = script(Path::new(r"C:\Users\k\.kc\preview.tsv"), "data:,");
        assert!(output.is_ascii(), "生成脚本含非 ASCII 字节");
    }

    #[test]
    fn writes_the_preview_cache_path_as_a_single_quoted_literal() {
        let path = Path::new(r"C:\Users\k\.kc\preview.tsv");
        let output = script(path, "data:,");
        assert!(output.contains(r"$global:KcPreviewPath = 'C:\Users\k\.kc\preview.tsv'"));
        assert!(!output.contains(PREVIEW_PATH), "占位符没有被替换");
    }

    #[test]
    fn doubles_single_quotes_in_the_preview_path() {
        let output = script(Path::new("/tmp/it's/preview.tsv"), "data:,");
        assert!(output.contains("$global:KcPreviewPath = '/tmp/it''s/preview.tsv'"));
    }

    #[test]
    fn carries_the_predictor_as_base64_rather_than_as_source() {
        let uri = format!(
            "data:text/plain;charset=utf-8;base64,{}",
            base64(PREDICTOR.as_bytes())
        );
        let output = script(Path::new("p"), &uri);
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
    fn the_predictor_only_calls_dotnet_from_the_prediction_path() {
        // 线程池线程上没有 runspace：cmdlet 与类内方法调用都会失败。
        for forbidden in ["Test-Path", "Get-Item", "Get-Content", "Write-Output"] {
            assert!(
                !PREDICTOR.contains(forbidden),
                "预测器不能用 cmdlet: {forbidden}"
            );
        }
        assert!(PREDICTOR.contains("System.IO.File.ReadAllLines"));
        assert!(PREDICTOR.contains("System.IO.File.GetLastWriteTimeUtc"));
        assert!(PREDICTOR.contains("System.Text.Encoding.UTF8"));
    }

    #[test]
    fn the_predictor_filters_by_prefix_and_fills_the_tooltip() {
        assert!(PREDICTOR.contains("context.InputAst.Extent.Text"));
        assert!(PREDICTOR.contains(
            "entry.SuggestionText.StartsWith(prefix, System.StringComparison.OrdinalIgnoreCase)"
        ));
        assert!(PREDICTOR.contains("note.Length == 0 ? null : note"));
        assert!(PREDICTOR.contains("_package.SuggestionEntries"));
        assert!(PREDICTOR.contains("_list.Clear()"));
    }

    #[test]
    fn the_predictor_keeps_the_cache_file_order() {
        // 字典不保证迭代顺序，候选会变成随机 10 条；文件已按最新在上排好，必须按顺序取。
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

    #[test]
    fn compiles_the_predictor_once_and_caches_it_by_powershell_version() {
        assert!(POWERSHELL.contains("KcPreviewPredictor-$($PSVersionTable.PSVersion).dll"));
        assert!(POWERSHELL.contains("if (-not (Test-Path -LiteralPath $KcPreviewDll)) {"));
        assert!(POWERSHELL.contains(
            "Add-Type -TypeDefinition $KcPredictorSource -OutputAssembly $KcPreviewTemp"
        ));
        assert!(POWERSHELL.contains("Add-Type -Path $KcPreviewDll"));
        assert!(POWERSHELL.contains(r#"if (-not ("KcPreviewPredictor" -as [type])) {"#));
    }

    #[test]
    fn registers_the_predictor_with_its_full_type_names() {
        assert!(POWERSHELL.contains("[KcPreviewPredictor]::new($global:KcPreviewPath)"));
        assert!(POWERSHELL.contains(
            "[System.Management.Automation.Subsystem.SubsystemManager]::RegisterSubsystem("
        ));
        assert!(POWERSHELL
            .contains("[System.Management.Automation.Subsystem.SubsystemKind]::CommandPredictor"));
    }

    #[test]
    fn previews_with_the_plugin_source_in_list_view() {
        assert!(POWERSHELL.contains("Set-PSReadLineOption -PredictionSource Plugin"));
        assert!(!POWERSHELL.contains("HistoryAndPlugin"));
        assert!(POWERSHELL.contains("Set-PSReadLineOption -PredictionViewStyle ListView"));
    }

    #[test]
    fn captures_the_status_on_the_first_prompt_line() {
        let prompt = POWERSHELL
            .split_once("function global:prompt {")
            .expect("prompt 函数缺失")
            .1;
        let first = prompt
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty() && !line.starts_with('#'))
            .expect("prompt 函数体为空");
        assert_eq!(first, "$ok = $?");
    }

    #[test]
    fn records_each_history_id_once() {
        assert!(POWERSHELL.contains("Get-History -Count 1"));
        assert!(POWERSHELL.contains("$entry.Id -ne $global:KcLastId"));
        assert!(POWERSHELL.contains("kc record --command-env KC_COMMAND"));
        assert!(POWERSHELL.contains(r#"$env:KC_RECORD = if ($ok) { "1" } else { "0" }"#));
    }

    #[test]
    fn up_arrow_navigates_the_preview_before_opening_the_picker() {
        let handler = POWERSHELL
            .split_once("-Chord UpArrow")
            .expect("UpArrow 处理函数缺失")
            .1;
        let previous = handler
            .find("PreviousSuggestion")
            .expect("没有在预览列表里上移的分支");
        let picker = handler.find("Invoke-KcPick").expect("没有开 kc 的分支");

        // 预览里已有选中项时不能开 kc，否则用户按 ↑ 想上移却被丢进 TUI。
        assert!(
            handler.contains("HasPredictionSelection()"),
            "↑ 必须先问预览有没有选中项"
        );
        assert!(previous < picker, "↑ 必须先处理预览选中，再考虑开 kc");
        assert!(
            handler.contains("PreviousLine()"),
            "多行命令仍要走 PreviousLine"
        );
    }

    #[test]
    fn has_no_bash_integration_left() {
        assert!(!POWERSHELL.contains("bash"));
        assert!(!POWERSHELL.contains("READLINE_LINE"));
        assert!(!POWERSHELL.contains("atuin"));
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
