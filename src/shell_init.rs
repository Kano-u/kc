use std::process::ExitCode;

pub fn print(shell: Option<&str>) -> ExitCode {
    match shell {
        Some("powershell") => {
            print!("{POWERSHELL}");
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!("用法: kc init powershell");
            ExitCode::from(2)
        }
    }
}

/// 这份脚本必须保持纯 ASCII：它经 `kc init powershell | Invoke-Expression`
/// 进入 PowerShell，而管道的解码用的是控制台代码页。非 ASCII 字节在
/// 代码页不是 UTF-8 的终端（如 936）会被解坏，脚本随即语法错误。
const POWERSHELL: &str = r#"# Keep this block as the last statement of the profile.
# It defines global:prompt, so any prompt defined after it would win
# and commands would silently stop reaching the history database.

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
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stays_ascii_so_any_console_codepage_can_decode_it() {
        assert!(POWERSHELL.is_ascii(), "脚本含非 ASCII 字节，管道解码会破坏它");
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
        assert_eq!(print(None), ExitCode::from(2));
        assert_eq!(print(Some("bash")), ExitCode::from(2));
    }
}
