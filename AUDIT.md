# kc 代码审计：违反 AGENTS.md 原则的地方

审计对象：`src/`（13 个文件，2948 行）。基线：`cargo test` 65 通过、`cargo clippy -D warnings` 无告警。

判定依据：

1. Go「少就是多」——**有且仅有一种方法把事情做好做对**。最优方案若失败，给它权限/修好它，而不是退回备选方案。
2. Arch Linux——**不考虑兼容性**，只追求最新稳定的单一方案。
3. 「只留关键，不堆兼容」——不保留补丁式兼容代码和备注。

## 用户已确认的范围

| 决定 | 影响 |
| --- | --- |
| 必须保留多主机备注合并 | B1 是功能不是包袱；实现方式要收敛（写回只写本机文件） |
| 只用 Termux 与 Windows | 剪贴板候选链里 4 个是死方案 |
| `KC_CONFIG_DIR` 不设置就报错 | B3 执行，不再有 `~/.config/kc` 默认位置 |
| 失败状态暂时不做界面 | `ok` 字段留着不动 |
| 配置错误直接报错（后续决定，覆盖之前的选择） | C2 已修复 |
| 家目录解析失败改成报错 | B2 执行 |
| Termux 的 shell 集成暂时不做 | 记为功能缺口（D 节） |

## 处理状态

- ✅ 已修复
- ⏸️ 已确认保留（撤回判定）
- 🔲 待办

---

## A. 同一件事存在多套实现

### ✅ A1. `kc pick` 有两套输出格式：`json` 与 `nul`

- 原位置：`src/pick.rs` 的 `enum OutputFormat`、`--format`、`encode` 两分支。
- 问题：`--format nul` 无任何调用方，README 与 `--help` 也未提及，却带着一个专属测试为死代码背书。
- 已做：删除 `OutputFormat`、`--format` 与 `Nul` 分支，`encode` 只返回 JSON。

### ✅ A2. `kc pick` 的参数各有两套来源

- 原位置：`--query` / `--query-env`、`--result-file` / `--result-file-env`，以及 `src/pick.rs` 里"谁覆盖谁"的优先级逻辑。
- 问题：实际只用 env 那套；命令行那套无调用方。缺值时还静默忽略（`if index < args.len()`）。
- 已做：只保留 `--query-env` / `--result-file-env`，缺值直接报错退出。同时把 `--help` 改成真实用法（原 C5 一部分）。

### ✅ A3. 非 Windows 剪贴板是「候选工具列表逐个回退」

- 原位置：`src/tui/clipboard.rs`，候选 `wl-copy` / `xclip` / `xsel` / `termux-clipboard-set` / `pbcopy`，失败就 `continue` 试下一个。
- 问题：正是「最优命令失败就退回其他方案」的原型。用户只用 Termux，其中 4 个是永远走不到的死分支，错误信息还被 `last_error` 稀释。
- 已做：非 Windows 只留 `termux-clipboard-set`，启动/写入/退出码任一环节出错都直接把原因报出来。

### ✅ A4. 配置默认值有两个来源

- 原位置：`Config::default()` 的 `history_limit: 5000` 与字段级回落 `.unwrap_or(5000)`。
- 问题：同一条默认值写两遍，改一处会不一致。
- 已做：`load_from` 先取 `Self::default()`，字段回落引用 `default.history_limit`，默认值只有一个权威来源。

### ✅ A5. 去重做了两遍（内存里一次、SQL 里一次）

- 原位置：`src/import.rs` 的 `dedup_keeping_last`（O(n²)）与 `src/history.rs` 的 `ON CONFLICT(command) DO UPDATE`。
- 问题：同一语义两个实现，内存版是低效重复劳动。
- 已做：删除 `dedup_keeping_last`，去重只交给历史表主键。

---

## B. 兼容包袱

### ⏸️ B1. `NoteStore` 维护 `local_notes` 与 `notes` 两份列表

