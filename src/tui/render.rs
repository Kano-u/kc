use ratatui::{
    layout::{Alignment, Constraint, Layout, Margin, Position, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
};

use super::app::{App, Mode};
use super::style::{command_line, CHROME, COMMAND};

const MARKER: Color = COMMAND;
const NOTE_SAVE_HINT: &str = "↵ 保存";
const CURSOR: &str = "❯ ";
// Nerd Font 图标：备注用铅笔、复制用叠页、删除用垃圾桶。
const TOOLBAR_NOTE: &str = "\u{f044} 备注";
const TOOLBAR_COPY: &str = "\u{f0c5} 复制";
const TOOLBAR_DELETE: &str = "\u{f014} 删除";
const TOOLBAR_GAP: u16 = 2;

pub(super) fn render(frame: &mut ratatui::Frame, app: &mut App) {
    let search_mode = app.mode == Mode::Search;
    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .split(frame.area());

    // 列表直接铺在终端背景上，没有边框、提示行和外框。
    app.list_area = chunks[2];
    app.clamp_scroll();
    let end = (app.scroll_offset + app.visible_height()).min(app.filtered.len());
    let top = app.list_top();

    for (offset, command) in app.filtered[app.scroll_offset..end].iter().enumerate() {
        // 弹窗打开时列表退到幕后，只有弹窗内的内容保持明亮。
        let selected = search_mode && app.scroll_offset + offset == app.selected;
        let area = Rect {
            y: top + offset as u16,
            height: 1,
            ..chunks[2]
        };
        render_row(frame, app, command, area, selected, search_mode);
    }

    render_toolbar(frame, app, chunks[0]);
    render_divider(frame, chunks[1]);
    if search_mode {
        render_prompt(frame, app, chunks[3]);
    } else {
        if app.mode == Mode::Note {
            render_note_modal(frame, app);
        } else {
            render_delete_confirm(frame, app);
        }
    }
}

fn render_divider(frame: &mut ratatui::Frame, area: Rect) {
    if area.width == 0 {
        return;
    }
    frame.render_widget(
        Block::default()
            .border_type(BorderType::Plain)
            .borders(Borders::BOTTOM)
            .border_style(Style::default().fg(CHROME)),
        area,
    );
}
fn render_toolbar(frame: &mut ratatui::Frame, app: &mut App, area: Rect) {
    if area.width == 0 {
        app.toolbar_note_area = Rect::default();
        app.toolbar_copy_area = Rect::default();
        app.toolbar_delete_area = Rect::default();
        return;
    }

    let note_width = text_width(TOOLBAR_NOTE).min(area.width);
    app.toolbar_note_area = Rect {
        width: note_width,
        ..area
    };

    let copy_x = app.toolbar_note_area.right().saturating_add(TOOLBAR_GAP);
    let copy_width = text_width(TOOLBAR_COPY).min(area.right().saturating_sub(copy_x));
    app.toolbar_copy_area = Rect {
        x: copy_x,
        width: copy_width,
        ..area
    };

    let delete_x = app.toolbar_copy_area.right().saturating_add(TOOLBAR_GAP);
    let delete_width = text_width(TOOLBAR_DELETE).min(area.right().saturating_sub(delete_x));
    app.toolbar_delete_area = Rect {
        x: delete_x,
        width: delete_width,
        ..area
    };

    let style = if app.mode == Mode::Search {
        Style::default()
    } else {
        Style::default().add_modifier(Modifier::DIM)
    };
    frame.render_widget(
        Paragraph::new(TOOLBAR_NOTE).style(style),
        app.toolbar_note_area,
    );
    if app.toolbar_copy_area.width > 0 {
        frame.render_widget(
            Paragraph::new(TOOLBAR_COPY).style(style),
            app.toolbar_copy_area,
        );
    }
    if app.toolbar_delete_area.width > 0 {
        frame.render_widget(
            Paragraph::new(TOOLBAR_DELETE).style(style),
            app.toolbar_delete_area,
        );
    }
}

fn render_row(
    frame: &mut ratatui::Frame,
    app: &App,
    command: &str,
    area: Rect,
    selected: bool,
    focused: bool,
) {
    if area.width == 0 {
        return;
    }
    let mut line = command_line(command);
    if let Some(note) = app.notes.get(command).filter(|note| !note.is_empty()) {
        line.spans.push(Span::styled(
            format!("   {note}"),
            Style::default().fg(Color::Yellow),
        ));
    }
    if selected {
        line = line.style(Style::default().add_modifier(Modifier::REVERSED));
    } else if !focused {
        // Span 自带前景色，只能逐个覆盖才能真的变暗。
        line.spans = line
            .spans
            .into_iter()
            .map(|span| span.style(Style::default().fg(CHROME)))
            .collect();
    }
    frame.render_widget(line, area);
}

fn render_prompt(frame: &mut ratatui::Frame, app: &App, area: Rect) {
    let counter = app
        .notice
        .clone()
        .unwrap_or_else(|| format!("{} / {}", app.filtered.len(), app.history.len()));
    let counter_style = if app.notice.is_some() {
        Style::default().fg(Color::Red)
    } else {
        Style::default().fg(CHROME)
    };
    let counter_width = text_width(&counter);
    let show_counter = area.width > counter_width + 16;
    let prompt_area = Rect {
        width: if show_counter {
            area.width - counter_width - 2
        } else {
            area.width
        },
        ..area
    };

    let marker_width = text_width(CURSOR);
    let text_area = Rect {
        x: prompt_area.x + marker_width,
        width: prompt_area.width.saturating_sub(marker_width),
        ..prompt_area
    };
    if text_area.width == 0 {
        return;
    }

    frame.render_widget(
        Line::from(Span::styled(CURSOR, Style::default().fg(MARKER))),
        prompt_area,
    );

    let (query, cursor) = if app.query.is_empty() {
        (Line::default(), 0)
    } else {
        let visible = clip_left(&app.query, text_area.width.saturating_sub(1));
        let cursor = text_width(&visible);
        (Line::from(Span::raw(visible)), cursor)
    };
    frame.render_widget(query, text_area);
    frame.set_cursor_position(Position {
        x: (text_area.x + cursor).min(text_area.right().saturating_sub(1)),
        y: text_area.y,
    });

    if show_counter {
        frame.render_widget(
            Line::from(Span::styled(counter, counter_style)).alignment(Alignment::Right),
            Rect {
                x: area.right() - counter_width,
                width: counter_width,
                y: area.y,
                height: 1,
            },
        );
    }
}

fn render_note_modal(frame: &mut ratatui::Frame, app: &mut App) {
    let modal = centered_modal(frame.area());
    if modal.width < 8 || modal.height < 4 {
        return;
    }
    frame.render_widget(Clear, modal);
    frame.render_widget(
        Block::bordered()
            .border_type(BorderType::Plain)
            .border_style(Style::default().fg(CHROME))
            .title(Span::styled(" 备注 ", Style::default().fg(CHROME))),
        modal,
    );

    let inner = modal.inner(Margin::new(2, 1));
    if inner.width == 0 || inner.height < 3 {
        return;
    }
    let rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .split(inner);

    let command = app.note_command.clone().unwrap_or_default();
    frame.render_widget(
        Paragraph::new(clip_left(&command, rows[0].width)).style(Style::default().fg(CHROME)),
        rows[0],
    );

    let (visible, cursor) = visible_note(&app.note, app.note_cursor, rows[1].width);
    frame.render_widget(Paragraph::new(visible), rows[1]);
    frame.set_cursor_position(Position {
        x: (rows[1].x + cursor).min(rows[1].right().saturating_sub(1)),
        y: rows[1].y,
    });

    let hint_width = text_width(NOTE_SAVE_HINT).min(rows[3].width);
    app.save_area = Rect {
        x: rows[3].right() - hint_width,
        width: hint_width,
        y: rows[3].y,
        height: 1,
    };
    frame.render_widget(
        Paragraph::new(NOTE_SAVE_HINT)
            .alignment(Alignment::Right)
            .style(Style::default().fg(CHROME)),
        rows[3],
    );
}

fn render_delete_confirm(frame: &mut ratatui::Frame, app: &mut App) {
    let modal = centered_modal(frame.area());
    if modal.width < 24 || modal.height < 6 {
        return;
    }
    frame.render_widget(Clear, modal);
    frame.render_widget(
        Block::bordered()
            .border_type(BorderType::Plain)
            .border_style(Style::default().fg(CHROME))
            .title(Span::styled(" 删除历史 ", Style::default().fg(CHROME))),
        modal,
    );

    let inner = modal.inner(Margin::new(2, 1));
    if inner.width == 0 || inner.height < 4 {
        return;
    }
    let rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .split(inner);

    frame.render_widget(
        Paragraph::new("确定删除这条历史记录？")
            .alignment(Alignment::Center)
            .style(Style::default().fg(CHROME)),
        rows[0],
    );
    let command = app.delete_command.clone().unwrap_or_default();
    frame.render_widget(
        Paragraph::new(clip_left(&command, rows[1].width)).style(Style::default().fg(CHROME)),
        rows[1],
    );

    if let Some(note) = app.notes.get(&command).filter(|note| !note.is_empty()) {
        frame.render_widget(
            Paragraph::new(format!("备注：{note}")).style(Style::default().fg(Color::Yellow)),
            rows[2],
        );
    }

    let buttons =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).split(rows[3]);
    app.cancel_area = buttons[0];
    app.confirm_area = buttons[1];
    frame.render_widget(
        Paragraph::new("取消")
            .alignment(Alignment::Center)
            .style(Style::default().fg(CHROME)),
        app.cancel_area,
    );
    frame.render_widget(
        Paragraph::new("确定")
            .alignment(Alignment::Center)
            .style(Style::default().fg(COMMAND)),
        app.confirm_area,
    );
}

