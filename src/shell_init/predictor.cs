public sealed class KcPreviewPredictor : System.Management.Automation.Subsystem.Prediction.ICommandPredictor
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