- 位置：`src/notes.rs`（三个字段、`load_from_paths` 分别加载、`set`/`remove` 双写两份、`save` 只写 `local_notes`）。
- 判定：多主机备注合并是**用户明确要求的功能**，两份列表是"合并读取 + 只写回本机"的实现手段，不删除。
- 仍存的问题：`set` 每次 `clone` 一份 `Note`（`src/notes.rs:62`），`notes` 侧的查找是线性扫描。数据量小时无碍，属可接受的实现成本。
- 遗留：`path: Option<PathBuf>` 里的 `None` 分支在真实流程中不可达（`load()` 一定带 `write_notes()`），`save()` 的 `Err("备注路径未设置。")` 是纯防御代码。**建议**：改成非 `Option`。

### ✅ B2. 家目录回退，并把未知家目录降级成当前目录

- 原位置：`src/data_paths.rs`：`USERPROFILE` → `HOME` → `"."`。
- 问题：`HOME` 是 Termux 必须的（保留）；但最后 `PathBuf::from(".")` 会把历史与备注写进任意工作目录，是最危险的静默降级。
- 已做：`HOME` 保留，`"."` 兜底改成报错退出。为此 `history_db()`、`config_dir()` 返回 `Result`，调用点全部改为 `?` 传播。

### ✅ B3. `config_dir()` 的回退链

- 原位置：`src/data_paths.rs`：`KC_CONFIG_DIR` 非空则用，否则 `~/.config/kc`。
- 问题：默认位置是猜测。用户要求：不设置就报错，不要有隐藏默认值。
- 已做：`config_dir()` 只认 `KC_CONFIG_DIR`，未设置（或空值）直接报错退出。为此抽了个纯函数 `config_dir_from` 以便测试，并新增 `requires_kc_config_dir_instead_of_guessing_a_default`。
- 连带更新：`src/main.rs` 帮助栏改为「配置/数据目录，必须设置」，README 的「自定义配置与数据目录」改为「配置与数据目录」并说明不设置会报错。

### ⏸️ B4. `ok`（成功/失败）字段写库却从不读取

- 位置：`src/history.rs`（建表、`upsert` 写入）、`src/history.rs` 的 `load` 只 `SELECT command`、`src/record.rs` 的 `succeeded()`。
- 判定：用户表示失败状态**暂时不处理**，且选择**留着不动**。撤回原判定，不改。
- 备注：`import` 里硬编码 `ok = 1`（导入历史没有成功状态可谈）。等真做界面时再一起处理。

### ⏸️ B5. 删除被刻意做成幂等

- 位置：`src/history.rs` 的 `delete`，以及测试「幂等：再删一次不报错」。
- 判定：删不存在的行视为成功，是本项目的既定语义，**保留**。

### ⏸️ B6. `kc record` 永远返回成功

- 位置：`src/record.rs`：错误只 `eprintln`，恒 `ExitCode::SUCCESS`。
- 判定：这是刻意的设计——记录历史绝不能干扰提示符，且已写进注释与 README。**保留**。

---

## C. 补丁式堆叠与文档漂移

### 🔲 C1. `filter` 在三处各自过滤

- 位置：`src/record.rs`（写入前）、`src/import.rs`（导入前）、`src/history.rs`（读取时再过滤）。
- 问题：同一规则三处实现。读取期过滤是为「filter 添加前就已入库的旧命令」服务，属历史兼容。
- 待定：需要先决定过滤的权威点在哪（写入期 or 读取期），再删另一处。**未改**。

### ✅ C2. 配置解析失败时静默降级

- 原位置：`src/app.rs`（读不到/解析失败 → 默认值，非法正则只打警告并忽略）、`src/record.rs`（`unwrap_or_default()`，配置坏掉时"不过滤照记"）、`src/import.rs`（报错后仍用默认值导入）。
- 问题：配置坏掉时给同一问题两种结果（报错 vs 凑合跑）；`kc record` 尤其危险——filter 失效窗口期会把本该挡住的敏感命令写进 history.db。
- 已做：`Config::load_from` 改为返回 `Result`，四处全部硬报错：
  - 配置文件缺失 → 用默认值（唯一的非错误情形）
  - 读取失败 / TOML 语法错 → 报错
  - `history_limit` 非非负整数 → 报错
  - `filter` 不是字符串数组 / 含非字符串项 / 正则非法 → 报错（`parse_filters` 改为返回 `Result`）
  - `kc record` / `kc import` 不再 `unwrap_or_default()`，配置失败即中止
