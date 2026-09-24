use crate::args;
use crate::data_paths::{config_dir, history_db};
use std::path::Path;
use std::process::ExitCode;

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

/// zsh 版本（Termux/WSL）。注释写英文并非编码需要，而是避免再多一条要验证的规则。
///
/// 两个钩子分工是必须的：`preexec` 的 `$1` 才是用户输入原文，状态则要在
/// `precmd` 取。钩子顺序不影响 `$?`：zsh 在每个 hook 函数进出时都会保存并恢复它，
/// 所以不管 `_kc_precmd` 排在哪里，读到的都是上一条命令的真实状态。
/// 仍然独立取状态的原因是 `preexec` 里 `$?` 指的是之前的东西，不是刚要跑的命令。
const ZSH: &str = r#"# Keep this block as the last statement of .zshrc.
# It defines precmd/preexec hooks; anything defined after it still works,
# but kc would stop seeing commands.

# Hook order does not matter: zsh restores $? for each hook function, so
# _kc_precmd sees the real status wherever it sits. Keep 'local ok=$?'
# the first line of _kc_precmd, though: any statement before it overwrites it.
_kc_pending=

_kc_preexec() {
  _kc_pending=$1
}

_kc_precmd() {
  local ok=$?
  [[ -n $_kc_pending ]] || return 0
  KC_RECORD=$(( ok == 0 ? 1 : 0 )) KC_COMMAND=$_kc_pending \
    kc record --command-env KC_COMMAND
  unset KC_RECORD KC_COMMAND _kc_pending
}

preexec_functions=(_kc_preexec $preexec_functions)
precmd_functions=(_kc_precmd $precmd_functions)

# zle -I hands the terminal to the TUI; without it ZLE and the full-screen
# TUI fight over the screen. zle reset-prompt then redraws the line, which
# is what leaves the prompt clean after the alternate screen is restored.
#
# kc pick emits JSON and jq parses it: hand-rolled parsing would break on
# multi-line commands and quotes, and JSON escaping is the one format kc
# promises. 'jq -r' also drops the quotes a raw JSON string would keep.
_kc_pick() {
  zle -I
  local result_file=$(mktemp)
  KC_QUERY=$BUFFER KC_RESULT_FILE=$result_file \
    kc pick --query-env KC_QUERY --result-file-env KC_RESULT_FILE
  local action=$(jq -r '.action' "$result_file")
  local command=$(jq -r '.command // empty' "$result_file")
  rm -f "$result_file"
  case $action in
    insert)  BUFFER=$command; CURSOR=${#BUFFER}; zle reset-prompt ;;
    execute) BUFFER=$command; CURSOR=${#BUFFER}; zle accept-line ;;
    *)       zle reset-prompt ;;
  esac
}
zle -N _kc_pick

# terminfo first, literal sequence as fallback: on terminals whose terminfo
# lacks kcuu1 the literal is the only way UpArrow reaches the widget.
[[ -n ${terminfo[kcuu1]} ]] && bindkey ${terminfo[kcuu1]} _kc_pick
bindkey '^[[A' _kc_pick
"#;

/// 这份脚本必须保持纯 ASCII：它经 `kc init --shell powershell | Invoke-Expression`
/// 进入 PowerShell，而管道的解码用的是控制台代码页。非 ASCII 字节在
/// 代码页不是 UTF-8 的终端（如 936）会被解坏，脚本随即语法错误。
const POWERSHELL: &str = r#"# Keep this block as the last statement of the profile.
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
"#;