/// 只保留字符串右侧，让光标始终留在可见范围内。
fn clip_left(text: &str, width: u16) -> String {
    let width = width as usize;
    if width == 0 {
        return String::new();
    }
    let count = text.chars().count();
    if count <= width {
        return text.to_owned();
    }
    text.chars().skip(count - width).collect()
}

/// 返回可视文本和光标在其中的列位置。
fn visible_note(text: &str, cursor: usize, width: u16) -> (String, u16) {
    if width == 0 {
        return (String::new(), 0);
    }
    let width = width as usize;
    let characters: Vec<char> = text.chars().collect();
    let cursor = cursor.min(characters.len());
    // 光标占用最后一格，窗口末端与光标对齐。
    let offset = cursor.saturating_sub(width.saturating_sub(1));
    let end = (offset + width).min(characters.len());
    let visible: String = characters[offset..end].iter().collect();
    let before: String = characters[offset..cursor].iter().collect();
    let column = text_width(&before).min(width.saturating_sub(1) as u16);
    (visible, column)
}

fn text_width(text: &str) -> u16 {
    Span::raw(text).width() as u16
}

fn centered_modal(area: Rect) -> Rect {
    let width = area.width.saturating_sub(8).clamp(24, 72).min(area.width);
    let height = 7.min(area.height);
    Rect {
        x: area.x + (area.width.saturating_sub(width)) / 2,
        y: area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{notes_for_test, remove_test_notes};
    use super::*;
    use crate::notes::NoteStore;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    #[test]
    fn renders_delete_confirmation_for_noted_command() {
        let history = vec!["dir".to_owned()];
        let mut notes = notes_for_test("delete-modal");
        notes.set("dir", "list files").unwrap();
        let mut app = App::new(&history, &mut notes, "");
        app.request_delete();

        let mut terminal = Terminal::new(TestBackend::new(50, 10)).unwrap();
        terminal.draw(|frame| render(frame, &mut app)).unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        let compact: String = text.split_whitespace().collect();
        assert!(compact.contains("确定删除这条历史记录？"), "{text}");
        assert!(compact.contains("取消确定"), "{text}");
        remove_test_notes(&notes);
    }

    fn screen(width: u16, height: u16, query: &str) -> Vec<String> {
        // history 由最旧到最新排列。
        let history: Vec<String> = vec![
            "atuin search --delete-it-all".to_owned(),
            "uv run h.py".to_owned(),
            "atuin search --format json --limit 5".to_owned(),
            "dir".to_owned(),
            "scoop update".to_owned(),
            "atuin".to_owned(),
        ];
        let mut notes = NoteStore::default();
        let mut app = App::new(&history, &mut notes, query);
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| render(frame, &mut app)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        buffer
            .content
            .chunks(width as usize)
            .map(|row| {
                let mut text = String::new();
                let mut wide_tail = false;
                for cell in row.iter() {
                    if wide_tail {
                        wide_tail = false;
                        continue;
                    }
                    let symbol = cell.symbol();
                    text.push_str(symbol);
                    wide_tail = Span::raw(symbol).width() > 1;
                }
                text.trim_end().to_owned()
            })
            .collect()
    }

    #[test]
    fn shows_history_bottom_up_with_newest_next_to_the_prompt() {
        let lines = screen(50, 8, "");
        // 最旧命令在上，最新命令紧贴输入行。
        assert_eq!(
            lines[0],
            "\u{f044} 备注  \u{f0c5} 复制  \u{f014} 删除".to_owned()
        );
        assert_eq!(lines[1], "─".repeat(50));
        assert_eq!(lines[2], "uv run h.py".to_owned(), "{lines:?}");
        assert_eq!(lines[3], "atuin search --format json --limit 5".to_owned());
        assert_eq!(lines[4], "dir".to_owned());
        assert_eq!(lines[5], "scoop update".to_owned());
        assert_eq!(lines[6], "atuin".to_owned());
        assert!(!lines
            .iter()
            .any(|line| line.contains("选择") || line.contains("esc")));
        assert!(lines[7].ends_with("6 / 6"));
    }

    #[test]
    fn keeps_the_newest_entries_at_the_bottom_when_scrolling() {
        let lines = screen(40, 4, "json");
        // 过滤命中一条时仍靠底部显示，紧贴输入行。
        assert_eq!(lines[2], "atuin search --format json --limit 5".to_owned());
        assert!(lines[3].starts_with("❯ json"));
    }

    #[test]
    fn toolbar_actions_follow_the_selected_command() {
        let history = vec!["dir".to_owned()];
        let mut notes = notes_for_test("toolbar-actions");
        notes.set("dir", "list files").unwrap();
        let mut app = App::new(&history, &mut notes, "");
        let mut terminal = Terminal::new(TestBackend::new(40, 8)).unwrap();
        terminal.draw(|frame| render(frame, &mut app)).unwrap();

        assert_eq!(terminal.backend().buffer()[(0, 0)].fg, Color::Reset);

        let note = Position {
            x: app.toolbar_note_area.x,
            y: app.toolbar_note_area.y,
        };
        assert!(app.activate_toolbar(note));
        assert_eq!(app.mode, Mode::Note);
        app.mode = Mode::Search;
        app.note_command = None;

        terminal.draw(|frame| render(frame, &mut app)).unwrap();
        let delete = Position {
            x: app.toolbar_delete_area.x,
            y: app.toolbar_delete_area.y,
        };
        assert!(app.activate_toolbar(delete));
        assert_eq!(app.mode, Mode::DeleteConfirm);
        assert_eq!(app.delete_command.as_deref(), Some("dir"));
        remove_test_notes(&notes);
    }

    #[test]
    fn renders_note_modal_over_the_list() {
        let history = vec!["cargo test".to_owned()];
        let mut notes = NoteStore::default();
        let mut app = App::new(&history, &mut notes, "");
        app.enter_note_mode();
        app.note = "运行测试".to_owned();
        app.note_cursor = app.note.chars().count();

        let mut terminal = Terminal::new(TestBackend::new(46, 9)).unwrap();
        terminal.draw(|frame| render(frame, &mut app)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let text: String = buffer.content.iter().map(|cell| cell.symbol()).collect();
        let compact: String = text.split_whitespace().collect();
        assert!(text.contains("cargo test"), "modal did not render: {text}");
        assert!(compact.contains("运行测试"), "note text missing: {text}");
        assert_eq!(app.mode, Mode::Note);
    }
    #[test]
    fn dims_list_while_note_modal_is_open() {
        let history = vec!["atuin search".to_owned()];
        let mut notes = NoteStore::default();
        let mut app = App::new(&history, &mut notes, "");
        let mut terminal = Terminal::new(TestBackend::new(46, 9)).unwrap();
        terminal.draw(|frame| render(frame, &mut app)).unwrap();
        // 单条命令底部对齐，紧贴输入行。
        let before = terminal.backend().buffer()[(0, 7)].fg;
        assert_eq!(before, COMMAND);

        app.enter_note_mode();
        terminal.draw(|frame| render(frame, &mut app)).unwrap();
        let dimmed = terminal.backend().buffer()[(0, 7)].fg;
        assert_eq!(dimmed, CHROME);
        assert_eq!(app.mode, Mode::Note);
    }
}
