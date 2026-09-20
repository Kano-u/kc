# kc

基于 Atuin 的命令历史搜索与备注工具。

## 功能

- 向上箭头打开 TUI
- 顶部工具栏提供“备注”、“复制”和“删除”，也可点击操作
- 底部搜索，实时过滤命令和备注
- `Enter` 执行选中命令
- `Tab` / `→` 插入命令但不执行
- `←` 编辑选中命令备注
- 鼠标/触摸：点击选择、滚轮滚动、点击保存
- `Shift+Backspace` 删除选中命令的 Atuin 历史；无备注直接删除，有备注需确认
- 输入以空格开头时只搜索有备注的命令，空格后的文字参与匹配
- 鼠标/触摸：点击选择、滚轮滚动、点击保存或确认
- 主机标记 `.host` 决定当前主机的配置文件和备注写入文件
- 同目录下所有 `*.notes.jsonl` 会合并读取，同名备注取更新时间较新的记录

## 安装

```powershell
cargo install --path .
```

## 从 Git Clone 安装

先在系统中安装 `git`、Rust 工具链和 Atuin。然后：

```bash
git clone https://github.com/Kano-u/kc.git
cd kc
cargo install --path .
```

更新时重新拉取并强制安装：

```bash
git pull
cargo install --path . --force
```

## PowerShell 集成

在 PowerShell profile 中添加：

```powershell
kc init powershell | Out-String | Invoke-Expression
```

## Termux / Bash 集成

在 `~/.bashrc` 中添加：

```bash
eval "$(kc init bash)"
```

## 配置

同步目录中必须恰好存在一个 `.host` 文件，例如 `com.host`。文件名决定：

- 配置文件：`com.config.toml`
- 备注写入文件：`com.notes.jsonl`

kc 会合并读取目录下所有 `*.notes.jsonl`。同名命令保留 `updated_at` 最新的一条，但保存时只写当前 `.host` 对应的备注文件。

```toml
history_limit = 5000
```

`filter` 是排除规则列表，匹配任一正则的命令会从列表中隐藏。正则默认不是完全匹配，若要完整匹配整条命令，请加上 `^` 和 `$`：

```toml
history_limit = 5000
filter = [
  "^dir$",
  "^ls -la$",
  "secret",
]
```

上面的 `^dir$` 和 `^ls -la$` 只过滤完全相同的命令；`secret` 会过滤任何包含该文字的命令。

### 自定义配置与数据目录

设置 `KC_CONFIG_DIR` 后，kc 从该目录查找 `.host`、`*.config.toml` 和 `*.notes.jsonl`。未设置时使用 `~/.config/kc`。

```bash
export KC_CONFIG_DIR="$HOME/kc"
```

建议使用绝对路径；相对路径会相对于启动 `kc` 时的工作目录解析。

## 开发检查

```powershell
cargo test
cargo clippy --all-targets -- -D warnings
cargo build --release
```
