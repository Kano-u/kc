/// 这份脚本必须保持纯 ASCII：它经 `kc init --shell powershell | Invoke-Expression`
/// 进入 PowerShell，而管道的解码用的是控制台代码页。非 ASCII 字节在
/// 代码页不是 UTF-8 的终端（如 936）会被解坏，脚本随即语法错误。
pub(super) const POWERSHELL: &str = r##"# Keep this block as the last statement of the profile.
# It defines global:prompt, so any prompt defined after it would win
# and commands would silently stop reaching the history database.

# The paths arrive base64-encoded so this script stays pure ASCII: KC_CONFIG_DIR may
# contain non-ASCII characters, and the pipe that feeds Invoke-Expression decodes with
# the console codepage, which mangles them on a non-UTF-8 terminal (e.g. 936).
$KcUtf8 = [System.Text.Encoding]::UTF8
$global:KcHistoryDb = $KcUtf8.GetString(
    [System.Convert]::FromBase64String('@KC_HISTORY_DB@'))
$global:KcNotesDir = $KcUtf8.GetString(
    [System.Convert]::FromBase64String('@KC_NOTES_DIR@'))

# The predictor is compiled C#, not a PowerShell class, and the source travels as a
# base64 data URI so this script stays pure ASCII. Two PSReadLine facts force this:
# it runs the predictor on a thread-pool thread where no runspace exists, so a
# PowerShell class cannot execute there at all; and it calls the predictor from the
# render path, where interpreted script would be felt on every keystroke.
# Compiling costs ~450 ms, so the assembly is cached under the data directory and
# keyed by the PowerShell version.
$KcPredictorSource = $KcUtf8.GetString(
    [System.Convert]::FromBase64String(('@KC_PREDICTOR_URI@' -split ',', 2)[1]))
$KcPreviewDll = Join-Path (Split-Path -Parent $global:KcHistoryDb) `
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

$global:KcPreviewPredictor = [KcPreviewPredictor]::new($global:KcHistoryDb, $global:KcNotesDir)
[System.Management.Automation.Subsystem.SubsystemManager]::RegisterSubsystem(
    [System.Management.Automation.Subsystem.SubsystemKind]::CommandPredictor,
    $global:KcPreviewPredictor
)

# Plugin, not the combined history-and-plugin source: PSReadLine drops plugin suggestions
# that are byte-identical to a history entry, which would hide kc's own candidates.
Set-PSReadLineOption -PredictionSource Plugin
Set-PSReadLineOption -PredictionViewStyle ListView

Remove-Variable KcPredictorSource, KcPreviewDll, KcPreviewTemp, KcUtf8 -ErrorAction SilentlyContinue

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
"##;

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(POWERSHELL.contains(
            "[KcPreviewPredictor]::new($global:KcHistoryDb, $global:KcNotesDir)"
        ));
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
}