- 验证：`kc` 在坏配置下退出码 1 且不打印界面；新增 6 个测试覆盖各错误分支。

### 🔲 C3. `apply_delete` 的选中项修补算法

- 位置：`src/tui/app.rs`：删除后按 `filtered_index > 0` 分支调整 `selected`，再 `scroll_offset.min(selected)`、`clamp_scroll()`。
- 问题：列表顺序改成「最旧在上、最新在下」后叠加的补丁；历史上还有一次整块回滚（`e2c47e1 Revert "feat: keep selection inside the middle band while scrolling"`）。
- 待定：应把「删除后选中哪一行」收敛成一个纯函数。**未改**。

### 🔲 C4. PowerShell 去重的双重条件

- 位置：`src/shell_init.rs`：`$global:KcLastId = -1`，条件 `$null -ne $entry -and $entry.Id -ne $global:KcLastId`。
- 问题：为「空回车只重绘提示符」保留 `-1` 哨兵初值与双重判断。
- 待定：可收敛成单一判据。**未改**（脚本行为已稳定，改动需实机验证）。

### ✅ C5. 文档与实现漂移

- 已做：`kc --help` 里 `kc pick --query QUERY` 改为真实的 `--query-env` / `--result-file-env`。
- 遗留：README 只描述 PowerShell 集成，而代码保留非 Windows 路径（Termux）。D 节处理。

### 🔲 C6. 子命令参数解析风格不一致

- 位置：`import` 用位置参数、`record` 用精确一对、`pick` 用手写 flag 循环。
- 问题：三种子命令三套约定。
- 已做（部分）：`pick` 的循环现在缺值即报错，不再静默忽略。
- 待定：三者是否统一成同一种解析方式。**未改**。

---

## D. 功能缺口（本次新发现）

### 🔲 D1. `kc init` 只有 PowerShell，Termux 无法初始化

- 位置：`src/shell_init.rs`：`Some("powershell")` 之外全部报「用法: kc init powershell」并返回 2；`src/main.rs` 与 README 同样只提 PowerShell。
- 问题：用户明确说「只在 Termux、Windows 中使用」，但 Termux 上：
  - 没有 `kc init` 脚本可用，`↑` 键没人绑定到 `kc pick`；
  - `kc import` 只认 PSReadLine 的 `APPDATA` 路径，在 Termux 上必然失败；
  - 因此 Termux 上 kc 目前只能手动跑 TUI。
- 判定：用户选择**先不管，只清理现有代码**。记为功能缺口，本次不动。
- 建议：后续新增 `kc init bash`（prompt 钩子 + 方向键绑定），并让 `kc import` 支持 bash/zsh 历史文件。

---

## 效果汇总（本次实际改动）

| 指标 | 前 | 后 |
| --- | --- | --- |
| `cargo test` | 65 通过 | 67 通过 |
| `cargo clippy -D warnings` | 无告警 | 无告警 |
| 改动行数 | — | +168 / −275（净 −107，不含本轮） |
| 死方案 | `nul` 输出、4 个剪贴板候选、内存去重、重复默认值 | 全部删除 |
| 静默降级 | 家目录缺失→当前目录、`pick` 缺参→忽略、配置目录→猜默认位置 | 均改为报错 |

## 剩余待办

| 优先级 | 条目 | 需要先决定什么 |
| --- | --- | --- |
| 中 | C1 filter 三处过滤 | 权威点放写入期还是读取期 |
| 中 | C6 参数解析风格 | 是否统一 |
| 低 | C3 删除后选中算法 | 收敛成纯函数 |
| 低 | C4 PowerShell 去重条件 | 需实机验证 |
| 低 | B1 的 `path: Option` | 改成非 `Option` |
| 后续 | D1 Termux 集成 | `kc init bash` + `kc import` 支持 bash/zsh |
