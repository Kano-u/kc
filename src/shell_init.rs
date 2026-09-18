use std::process::ExitCode;

pub fn print(shell: Option<&str>) -> ExitCode {
    match shell {
        Some("powershell") => {
            print!("{POWERSHELL}");
            ExitCode::SUCCESS
        }
        Some("bash") => {
            print!("{BASH}");
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!("用法: kc init powershell | kc init bash");
            ExitCode::from(2)
        }
    }
}

const POWERSHELL: &str = r#"
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

const BASH: &str = r#"
__kc_pick() {
    local __kc_file __kc_action __kc_command
    __kc_file="$(mktemp)" || return

    if ! KC_RESULT_FILE="$__kc_file" kc pick \
        --query "$READLINE_LINE" \
        --result-file-env KC_RESULT_FILE \
        --format nul; then
        rm -f "$__kc_file"
        return
    fi

    exec 3<"$__kc_file"
    IFS= read -r -d '' __kc_action <&3 || true
    IFS= read -r -d '' __kc_command <&3 || true
    exec 3<&-
    rm -f "$__kc_file"

    case "$__kc_action" in
        cancel) return ;;
        execute)
            if declare -F __atuin_accept_line >/dev/null 2>&1; then
                __atuin_accept_line "$__kc_command"
            else
                READLINE_LINE="$__kc_command"
                READLINE_POINT=${#READLINE_LINE}
            fi
            ;;
        *)
            READLINE_LINE="$__kc_command"
            READLINE_POINT=${#READLINE_LINE}
            ;;
    esac
}

if declare -F atuin-bind >/dev/null 2>&1; then
    atuin-bind -m emacs      '\e[A' __kc_pick
    atuin-bind -m emacs      '\eOA' __kc_pick
    atuin-bind -m vi-insert  '\e[A' __kc_pick
    atuin-bind -m vi-insert  '\eOA' __kc_pick
    atuin-bind -m vi-command '\e[A' __kc_pick
    atuin-bind -m vi-command '\eOA' __kc_pick
else
    bind -x '"\e[A": __kc_pick'
    bind -x '"\eOA": __kc_pick'
fi
"#;
