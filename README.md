# kc

自记录命令历史的搜索与备注工具。

## 功能

- 向上箭头打开 TUI
- 顶部工具栏提供“备注”、“复制”和“删除”，也可点击操作
- 底部搜索，实时过滤命令和备注
- `Enter` 执行选中命令
- `Tab` / `→` 插入命令但不执行
- `←` 编辑选中命令备注
- `Shift+Backspace` 删除选中命令，无备注直接删除，有备注需确认
- 输入以空格开头时只搜索有备注的命令，空格后的文字参与匹配
- 鼠标/触摸：点击选择、滚轮滚动、点击保存或确认
- 主机标记 `.host` 决定当前主机的配置文件和备注写入文件
- 同目录下所有 `*.notes.jsonl` 会合并读取，同名备注取更新时间较新的记录

## 安装

```powershell
cargo install --path .
```

## 从 Git Clone 安装

先在系统中安装 `git` 和 Rust 工具链。然后：

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
kc init --shell powershell | Out-String | Invoke-Expression
```

这段输出必须放在 profile 的最后一行：它定义 `global:prompt`，之后定义的 prompt 会把它覆盖掉，命令就会悄悄不再进历史。

## 命令历史

kc 自己记录命令历史，不依赖 Atuin 等外部工具。

记录由 PowerShell 的 `prompt` 钩子驱动：每次提示符出现前，kc 读取刚执行的那条命令，连同成功/失败一并写入数据库。失败的命令同样记录。

历史数据库固定在用户目录，与同步目录无关，也不参与同步：

| 平台 | 路径 |
| --- | --- |
| Windows | `%USERPROFILE%\.kc\history.db` |
| 其他 | `~/.kc/history.db` |

数据库是 SQLite，`command` 是主键，同名命令只保留一条，重复执行刷新时间戳。删除是物理删除，文件里不留痕迹。

### 导入 PowerShell 历史

PSReadLine 自己的历史文件不会被 kc 读取，但可以一次性并入：

```powershell
kc import --shell powershell
```

它读取 `%APPDATA%\Microsoft\Windows\PowerShell\PSReadLine\ConsoleHost_history.txt`，按文件顺序写入历史库。该文件没有时间戳，kc 用当前时间往前铺开、依序递增，导入的命令排在现有历史之后。

处理方式与 `record` 一致：命中 `filter` 的命令不导入，已经存在的同名命令按导入顺序刷新时间和成功标记。行尾反引号是 PSReadLine 的续行标记，会被还原成命令里的换行，所以多行命令导入后是一条完整的命令。

重复导入是安全的：同名命令只是被刷新，不会产生重复行。

## 配置

同步目录中必须恰好存在一个 `.host` 文件，例如 `com.host`。文件名决定：

- 配置文件：`com.config.toml`
- 备注写入文件：`com.notes.jsonl`

kc 会合并读取目录下所有 `*.notes.jsonl`。同名命令保留 `updated_at` 最新的一条，但保存时只写当前 `.host` 对应的备注文件。

```toml
history_limit = 5000
```

`filter` 是排除规则列表，匹配任一正则的命令既不显示、也不记录，因此可以把密钥挡在历史数据库之外。正则默认不是完全匹配，若要完整匹配整条命令，请加上 `^` 和 `$`：

```toml
history_limit = 5000
filter = [
  "^dir$",
  "^ls -la$",
  "secret",
]
```

上面的 `^dir$` 和 `^ls -la$` 只过滤完全相同的命令；`secret` 会过滤任何包含该文字的命令。

配置文件解析失败、`history_limit` 不是非负整数、`filter` 不是字符串数组、含非字符串项或正则非法，kc 都直接报错退出，不会静默改用默认值——否则 `filter` 会悄悄失效，把本该挡住的命令写进历史库。

### 配置与数据目录

kc 不猜测默认位置，`KC_CONFIG_DIR` 必须设置。kc 从该目录查找 `.host`、`*.config.toml` 和 `*.notes.jsonl`。未设置时直接报错退出。

```bash
export KC_CONFIG_DIR="$HOME/kc"
```

建议使用绝对路径；相对路径会相对于启动 `kc` 时的工作目录解析。历史数据库不受此变量影响，始终在用户目录下。

## 开发检查

```powershell
cargo test
cargo clippy --all-targets -- -D warnings
cargo build --release
```
