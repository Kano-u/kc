use crate::args;
use crate::data_paths::preview_cache;
use std::path::Path;
use std::process::ExitCode;

const USAGE: &str = "kc init --shell powershell";
const ALLOWED: &[&str] = &["--shell"];

/// 缓存文件的绝对路径在运行时替换进来：脚本本体是编译期常量，必须保持纯 ASCII。
const PREVIEW_PATH: &str = "@KC_PREVIEW_PATH@";

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
    Ok(script(&preview_cache()?))
}

/// 路径写进 PowerShell 单引号字符串：反斜杠不被解释，路径里的单引号写成两个。
fn script(path: &Path) -> String {
    let literal = path.to_string_lossy().replace('\'', "''");
    POWERSHELL.replace(PREVIEW_PATH, &literal)
}

/// 这份脚本必须保持纯 ASCII：它经 `kc init --shell powershell | Invoke-Expression`
/// 进入 PowerShell，而管道的解码用的是控制台代码页。非 ASCII 字节在
/// 代码页不是 UTF-8 的终端（如 936）会被解坏，脚本随即语法错误。
const POWERSHELL: &str = r#"# Keep this block as the last statement of the profile.
# It defines global:prompt, so any prompt defined after it would win
# and commands would silently stop reaching the history database.

$global:KcPreviewPath = '@KC_PREVIEW_PATH@'
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

    if (!$line.Contains("`n")) {
        Invoke-KcPick
    } else {
        [Microsoft.PowerShell.PSConsoleReadLine]::PreviousLine()
    }
}

# The command predictor reads the cache file kc exports and runs on every keystroke,
# so it must never fork kc. Every type below is spelled with its full name: this block
# reaches the profile through Invoke-Expression, where namespace imports have no effect.
class KcPreviewCache : System.Management.Automation.Subsystem.Prediction.ICommandPredictor {
    [guid] $Id = [guid]::NewGuid()
    [string] $Name = 'kc'
    [string] $Description = 'kc history preview'
    [System.Collections.Generic.Dictionary[string, string]] $FunctionsToDefine = $null
    [string] $Path
    [datetime] $Stamp = [datetime]::MinValue
    [System.Collections.Generic.Dictionary[string, string]] $Entries = [System.Collections.Generic.Dictionary[string, string]]::new()
    [System.Management.Automation.Subsystem.Prediction.SuggestionPackage] $Package

    KcPreviewCache([string] $cachePath) {
        $this.Path = $cachePath
        $this.Reload()
        # SuggestionPackage refuses an empty list, so the package is built once around a
        # seed entry; every call reuses its own List and returns the same package.
        $seed = [System.Collections.Generic.List[System.Management.Automation.Subsystem.Prediction.PredictiveSuggestion]]::new()
        $seed.Add([System.Management.Automation.Subsystem.Prediction.PredictiveSuggestion]::new('seed'))
        $this.Package = [System.Management.Automation.Subsystem.Prediction.SuggestionPackage]::new($seed)
        $this.Package.SuggestionEntries.Clear()
    }

    # One stat per keystroke: the file is only re-read when kc rewrote it.
    [void] Refresh() {
        if (-not (Test-Path -LiteralPath $this.Path)) {
            return
        }
        $current = (Get-Item -LiteralPath $this.Path).LastWriteTimeUtc
        if ($current -eq $this.Stamp) {
            return
        }
        $this.Reload()
    }

    [void] Reload() {
        $this.Entries.Clear()
        $this.Stamp = [datetime]::MinValue
        if (-not (Test-Path -LiteralPath $this.Path)) {
            return
        }
        $this.Stamp = (Get-Item -LiteralPath $this.Path).LastWriteTimeUtc
        # The notes are UTF-8; the default encoding would mangle them.
        foreach ($line in [System.IO.File]::ReadAllLines($this.Path, [System.Text.Encoding]::UTF8)) {
            $split = $line.IndexOf([char] 9)
            if ($split -le 0) {
                continue
            }
            $this.Entries[$line.Substring(0, $split)] = $line.Substring($split + 1)
        }
    }

