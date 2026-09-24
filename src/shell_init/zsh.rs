/// zsh 版本（Termux/WSL）。注释写英文并非编码需要，而是避免再多一条要验证的规则。
///
/// 两个钩子分工是必须的：`preexec` 的 `$1` 才是用户输入原文，状态则要在
/// `precmd` 取。钩子顺序不影响 `$?`：zsh 在每个 hook 函数进出时都会保存并恢复它，
/// 所以不管 `_kc_precmd` 排在哪里，读到的都是上一条命令的真实状态。
/// 仍然独立取状态的原因是 `preexec` 里 `$?` 指的是之前的东西，不是刚要跑的命令。
pub(super) const ZSH: &str = r##"# Keep this block as the last statement of .zshrc.
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
"##;

#[cfg(test)]
mod tests {
    use super::super::{main, rendered_zsh, ZSH};
    use std::process::ExitCode;

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
}
