use crate::notes::NoteStore;
use crossterm::{
    event::{
        DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers,
        MouseButton, MouseEventKind,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Layout, Position, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Clear, List, ListItem, ListState, Paragraph},
    Terminal,
};
use serde::Serialize;
use std::io::Stdout;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Search,
    Note,
}

struct App<'a> {
    history: &'a [String],
    notes: &'a mut NoteStore,
    filtered: Vec<String>,
    selected: usize,
    scroll_offset: usize,
    query: String,
    mode: Mode,
    note: String,
    note_cursor: usize,
    note_command: Option<String>,
    list_area: Rect,
    save_area: Rect,
}

impl<'a> App<'a> {
    fn new(history: &'a [String], notes: &'a mut NoteStore, query: &str) -> Self {
        let mut app = Self {
            history,
            notes,
            filtered: Vec::new(),
            selected: 0,
            scroll_offset: 0,
            query: query.to_owned(),
            mode: Mode::Search,
            note: String::new(),
            note_cursor: 0,
            note_command: None,
            list_area: Rect::default(),
            save_area: Rect::default(),
        };
        app.refresh();
        app
    }

    fn refresh(&mut self) {
        let query = self.query.to_lowercase();
        self.filtered = self
            .history
            .iter()
            .filter(|command| {
                query.is_empty()
                    || command.to_lowercase().contains(&query)
                    || self
                        .notes
                        .get(command)
                        .is_some_and(|note| note.to_lowercase().contains(&query))
            })
            .cloned()
            .collect();
        self.selected = self.selected.min(self.filtered.len().saturating_sub(1));
        self.clamp_scroll();
    }

    fn visible_height(&self) -> usize {
        self.list_area.height.saturating_sub(2).max(1) as usize
    }

    fn clamp_scroll(&mut self) {
        let height = self.visible_height();
        if self.selected < self.scroll_offset {
            self.scroll_offset = self.selected;
        } else if self.selected >= self.scroll_offset + height {
            self.scroll_offset = self.selected + 1 - height;
        }
    }

    fn move_selection(&mut self, delta: isize) {
        if self.filtered.is_empty() {
            return;
        }
        let next = self.selected as isize + delta;
        self.selected = next.clamp(0, self.filtered.len() as isize - 1) as usize;
        self.clamp_scroll();
    }

    fn selected_command(&self) -> Option<&str> {
        self.filtered.get(self.selected).map(String::as_str)
    }

    fn enter_note_mode(&mut self) {
        if let Some(command) = self.selected_command().map(str::to_owned) {
            self.note = self.notes.get(&command).unwrap_or_default().to_owned();
            self.note_cursor = self.note.chars().count();
            self.note_command = Some(command);
            self.mode = Mode::Note;
        }
    }

    fn save_note(&mut self) {
        if let Some(command) = self.note_command.clone() {
            let note = self.note.clone();
            if let Err(error) = self.notes.set(&command, &note) {
                eprintln!("{error}");
            }
        }
        self.mode = Mode::Search;
        self.note_command = None;
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

        let event = crossterm::event::read().map_err(|error| format!("读取输入失败: {error}"))?;
        match event {
            Event::Key(key) if key.kind == KeyEventKind::Press => match app.mode {
                Mode::Search => match key.code {
                    KeyCode::Up => app.move_selection(-1),
                    KeyCode::Down => app.move_selection(1),
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
                    KeyCode::Backspace => {
                        app.query.pop();
                        app.selected = 0;
                        app.scroll_offset = 0;
                        app.refresh();
                    }
                    KeyCode::Char(character) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                        app.query.push(character);
                        app.selected = 0;
                        app.scroll_offset = 0;
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
            },
            Event::Mouse(mouse) => match mouse.kind {
                MouseEventKind::ScrollUp => app.move_selection(-1),
                MouseEventKind::ScrollDown => app.move_selection(1),
                MouseEventKind::Down(MouseButton::Left) => {
                    let position = Position {
                        x: mouse.column,
                        y: mouse.row,
                    };
                    if app.mode == Mode::Note {
                        if app.save_area.contains(position) {
                            app.save_note();
                        }
                    } else if app.list_area.contains(position) {
                        let row = mouse.row.saturating_sub(app.list_area.y + 1) as usize;
                        let index = app.scroll_offset + row;
                        if index < app.filtered.len() {
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

fn render(frame: &mut ratatui::Frame, app: &mut App) {
    let chunks = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(1),
        Constraint::Length(3),
    ])
    .split(frame.area());

    let title = if app.mode == Mode::Note {
        "备注"
    } else {
        "历史"
    };

    let help = if app.mode == Mode::Note {
        "Enter 保存  Esc 取消"
    } else {
        "↑↓ 选择  Enter 执行  Tab/→ 插入  ← 备注  Esc 退出"
    };

    frame.render_widget(
        Paragraph::new(help)
            .block(Block::bordered().title(title))
            .alignment(Alignment::Left),
        chunks[0],
    );

    let visible_height = chunks[1].height.saturating_sub(2).max(1) as usize;
    app.list_area = chunks[1];
    app.clamp_scroll();
    let end = (app.scroll_offset + visible_height).min(app.filtered.len());
    let items: Vec<ListItem> = app.filtered[app.scroll_offset..end]
        .iter()
        .map(|command| {
            let note = app.notes.get(command);
            let content = match note {
                Some(note) if !note.is_empty() => format!("{command}  # {note}"),
                _ => command.clone(),
            };
            ListItem::new(content)
        })
        .collect();

    let local_selected = app.selected.saturating_sub(app.scroll_offset);
    let mut state = ListState::default().with_selected(Some(local_selected));
    frame.render_stateful_widget(
        List::new(items)
            .block(Block::bordered().title("命令历史"))
            .highlight_style(Style::default().fg(Color::Black).bg(Color::Cyan))
            .highlight_symbol("❯ "),
        chunks[1],
        &mut state,
    );

    let query_span = if app.query.is_empty() {
        Span::styled("输入搜索", Style::default().fg(Color::DarkGray))
    } else {
        Span::raw(app.query.clone())
    };
    frame.render_widget(
        Paragraph::new(Line::from(query_span))
            .block(Block::bordered().title("搜索"))
            .alignment(Alignment::Left),
        chunks[2],
    );

    if app.mode == Mode::Note {
        let modal = centered_rect(70, 9, frame.area());
        frame.render_widget(Clear, modal);
        let inner = Block::bordered()
            .title("备注")
            .border_style(Style::default().fg(Color::Cyan));
        frame.render_widget(inner, modal);

        let text_chunks = Layout::vertical([
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(1),
        ])
        .margin(1)
        .split(modal);

        frame.render_widget(
            Paragraph::new(app.note_command.clone().unwrap_or_default())
                .style(Style::default().fg(Color::Gray)),
            text_chunks[0],
        );
        frame.render_widget(
            Paragraph::new(app.note.clone()).style(Style::default().fg(Color::White)),
            text_chunks[1],
        );

        app.save_area = Rect {
            x: modal.x + modal.width.saturating_sub(12),
            y: modal.y + modal.height.saturating_sub(3),
            width: 8,
            height: 1,
        };
        frame.render_widget(
            Paragraph::new("保存")
                .alignment(Alignment::Center)
                .style(
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
            app.save_area,
        );
    }
}

fn centered_rect(percent_x: u16, height: u16, area: Rect) -> Rect {
    let width = area.width.saturating_mul(percent_x) / 100;
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect {
        x,
        y,
        width,
        height,
    }
}