    [System.Management.Automation.Subsystem.Prediction.SuggestionPackage] GetSuggestion(
        [System.Management.Automation.Subsystem.Prediction.PredictionClient] $client,
        [System.Management.Automation.Subsystem.Prediction.PredictionContext] $context,
        [System.Threading.CancellationToken] $cancellationToken) {
        $target = $this.Package.SuggestionEntries
        $target.Clear()
        $this.Refresh()
        $prefix = $context.InputAst.Extent.Text
        if (-not [string]::IsNullOrEmpty($prefix)) {
            foreach ($pair in $this.Entries.GetEnumerator()) {
                if ($pair.Key.StartsWith($prefix, [System.StringComparison]::OrdinalIgnoreCase)) {
                    $target.Add([System.Management.Automation.Subsystem.Prediction.PredictiveSuggestion]::new($pair.Key))
                    if ($target.Count -ge 10) {
                        break
                    }
                }
            }
        }
        return $this.Package
    }

    [bool] CanAcceptFeedback(
        [System.Management.Automation.Subsystem.Prediction.PredictionClient] $client,
        [System.Management.Automation.Subsystem.Prediction.PredictorFeedbackKind] $feedbackKind) {
        return $false
    }

    [void] OnSuggestionDisplayed(
        [System.Management.Automation.Subsystem.Prediction.PredictionClient] $client,
        [uint32] $session, [int] $count) {
    }

    [void] OnSuggestionAccepted(
        [System.Management.Automation.Subsystem.Prediction.PredictionClient] $client,
        [uint32] $session, [string] $acceptedSuggestion) {
    }

    [void] OnCommandLineAccepted(
        [System.Management.Automation.Subsystem.Prediction.PredictionClient] $client,
        [System.Collections.Generic.IReadOnlyList[string]] $history) {
    }

    [void] OnCommandLineExecuted(
        [System.Management.Automation.Subsystem.Prediction.PredictionClient] $client,
        [string] $commandLine, [bool] $success) {
    }
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stays_ascii_so_any_console_codepage_can_decode_it() {
        assert!(
            POWERSHELL.is_ascii(),
            "脚本含非 ASCII 字节，管道解码会破坏它"
        );
    }

    #[test]
    fn writes_the_preview_cache_path_as_a_single_quoted_literal() {
        let path = Path::new(r"C:\Users\k\.kc\preview.tsv");
        let output = script(path);
        assert!(output.contains(r"$global:KcPreviewPath = 'C:\Users\k\.kc\preview.tsv'"));
        assert!(!output.contains(PREVIEW_PATH), "占位符没有被替换");
        assert!(output.is_ascii());
    }

    #[test]
    fn doubles_single_quotes_in_the_preview_path() {
        let output = script(Path::new("/tmp/it's/preview.tsv"));
        assert!(output.contains("$global:KcPreviewPath = '/tmp/it''s/preview.tsv'"));
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
    fn the_predictor_class_uses_fully_qualified_names() {
        assert!(POWERSHELL.contains(
            "class KcPreviewCache : System.Management.Automation.Subsystem.Prediction.ICommandPredictor"
        ));
        assert!(!POWERSHELL.contains("using namespace"));
    }

    #[test]
    fn the_predictor_reads_the_cache_as_utf8_and_filters_by_prefix() {
        assert!(POWERSHELL.contains("[System.Text.Encoding]::UTF8"));
        assert!(POWERSHELL.contains(
            "$pair.Key.StartsWith($prefix, [System.StringComparison]::OrdinalIgnoreCase)"
        ));
        assert!(POWERSHELL.contains("$context.InputAst.Extent.Text"));
    }

    #[test]
    fn the_predictor_caches_the_file_by_modification_time() {
        assert!(POWERSHELL.contains("(Get-Item -LiteralPath $this.Path).LastWriteTimeUtc"));
        assert!(POWERSHELL.contains("if ($current -eq $this.Stamp)"));
    }

    #[test]
    fn keeps_the_arrow_key_picker() {
        assert!(POWERSHELL.contains("function global:Invoke-KcPick"));
        assert!(POWERSHELL.contains("Set-PSReadLineKeyHandler -Chord UpArrow"));
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
