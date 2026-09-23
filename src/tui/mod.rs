use crate::notes::NoteStore;
use crossterm::{
    event::{
        DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers,
        MouseButton, MouseEventKind,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, layout::Position, Terminal};
use serde::Serialize;
use std::io::Stdout;
use std::time::Duration;

mod app;
mod clipboard;
mod render;
mod style;
use app::{App, Mode};
use render::render;

const POLL_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PickAction {
    Insert,
    Execute,
    Cancel,
}

#[derive(Debug, Clone, Serialize)]
pub struct PickResult {
    pub action: PickAction,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
}

impl PickResult {
    fn cancel() -> Self {
        Self {
            action: PickAction::Cancel,
            command: None,
        }
    }
}

pub fn run(query: &str, history: &[String], notes: &mut NoteStore) -> Result<PickResult, String> {
    let mut terminal = setup_terminal()?;
    let result = event_loop(&mut terminal, query, history, notes);
    restore_terminal(&mut terminal)?;
    result
}

fn setup_terminal() -> Result<Terminal<CrosstermBackend<Stdout>>, String> {
    enable_raw_mode().map_err(|error| format!("无法进入终端原始模式: {error}"))?;
    execute!(std::io::stdout(), EnterAlternateScreen, EnableMouseCapture)
        .map_err(|error| format!("无法初始化界面: {error}"))?;
    Terminal::new(CrosstermBackend::new(std::io::stdout()))
        .map_err(|error| format!("无法创建终端界面: {error}"))
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<(), String> {
    disable_raw_mode().map_err(|error| format!("无法恢复终端模式: {error}"))?;
    execute!(std::io::stdout(), LeaveAlternateScreen, DisableMouseCapture)
        .map_err(|error| format!("无法恢复终端界面: {error}"))?;
    terminal
        .show_cursor()
        .map_err(|error| format!("无法恢复光标: {error}"))
}

fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    query: &str,
    history: &[String],
    notes: &mut NoteStore,
) -> Result<PickResult, String> {
    let mut app = App::new(history, notes, query);
    loop {
        terminal
            .draw(|frame| render(frame, &mut app))
            .map_err(|error| format!("渲染界面失败: {error}"))?;

        if !crossterm::event::poll(POLL_INTERVAL)
            .map_err(|error| format!("读取输入失败: {error}"))?
        {
            continue;
        }
        let event = crossterm::event::read().map_err(|error| format!("读取输入失败: {error}"))?;
        match event {
            Event::Key(key) if key.kind == KeyEventKind::Press => match app.mode {
                Mode::Search => match key.code {
                    KeyCode::Up => app.move_selection(-1),
                    KeyCode::Down if !app.move_down() => return Ok(PickResult::cancel()),
                    KeyCode::Down => {}
                    KeyCode::PageUp => app.move_selection(-(app.visible_height() as isize)),
                    KeyCode::PageDown => app.move_selection(app.visible_height() as isize),
                    KeyCode::Enter => {
                        if let Some(command) = app.selected_command() {
                            return Ok(PickResult {
                                action: PickAction::Execute,
                                command: Some(command.to_owned()),
                            });
                        }
                    }
                    KeyCode::Tab | KeyCode::Right => {
                        if let Some(command) = app.selected_command() {
                            return Ok(PickResult {
                                action: PickAction::Insert,
                                command: Some(command.to_owned()),
                            });
                        }
                    }
                    KeyCode::Left => app.enter_note_mode(),
                    KeyCode::Backspace if key.modifiers.contains(KeyModifiers::SHIFT) => {
                        app.request_delete();
                    }
                    KeyCode::Backspace => app.remove_query_char(),
                    KeyCode::Char(character) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                        app.notice = None;
                        app.query.push(character);
                        app.refresh();
                    }
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        return Ok(PickResult::cancel());
                    }
                    KeyCode::Esc => return Ok(PickResult::cancel()),
                    _ => {}
                },
                Mode::Note => match key.code {
                    KeyCode::Enter => app.save_note(),
                    KeyCode::Esc => {
                        app.mode = Mode::Search;
                        app.note_command = None;
                    }
                    KeyCode::Left => {
                        app.note_cursor = app.note_cursor.saturating_sub(1);
                    }
                    KeyCode::Right if app.note_cursor < app.note.chars().count() => {
                        app.note_cursor += 1;
                    }
                    KeyCode::Backspace if app.note_cursor > 0 => {
                        let index = char_to_byte_index(&app.note, app.note_cursor - 1);
                        app.note.remove(index);
                        app.note_cursor -= 1;
                    }
                    KeyCode::Delete if app.note_cursor < app.note.chars().count() => {
                        let index = char_to_byte_index(&app.note, app.note_cursor);
                        app.note.remove(index);
                    }
                    KeyCode::Char(character) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                        let index = char_to_byte_index(&app.note, app.note_cursor);
                        app.note.insert(index, character);
                        app.note_cursor += 1;
                    }
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        return Ok(PickResult::cancel());
                    }
                    _ => {}
                },
                Mode::DeleteConfirm => match key.code {
                    KeyCode::Enter => app.confirm_delete(),
                    KeyCode::Esc => app.cancel_delete(),
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        return Ok(PickResult::cancel());
                    }
                    _ => {}
                },
            },
            Event::Mouse(mouse) => match mouse.kind {
                MouseEventKind::ScrollUp if app.mode == Mode::Search => app.move_selection(-1),
                MouseEventKind::ScrollDown if app.mode == Mode::Search => app.move_selection(1),
                MouseEventKind::Down(MouseButton::Left) => {
                    let position = Position {
                        x: mouse.column,
                        y: mouse.row,
                    };
                    if app.mode == Mode::Note {
                        if app.save_area.contains(position) {
                            app.save_note();
                        }
                    } else if app.mode == Mode::DeleteConfirm {
                        if app.confirm_area.contains(position) {
                            app.confirm_delete();
                        } else if app.cancel_area.contains(position) {
                            app.cancel_delete();
                        }
                    } else if app.activate_toolbar(position) {
                    } else if app.list_area.contains(position) {
                        if let Some(index) = app.index_at(mouse.row) {
                            app.selected = index;
                            app.clamp_scroll();
                        }
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }
}

fn char_to_byte_index(text: &str, char_index: usize) -> usize {
    text.char_indices()
        .nth(char_index)
        .map(|(index, _)| index)
        .unwrap_or(text.len())
}

#[cfg(test)]
mod test_support {
    use crate::notes::NoteStore;

    pub(super) fn notes_for_test(name: &str) -> NoteStore {
        let path =
            std::env::temp_dir().join(format!("kc-tui-notes-{name}-{}.jsonl", std::process::id()));
        let _ = std::fs::remove_file(&path);
        NoteStore::load_from(&path).unwrap()
    }

    pub(super) fn remove_test_notes(notes: &NoteStore) {
        if let Some(path) = notes.path_for_test() {
            let _ = std::fs::remove_file(path);
        }
    }
}
