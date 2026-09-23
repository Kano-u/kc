use ratatui::{
    style::{Color, Style},
    text::{Line, Span},
};

/// 界面装饰使用暗灰，普通文字沿用终端的默认前景色，彩色留给命令本身。
pub(super) const CHROME: Color = Color::DarkGray;
pub(super) const COMMAND: Color = Color::Rgb(0x16, 0xc6, 0x0c);
pub(super) const OPTION: Color = Color::Rgb(0x3a, 0x96, 0xdd);

pub(super) fn command_line(command: &str) -> Line<'static> {
    let mut spans = Vec::new();
    let mut expect_command = true;
    for (gap, token) in tokenize(command) {
        if gap {
            spans.push(Span::raw(token.to_owned()));
            continue;
        }
        let style = if is_operator(token) {
            expect_command = true;
            Style::default().fg(CHROME)
        } else if expect_command {
            expect_command = false;
            Style::default().fg(COMMAND)
        } else if is_option(token) {
            Style::default().fg(OPTION)
        } else {
            Style::default()
        };
        spans.push(Span::styled(token.to_owned(), style));
    }
    Line::from(spans)
}

/// 把命令切成空白和词语两类片段，保证原样还原。
fn tokenize(command: &str) -> Vec<(bool, &str)> {
    let mut tokens: Vec<(bool, &str)> = Vec::new();
    let mut start = 0;
    let mut current: Option<bool> = None;
    for (index, character) in command.char_indices() {
        let gap = character.is_whitespace();
        match current {
            Some(state) if state == gap => {}
            Some(state) => {
                tokens.push((state, &command[start..index]));
                start = index;
                current = Some(gap);
            }
            None => current = Some(gap),
        }
    }
    if let Some(state) = current {
        tokens.push((state, &command[start..]));
    }
    tokens
}

fn is_operator(token: &str) -> bool {
    token
        .chars()
        .any(|character| matches!(character, '|' | '&' | ';' | '<' | '>'))
        && token
            .chars()
            .all(|character| matches!(character, '|' | '&' | ';' | '<' | '>' | '0'..='9'))
}

fn is_option(token: &str) -> bool {
    token.starts_with('-') && token.chars().count() > 1
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Style;

    fn rendered(command: &str) -> Vec<(String, Style)> {
        command_line(command)
            .spans
            .into_iter()
            .map(|span| (span.content.into_owned(), span.style))
            .collect()
    }
    #[test]
    fn uses_the_configured_command_and_option_colors() {
        assert_eq!(COMMAND, Color::Rgb(0x16, 0xc6, 0x0c));
        assert_eq!(OPTION, Color::Rgb(0x3a, 0x96, 0xdd));
    }

    #[test]
    fn colors_program_name_and_options() {
        let spans = rendered("atuin search --format json --limit 5");
        assert_eq!(spans[0].0, "atuin");
        assert_eq!(spans[0].1.fg, Some(COMMAND));
        assert_eq!(spans[2].0, "search");
        assert_eq!(spans[2].1.fg, None);
        assert_eq!(spans[4].0, "--format");
        assert_eq!(spans[4].1.fg, Some(OPTION));
        assert_eq!(spans[8].0, "--limit");
        assert_eq!(spans[8].1.fg, Some(OPTION));
        assert_eq!(spans[10].0, "5");
        assert_eq!(spans[10].1.fg, None);
    }

    #[test]
    fn keeps_whitespace_and_highlights_pipes() {
        let spans = rendered("dir | more");
        let text: String = spans.iter().map(|span| span.0.clone()).collect();
        assert_eq!(text, "dir | more");
        assert_eq!(spans[2].1.fg, Some(CHROME));
        assert_eq!(spans[4].0, "more");
        assert_eq!(spans[4].1.fg, Some(COMMAND));
    }

    #[test]
    fn treats_single_dash_as_argument() {
        let spans = rendered("cat -");
        assert_eq!(spans[2].0, "-");
        assert_eq!(spans[2].1.fg, None);
    }
}