/// 预测器源码。方法体里只能出现 .NET 调用：它跑在 PSReadLine 的线程池线程上，
/// 那里没有 runspace，cmdlet 与类内方法调用都会失败（而且失败会被静默吞掉）。
/// 类型全部写全限定名，因为这段代码经 `Add-Type` 编译时没有 `using` 命名空间。
///
/// 它直接读 `history.db` 与 `*.notes.jsonl`，不再需要 `kc` 预先导出任何缓存：
/// SQLite 走 P/Invoke 到 Windows 自带的 `winsqlite3.dll` —— 这恰好绕开了
/// “线程池线程上没有 runspace”的限制，因为 P/Invoke 不经过 PowerShell。
const PREDICTOR: &str = r#"public sealed class KcPreviewPredictor : System.Management.Automation.Subsystem.Prediction.ICommandPredictor
{
    // Windows 自带 winsqlite3.dll，所以这个 P/Invoke 不引入任何依赖。
    private const int SqliteOk = 0;
    private const int SqliteRow = 100;
    private const int SqliteOpenReadOnly = 1;

    [System.Runtime.InteropServices.DllImport("winsqlite3.dll", CallingConvention = System.Runtime.InteropServices.CallingConvention.Cdecl)]
    private static extern int sqlite3_open_v2(byte[] filename, out System.IntPtr db, int flags, System.IntPtr vfs);
    [System.Runtime.InteropServices.DllImport("winsqlite3.dll", CallingConvention = System.Runtime.InteropServices.CallingConvention.Cdecl)]
    private static extern int sqlite3_busy_timeout(System.IntPtr db, int milliseconds);
    [System.Runtime.InteropServices.DllImport("winsqlite3.dll", CallingConvention = System.Runtime.InteropServices.CallingConvention.Cdecl)]
    private static extern int sqlite3_prepare_v2(System.IntPtr db, byte[] sql, int length, out System.IntPtr statement, System.IntPtr tail);
    [System.Runtime.InteropServices.DllImport("winsqlite3.dll", CallingConvention = System.Runtime.InteropServices.CallingConvention.Cdecl)]
    private static extern int sqlite3_step(System.IntPtr statement);
    [System.Runtime.InteropServices.DllImport("winsqlite3.dll", CallingConvention = System.Runtime.InteropServices.CallingConvention.Cdecl)]
    private static extern System.IntPtr sqlite3_column_text(System.IntPtr statement, int column);
    [System.Runtime.InteropServices.DllImport("winsqlite3.dll", CallingConvention = System.Runtime.InteropServices.CallingConvention.Cdecl)]
    private static extern int sqlite3_column_bytes(System.IntPtr statement, int column);
    [System.Runtime.InteropServices.DllImport("winsqlite3.dll", CallingConvention = System.Runtime.InteropServices.CallingConvention.Cdecl)]
    private static extern int sqlite3_finalize(System.IntPtr statement);
    [System.Runtime.InteropServices.DllImport("winsqlite3.dll", CallingConvention = System.Runtime.InteropServices.CallingConvention.Cdecl)]
    private static extern int sqlite3_close(System.IntPtr db);

    private readonly string _dbPath;
    private readonly string _notesDir;

    // 顺序即优先级：查询已按 `at DESC` 排好，所以按顺序取前 10 条就是最近用过的 10 条。
    // 不能用字典 —— 字典不保证迭代顺序，候选会退化成随机 10 条。
    private readonly System.Collections.Generic.List<System.Management.Automation.Subsystem.Prediction.PredictiveSuggestion> _entries =
        new System.Collections.Generic.List<System.Management.Automation.Subsystem.Prediction.PredictiveSuggestion>();

    // 源文件的修改时间与大小。每按一键只比一次，内容只在变化时重读。
    private string _stamp;

    private readonly System.Collections.Generic.List<System.Management.Automation.Subsystem.Prediction.PredictiveSuggestion> _list;
    private readonly System.Management.Automation.Subsystem.Prediction.SuggestionPackage _package;

    public System.Guid Id { get; } = System.Guid.NewGuid();
    public string Name => "kc";
    public string Description => "kc history preview";
    public System.Collections.Generic.Dictionary<string, string> FunctionsToDefine => null;

    public KcPreviewPredictor(string dbPath, string notesDir)
    {
        _dbPath = dbPath;
        _notesDir = notesDir;
        // SuggestionPackage refuses an empty list, so it is built once around a seed
        // entry and every call reuses the list inside it, emptied first.
        var seed = new System.Collections.Generic.List<System.Management.Automation.Subsystem.Prediction.PredictiveSuggestion>();
        seed.Add(new System.Management.Automation.Subsystem.Prediction.PredictiveSuggestion("seed"));
        _package = new System.Management.Automation.Subsystem.Prediction.SuggestionPackage(seed);
        _list = _package.SuggestionEntries;
        _list.Clear();
    }

    public System.Management.Automation.Subsystem.Prediction.SuggestionPackage GetSuggestion(
        System.Management.Automation.Subsystem.Prediction.PredictionClient client,
        System.Management.Automation.Subsystem.Prediction.PredictionContext context,
        System.Threading.CancellationToken cancellationToken)
    {
        _list.Clear();

        // One stat per keystroke: only re-read when the database or a notes file changed.
        // A transient failure keeps the previous snapshot rather than emptying the list.
        string current = Stamp();
        if (current != _stamp)
        {
            _stamp = current;
            try
            {
                Reload();
            }
            catch
            {
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

    /// 历史数据库与所有备注文件的修改时间与大小。任一变化都足以触发重读。
    private string Stamp()
    {
        var stamp = new System.Text.StringBuilder();
        AppendStamp(stamp, _dbPath);
        if (!string.IsNullOrEmpty(_notesDir) && System.IO.Directory.Exists(_notesDir))
        {
            string[] files = System.IO.Directory.GetFiles(_notesDir, "*.notes.jsonl");
            System.Array.Sort(files, System.StringComparer.Ordinal);
            foreach (var file in files)
            {
                AppendStamp(stamp, file);
            }
        }
        return stamp.ToString();
    }

    private static void AppendStamp(System.Text.StringBuilder stamp, string path)
    {
        try
        {
            var info = new System.IO.FileInfo(path);
            if (info.Exists)
            {
                stamp.Append(info.LastWriteTimeUtc.Ticks).Append(':').Append(info.Length);
            }
        }
        catch
        {
        }
        stamp.Append('|');
    }

    private void Reload()
    {
        _entries.Clear();
        var notes = ReadNotes();
        foreach (var command in ReadCommands())
        {
            string note;
            notes.TryGetValue(command, out note);
            // The note is display-only and rides in ToolTip; it never reaches the
            // command line. An empty note becomes null, not an empty string.
            _entries.Add(new System.Management.Automation.Subsystem.Prediction.PredictiveSuggestion(
                command, string.IsNullOrEmpty(note) ? null : note));
        }
    }

    /// 按最近使用在前读出全部命令。整表载入内存，按键路径上就不必再碰数据库。
    /// 全量只发生在源文件变化时，实测 2 万条约 17 ms。
    private System.Collections.Generic.List<string> ReadCommands()
    {
        var commands = new System.Collections.Generic.List<string>();
        if (!System.IO.File.Exists(_dbPath))
        {
            return commands;
        }
        System.IntPtr db;
        if (sqlite3_open_v2(Utf8Z(_dbPath), out db, SqliteOpenReadOnly, System.IntPtr.Zero) != SqliteOk)
        {
            return commands;
        }
        try
        {
            // A writer holds the lock only briefly; wait instead of failing outright.
            sqlite3_busy_timeout(db, 100);
            System.IntPtr statement;
            if (sqlite3_prepare_v2(db, Utf8Z("SELECT command FROM history ORDER BY at DESC"), -1, out statement, System.IntPtr.Zero) != SqliteOk)
            {
                return commands;
            }
            try
            {
                while (sqlite3_step(statement) == SqliteRow)
                {
                    string command = Column(statement, 0);
                    if (string.IsNullOrEmpty(command))
                    {
                        continue;
                    }
                    // PSReadLine's own history suggestions skip multi-line commands,
                    // and the command line cannot hold one verbatim.
                    if (command.IndexOf('\n') != -1 || command.IndexOf('\r') != -1)
                    {
                        continue;
                    }
                    commands.Add(command);
                }
            }
            finally
            {
                sqlite3_finalize(statement);
            }
        }
        finally
        {
            sqlite3_close(db);
        }
        return commands;
    }

    /// 合并目录下所有 `*.notes.jsonl`；同名命令取 `updated_at` 较新的一条，
    /// 与 kc 自己的合并规则一致。文件按路径排序，使结果与枚举顺序无关。
    private System.Collections.Generic.Dictionary<string, string> ReadNotes()
    {
        var notes = new System.Collections.Generic.Dictionary<string, string>(System.StringComparer.Ordinal);
        var stamps = new System.Collections.Generic.Dictionary<string, string>(System.StringComparer.Ordinal);
        if (string.IsNullOrEmpty(_notesDir) || !System.IO.Directory.Exists(_notesDir))
        {
            return notes;
        }
        string[] files = System.IO.Directory.GetFiles(_notesDir, "*.notes.jsonl");
        System.Array.Sort(files, System.StringComparer.Ordinal);
        foreach (var file in files)
        {
            // The notes are UTF-8; the default encoding would mangle them.
            foreach (var line in System.IO.File.ReadAllLines(file, System.Text.Encoding.UTF8))
            {
                if (line.Length == 0)
                {
                    continue;
                }
                try
                {
                    using (var document = System.Text.Json.JsonDocument.Parse(line))
                    {
                        var root = document.RootElement;
                        string command = root.GetProperty("command").GetString();
                        if (string.IsNullOrEmpty(command))
                        {
                            continue;
                        }
                        string note = root.TryGetProperty("note", out var noteElement) ? noteElement.GetString() : null;
                        string updated = root.TryGetProperty("updated_at", out var stampElement) ? stampElement.GetString() : "";
                        string previous;
                        if (!stamps.TryGetValue(command, out previous) || System.String.CompareOrdinal(previous, updated) <= 0)
                        {
                            stamps[command] = updated;
                            notes[command] = note ?? "";
                        }
                    }
                }
                catch
                {
                }
            }
        }
        return notes;
    }

    private static byte[] Utf8Z(string value)
    {
        byte[] bytes = System.Text.Encoding.UTF8.GetBytes(value);
        byte[] terminated = new byte[bytes.Length + 1];
        System.Array.Copy(bytes, terminated, bytes.Length);
        return terminated;
    }

    private static string Column(System.IntPtr statement, int index)
    {
        System.IntPtr pointer = sqlite3_column_text(statement, index);
        if (pointer == System.IntPtr.Zero)
        {
            return null;
        }
        int length = sqlite3_column_bytes(statement, index);
        if (length == 0)
        {
            return "";
        }
        byte[] buffer = new byte[length];
        System.Runtime.InteropServices.Marshal.Copy(pointer, buffer, 0, length);
        return System.Text.Encoding.UTF8.GetString(buffer);
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

    #[test]
    fn zsh_output_is_ascii_and_succeeds() {
        assert_eq!(
            main(&["--shell".to_owned(), "zsh".to_owned()]),
            ExitCode::SUCCESS
        );
        assert!(rendered_zsh().is_ascii(), "zsh 脚本含非 ASCII 字节");
        assert_eq!(rendered_zsh(), ZSH);
    }

    #[test]
    fn zsh_registers_the_hooks_without_overwriting_existing_ones() {
        // 追加式赋值会覆盖别人的钩子，必须保留原有函数数组。
        assert!(ZSH.contains("precmd_functions=(_kc_precmd $precmd_functions)"));
        assert!(ZSH.contains("preexec_functions=(_kc_preexec $preexec_functions)"));
        assert!(!ZSH.contains("precmd_functions+=("));
        assert!(!ZSH.contains("preexec_functions+=("));
    }

    #[test]
    fn zsh_captures_the_status_on_the_first_precmd_line() {
        let precmd = ZSH
            .split_once("_kc_precmd() {")
            .expect("_kc_precmd 函数缺失")
            .1;
        let first = precmd
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty() && !line.starts_with('#'))
            .expect("_kc_precmd 函数体为空");
        assert_eq!(first, "local ok=$?");
    }

    #[test]
    fn zsh_records_the_command_with_its_status() {
        assert!(ZSH.contains("KC_RECORD=$(( ok == 0 ? 1 : 0 ))"));
        assert!(ZSH.contains("kc record --command-env KC_COMMAND"));
        assert!(ZSH.contains("_kc_pending=$1"));
        assert!(ZSH.contains("unset KC_RECORD KC_COMMAND _kc_pending"));
    }

    #[test]
    fn zsh_pick_hands_the_terminal_over_before_running_the_tui() {
        assert!(ZSH.contains("zle -I"), "ZLE 与全屏 TUI 会抢屏幕");
    }

    #[test]
    fn zsh_pick_inserts_without_executing_and_executes_on_accept() {
        let insert = ZSH
            .split_once("insert)")
            .expect("insert 分支缺失")
            .1
            .lines()
            .next()
            .expect("insert 分支为空");
        let execute = ZSH
            .split_once("execute)")
            .expect("execute 分支缺失")
            .1
            .lines()
            .next()
            .expect("execute 分支为空");
        assert!(insert.contains("zle reset-prompt"));
        assert!(!insert.contains("accept-line"));
        assert!(execute.contains("zle accept-line"));
    }

    #[test]
    fn zsh_pick_binds_up_arrow_from_terminfo_with_a_literal_fallback() {
        assert!(ZSH.contains("terminfo[kcuu1]"));
        assert!(ZSH.contains("bindkey '^[[A' _kc_pick"));
        assert!(ZSH.contains("zle -N _kc_pick"));
    }

    #[test]
    fn zsh_pick_parses_the_json_with_jq_and_never_by_hand() {
        assert!(ZSH.contains("jq -r '.action'"));
        assert!(ZSH.contains("jq -r '.command // empty'"));
        // 结果文件只经由 jq 读：手写解析会在多行命令、引号、反斜杠上出错。
        for line in ZSH.lines().filter(|line| line.contains("$result_file")) {
            let parsed = line.contains("jq -r")
                || line.contains("rm -f")
                || line.contains("KC_RESULT_FILE=$result_file");
            assert!(parsed, "结果文件出现了非 jq 的读取: {line}");
        }
    }

    #[test]
    fn zsh_pick_passes_the_current_buffer_as_the_query() {
        assert!(ZSH.contains("KC_QUERY=$BUFFER KC_RESULT_FILE=$result_file"));
        assert!(ZSH.contains("kc pick --query-env KC_QUERY --result-file-env KC_RESULT_FILE"));
    }

    #[test]
    fn zsh_script_contains_both_the_hooks_and_the_picker() {
        // 两个能力必须进同一份常量，否则 eval 一次只能拿到一半。
        assert!(ZSH.contains("_kc_precmd"));
        assert!(ZSH.contains("_kc_pick"));
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
