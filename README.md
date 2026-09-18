# kc

基于 Atuin 的命令历史搜索与备注工具。

## 功能

- 向上箭头打开 TUI
- 底部搜索，实时过滤命令和备注
- `Enter` 执行选中命令
- `Tab` / `→` 插入命令但不执行
- `←` 编辑选中命令备注
- 鼠标/触摸：点击选择、滚轮滚动、点击保存
- 备注保存在 `~/.config/kc/data/notes.jsonl`

## 安装

```powershell
cargo install --path .
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

配置文件：`~/.config/kc/config.toml`

```toml
history_limit = 5000
```

## 开发检查

```powershell
cargo test
cargo clippy --all-targets -- -D warnings
cargo build --release
```
