# kc

自记录命令历史的搜索与备注工具。

## 功能

- 输入命令前缀时，命令行下方列出匹配的 kc 历史与备注（PowerShell 集成）
- 预览列表里有选中项时，`↑`/`↓` 在列表里移动；没有选中项时 `↑` 打开 TUI
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

### 命令预览

脚本里注册了一个 PSReadLine 预测器：输入命令前缀时，命令行下方列出 kc 历史里匹配的命令，每条候选后面跟着它的备注。

- 候选来自 `kc export` 写出的缓存文件，位置见「命令历史」一节的路径表。预测器读文件而不是调用 kc，按键路径上不会有进程启动开销
- 缓存文件里最新执行过的命令排在最前，所以输入前缀时拿到的是**最近用过的**匹配命令
- 预测器只在缓存文件变化时重读；`kc record` 每次记录后都会刷新它，也可以手动执行 `kc export`
- 只有单行、不含 TAB 的命令会被导出：接受建议时插入的是缓存文件原文，多行命令没法原样插入，这类命令留给 TUI
- 匹配是**前缀匹配**（`StartsWith(输入, OrdinalIgnoreCase)`，忽略大小写），跟 PSReadLine 自己的历史预测一致；子串匹配暂不做
- 候选最多 10 条
- 预测器是编译成 DLL 的 C#，不是 PowerShell 脚本。这不是偏好：PSReadLine 把预测器放在没有 runspace 的线程池线程上跑，只给 20 ms 预算，解释执行的脚本两边都过不了
- 首次执行 `kc init` 时会编译一次，约 450 ms；DLL 缓存在 `~/.kc/KcPreviewPredictor-<PowerShell 版本>.dll`，之后启动只读它。**升级 kc 或 PowerShell 后都要删掉这个 DLL 才会重新编译**：程序集绑到具体的 SMA 版本，版本变了旧 DLL 用不了
- 升级 kc 后需要重新执行上面的 `kc init` 才会生效
- `↓` 进入候选列表并在列表内下移，`↑` 在已选中时上移；没有任何选中项时 `↑` 才打开 kc TUI。判断选中状态需要 `prediction-selection` 补丁，见下一节
- `F2` 在 Inline 与 ListView 之间切换，预览默认用 ListView
- 预测源的选项必须是 `Plugin`，不能改成 `HistoryAndPlugin`：PSReadLine 会滤掉与 History 源完全相同的建议，kc 的候选会被历史源顶掉，列表直接变空

**必须给 PSReadLine 打补丁**，见下一节。补丁提供两件事：

- 备注显示在每行候选后面。不打补丁时备注只在选中项的下方，需要先选中才看得到——这是 PSReadLine 的行为：命中备注的字段是 `ToolTip`，而它只在候选被选中时才渲染
- `HasPredictionSelection()` 只读 API。`↑` 靠它分辨「预览里有选中项」，没有它 `↑` 会直接报错。

### 打补丁（kc psreadline-patch）

```powershell
kc psreadline-patch
```

会在本机把 [Kano-u/PSReadLine-Patch](https://github.com/Kano-u/PSReadLine-Patch) 的补丁应用到当前 PSReadLine 上：

- `note-column`：让候选行尾部的 `[来源]` 位置显示备注
- `prediction-selection`：`HasPredictionSelection()` 只读 API，让 kc 能分辨「预览里有选中项」

```
PS D:\> npx
   > npx create-react-app my-app                        创建 react 项目
   > npx tsc --noEmit                                   只做类型检查
   > npx @agegr/pi-web@latest                           pi web 网页端
   ⋮
   <kc(10)>
```

- 需要 `git` 和 `dotnet` SDK。命令会自动问 pwsh 要当前 PSReadLine 版本与用户模块目录，然后拉取对应上游标签、应用补丁、本机编译、装进用户模块目录，最后新开一个 pwsh 验证是否真的加载到了补丁版
- 两个补丁各自独立、互不重叠，所以能叠在同一份上游源码上；改 `PATCH_FILES` 加补丁即可
- 构建目录 `~/.kc/psreadline` 每次运行整个重建
- **装之前必须关掉所有加载了 profile 的 PowerShell 窗口**：已加载的 DLL 在 Windows 上被锁住，覆盖必然失败。命令会先探测文件锁，有占用就整体拒绝并列出文件，不会留下半残的模块目录（非交互式的 `pwsh -File` 脚本不加载 profile，不会锁）
- 补丁只装进当前用户的模块目录，不动系统目录，也不需要管理员权限
- **升级 PowerShell 后要重跑一次**：PSReadLine 随 PowerShell 一起升级，版本号变了就得对着新版本重新打一次
- 装完要重启 PowerShell 窗口才生效
- 注：这是个人自用的改动，不向上游提交。打补丁后每行不再标注来源，来源仍可从底部 `<kc(10)>` 和 `Ctrl+↑↓` 看到

## zsh（Termux）集成

依赖 `jq`，先装：

```sh
pkg install jq
```

在 `.zshrc` 的**最后一行**加：

```zsh
eval "$(kc init --shell zsh)"
```

这段输出必须放在最后一行：它注册 `precmd`/`preexec` 钩子，之后定义的钩子虽然照样工作，但 kc 会看不到要记录的命令。

- 记录方式：`preexec` 抓命令原文，`precmd` 取 `$?`。成功失败都记，空回车不记
- `precmd` 里 `local ok=$?` 必须是第一条语句：它前面任何语句都会把真实状态码冲掉
- `↑` 打开 kc TUI；`Enter` 执行、`Tab`/`→` 插入不执行、`Esc` 取消且命令行不变
- `↑` 不再浏览 zsh 历史，原生 `Ctrl+R` 反向搜索保留不动
- 需要设置 `KC_CONFIG_DIR`（与其它平台一致），历史库仍在 `~/.kc/history.db`
- 不包含：`kc import --shell zsh`、`zsh-autosuggestions` 预览（下一期）

## 命令历史

kc 自己记录命令历史，不依赖 Atuin 等外部工具。

记录由 PowerShell 的 `prompt` 钩子驱动：每次提示符出现前，kc 读取刚执行的那条命令，连同成功/失败一并写入数据库。失败的命令同样记录。

历史数据库与预览缓存都固定在用户目录，与同步目录无关，也不参与同步：

| 平台 | 历史数据库 | 预览缓存 |
| --- | --- | --- |
| Windows | `%USERPROFILE%\.kc\history.db` | `%USERPROFILE%\.kc\preview.tsv` |
| 其他 | `~/.kc/history.db` | `~/.kc/preview.tsv` |

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
