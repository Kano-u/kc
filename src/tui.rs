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
    layout::{Alignment, Constraint, Layout, Margin, Position, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
    Terminal,
};
use serde::Serialize;
use std::io::Stdout;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

/// 界面装饰使用暗灰，普通文字沿用终端的默认前景色，彩色留给命令本身。
const CHROME: Color = Color::DarkGray;
const COMMAND: Color = Color::Rgb(0x16, 0xc6, 0x0c);
const OPTION: Color = Color::Rgb(0x3a, 0x96, 0xdd);
const MARKER: Color = COMMAND;
const POLL_INTERVAL: Duration = Duration::from_millis(100);

const NOTE_SAVE_HINT: &str = "↵ 保存";
const CURSOR: &str = "❯ ";
// Nerd Font 图标：备注用铅笔、复制用叠页、删除用垃圾桶。
const TOOLBAR_NOTE: &str = "\u{f044} 备注";
const TOOLBAR_COPY: &str = "\u{f0c5} 复制";
const TOOLBAR_DELETE: &str = "\u{f014} 删除";
const TOOLBAR_GAP: u16 = 2;

fn split_query(query: &str) -> (bool, &str) {
    match query.strip_prefix(' ') {
        Some(query) => (true, query),
        None => (false, query),
    }
}

fn matches_query(command: &str, note: Option<&str>, query: &str, notes_only: bool) -> bool {
    if notes_only && note.is_none_or(|note| note.is_empty()) {
        return false;
    }

    query.is_empty()
        || command.to_lowercase().contains(query)
        || note.is_some_and(|note| note.to_lowercase().contains(query))
}

fn spawn_delete(job: DeleteJob, sender: Sender<DeleteOutcome>) {
    thread::spawn(move || {
        let result = crate::history::delete_history(&job.command);
        let _ = sender.send(DeleteOutcome { job, result });
    });
}

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
    DeleteConfirm,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum DeleteRequest {
    Immediate(String),
    Confirm(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DeleteJob {
    command: String,
    history_index: usize,
    filtered_index: usize,
    scroll_offset: usize,
}

struct DeleteOutcome {
    job: DeleteJob,
    result: Result<(), String>,
}

struct App<'a> {
    history: Vec<String>,
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
    toolbar_note_area: Rect,
    toolbar_copy_area: Rect,
    toolbar_delete_area: Rect,
    save_area: Rect,
    delete_command: Option<String>,
    confirm_area: Rect,
    cancel_area: Rect,
    notice: Option<String>,
    delete_sender: Sender<DeleteOutcome>,
    delete_receiver: Receiver<DeleteOutcome>,
}

impl<'a> App<'a> {
    fn new(history: &'a [String], notes: &'a mut NoteStore, query: &str) -> Self {
        let (delete_sender, delete_receiver) = mpsc::channel();
        let mut app = Self {
            history: history.to_vec(),
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
            toolbar_note_area: Rect::default(),
            toolbar_copy_area: Rect::default(),
            toolbar_delete_area: Rect::default(),
            save_area: Rect::default(),
            delete_command: None,
            confirm_area: Rect::default(),
            cancel_area: Rect::default(),
            notice: None,
            delete_sender,
            delete_receiver,
        };
        app.refresh();
        app
    }

    fn refresh(&mut self) {
        self.rebuild_filtered();
        // 旧命令在上、最新命令在下；默认选中最新的一条。
        self.selected = self.filtered.len().saturating_sub(1);
        self.scroll_offset = 0;
        // 首次构建时列表尺寸还没确定，等渲染阶段拿到真实高度后再校正。
        if self.list_area.height > 0 {
            self.clamp_scroll();
        }
    }

    fn rebuild_filtered(&mut self) {
        let (notes_only, raw_query) = split_query(&self.query);
        let query = raw_query.to_lowercase();
        self.filtered = self
            .history
            .iter()
            .filter(|command| matches_query(command, self.notes.get(command), &query, notes_only))
            .cloned()
            .rev()
            .collect();
    }

    fn visible_height(&self) -> usize {
        self.list_area.height.max(1) as usize
    }

    /// 选中项在屏幕上停留的上下留白，让它停在列表的中段（1/5 到 4/5 之间）。
    fn scroll_margin(&self) -> usize {
        self.visible_height() / 5
    }

    /// 选中项停在中段内时滚动位置不动，越过中段才跟着滚动；
    /// 到达最旧或最新命令时列表顶到底，选中项随之贴顶或贴底。
    fn clamp_scroll(&mut self) {
        let height = self.visible_height();
        let margin = self.scroll_margin();
        // 中段即从列表顶部数 margin 行到 height - 1 - margin 行。
        let top_row = margin;
        let bottom_row = height.saturating_sub(1).saturating_sub(margin).max(top_row);
        let highest = self.selected.saturating_sub(top_row);
        let lowest = self.selected.saturating_sub(bottom_row);
        let offset = self.scroll_offset.clamp(lowest, highest);
        self.scroll_offset = offset.min(self.filtered.len().saturating_sub(height));
    }

    fn move_selection(&mut self, delta: isize) {
        if self.filtered.is_empty() {
            return;
        }
        let next = self.selected as isize + delta;
        self.selected = next.clamp(0, self.filtered.len() as isize - 1) as usize;
        self.clamp_scroll();
    }

    /// Returns false when Down moves past the newest command and should exit.
    fn move_down(&mut self) -> bool {
        if self.filtered.is_empty() || self.selected + 1 >= self.filtered.len() {
            return false;
        }
        self.move_selection(1);
        true
    }

    fn remove_query_char(&mut self) {
        if self.query.pop().is_some() {
            self.notice = None;
            self.refresh();
        }
    }

    /// 列表底部对齐，让最新命令贴着输入行显示。
    fn list_top(&self) -> u16 {
        if self.list_area.height == 0 {
            return self.list_area.y;
        }
        let visible = self
            .filtered
            .len()
            .saturating_sub(self.scroll_offset)
            .min(self.visible_height()) as u16;
        self.list_area.bottom().saturating_sub(visible)
    }

    fn index_at(&self, row: u16) -> Option<usize> {
        let top = self.list_top();
        if row < top {
            return None;
        }
        let index = self.scroll_offset + usize::from(row - top);
        (index < self.filtered.len()).then_some(index)
    }

    fn selected_command(&self) -> Option<&str> {
        self.filtered.get(self.selected).map(String::as_str)
    }

    fn activate_toolbar(&mut self, position: Position) -> bool {
        if self.mode != Mode::Search {
            return false;
        }
        if self.toolbar_note_area.contains(position) {
            self.enter_note_mode();
            true
        } else if self.toolbar_copy_area.contains(position) {
            self.copy_selected_command();
            true
        } else if self.toolbar_delete_area.contains(position) {
            self.request_delete();
            true
        } else {
            false
        }
    }

    fn delete_request(&self) -> Option<DeleteRequest> {
        let command = self.selected_command()?.to_owned();
        if self
            .notes
            .get(&command)
            .is_some_and(|note| !note.is_empty())
        {
            Some(DeleteRequest::Confirm(command))
        } else {
            Some(DeleteRequest::Immediate(command))
        }
    }

    fn request_delete(&mut self) {
        self.notice = None;
        match self.delete_request() {
            Some(DeleteRequest::Immediate(command)) => self.start_delete(&command),
            Some(DeleteRequest::Confirm(command)) => {
                self.delete_command = Some(command);
                self.mode = Mode::DeleteConfirm;
            }
            None => {}
        }
    }

    fn confirm_delete(&mut self) {
        if let Some(command) = self.delete_command.take() {
            self.start_delete(&command);
        }
        self.mode = Mode::Search;
    }

    fn cancel_delete(&mut self) {
        self.delete_command = None;
        self.mode = Mode::Search;
    }

    fn start_delete(&mut self, command: &str) {
        if let Some(history_index) = self.history.iter().position(|item| item == command) {
            let filtered_index = self.selected;
            let scroll_offset = self.scroll_offset;
            let job = DeleteJob {
                command: command.to_owned(),
                history_index,
                filtered_index,
                scroll_offset,
            };
            self.remove_local(&job);
            spawn_delete(job, self.delete_sender.clone());
        }
    }

    fn remove_local(&mut self, job: &DeleteJob) {
        self.history.remove(job.history_index);
        let had_previous = job.filtered_index > 0;
        self.rebuild_filtered();
        self.selected = if had_previous {
            (job.filtered_index - 1).min(self.filtered.len().saturating_sub(1))
        } else {
            0
        };
        self.scroll_offset = job.scroll_offset.min(self.selected);
        self.clamp_scroll();
    }

    fn finish_delete(&mut self, outcome: DeleteOutcome) {
        if let Err(error) = outcome.result {
            let index = outcome.job.history_index.min(self.history.len());
            self.history.insert(index, outcome.job.command);
            self.rebuild_filtered();
            self.selected = outcome.job.filtered_index;
            self.scroll_offset = outcome.job.scroll_offset;
            self.clamp_scroll();
            self.notice = Some(error);
        }
    }

    fn copy_selected_command(&mut self) {
        let Some(command) = self.selected_command() else {
            return;
        };
        self.notice = match copy_to_clipboard(command) {
            Ok(()) => Some("已复制命令".to_owned()),
            Err(error) => Some(error),
        };
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
        let outcomes: Vec<DeleteOutcome> = app.delete_receiver.try_iter().collect();
        for outcome in outcomes {
            app.finish_delete(outcome);
        }
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
                    KeyCode::Left if key.modifiers.contains(KeyModifiers::SHIFT) => {}
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

fn copy_to_clipboard(text: &str) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::iter;
        use std::ptr;
        use windows_sys::Win32::System::DataExchange::{
            CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
        };
        use windows_sys::Win32::System::Memory::{
            GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE,
        };
        use windows_sys::Win32::System::Ole::CF_UNICODETEXT;

        let text = text.replace('\n', "\r\n");
        let wide: Vec<u16> = text.encode_utf16().chain(iter::once(0)).collect();
        let bytes = wide
            .len()
            .checked_mul(std::mem::size_of::<u16>())
            .ok_or_else(|| "命令过长，无法复制。".to_owned())?;

        unsafe {
            let handle = GlobalAlloc(GMEM_MOVEABLE, bytes);
            if handle.is_null() {
                return Err("无法分配剪贴板内存。".to_owned());
            }
            let pointer = GlobalLock(handle).cast::<u16>();
            if pointer.is_null() {
                return Err("无法写入剪贴板。".to_owned());
            }
            ptr::copy_nonoverlapping(wide.as_ptr(), pointer, wide.len());
            let _ = GlobalUnlock(handle);

            if OpenClipboard(ptr::null_mut()) == 0 {
                return Err("无法打开系统剪贴板。".to_owned());
            }
            if EmptyClipboard() == 0 {
                let _ = CloseClipboard();
                return Err("无法清空系统剪贴板。".to_owned());
            }
            if SetClipboardData(u32::from(CF_UNICODETEXT), handle).is_null() {
                let _ = CloseClipboard();
                return Err("无法写入系统剪贴板。".to_owned());
            }
            let _ = CloseClipboard();
        }
        Ok(())
    }

    #[cfg(not(windows))]
    {
        use std::io::Write;
        use std::process::{Command, Stdio};

        let candidates: &[(&str, &[&str])] = &[
            ("wl-copy", &[]),
            ("xclip", &["-selection", "clipboard"]),
            ("xsel", &["--clipboard", "--input"]),
            ("termux-clipboard-set", &[]),
            ("pbcopy", &[]),
        ];
        let mut last_error = None;
        for (program, args) in candidates {
            let mut child = match Command::new(program)
                .args(*args)
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
            {
                Ok(child) => child,
                Err(error) => {
                    last_error = Some(error.to_string());
                    continue;
                }
            };
            if let Some(mut stdin) = child.stdin.take() {
                if stdin.write_all(text.as_bytes()).is_err() {
                    continue;
                }
            }
            match child.wait() {
                Ok(status) if status.success() => return Ok(()),
                Ok(status) => last_error = Some(status.to_string()),
                Err(error) => last_error = Some(error.to_string()),
            }
        }
        Err(match last_error {
            Some(error) => format!("复制失败: {error}"),
            None => "复制失败: 未找到可用的剪贴板工具。".to_owned(),
        })
    }
}

fn render(frame: &mut ratatui::Frame, app: &mut App) {
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
            Style::default().fg(CHROME),
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
            Paragraph::new(format!("备注：{note}")).style(Style::default().fg(CHROME)),
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

/// 按 PowerShell 习惯着色：程序名绿色、参数蓝色，其余保持默认前景色。
fn command_line(command: &str) -> Line<'static> {
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
    use ratatui::backend::TestBackend;

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

    #[test]
    fn clips_long_query_from_the_left() {
        assert_eq!(clip_left("abcdef", 3), "def");
        assert_eq!(clip_left("abc", 3), "abc");
        assert_eq!(clip_left("abc", 0), "");
    }

    #[test]
    fn keeps_note_cursor_inside_the_box() {
        assert_eq!(visible_note("hello", 5, 10), ("hello".to_owned(), 5));
        assert_eq!(visible_note("hello", 5, 3), ("lo".to_owned(), 2));
        assert_eq!(visible_note("hello", 1, 3), ("hel".to_owned(), 1));
    }

    fn notes_for_test(name: &str) -> NoteStore {
        let path =
            std::env::temp_dir().join(format!("kc-tui-notes-{name}-{}.jsonl", std::process::id()));
        let _ = std::fs::remove_file(&path);
        NoteStore::load_from(&path).unwrap()
    }

    fn remove_test_notes(notes: &NoteStore) {
        if let Some(path) = notes.path_for_test() {
            let _ = std::fs::remove_file(path);
        }
    }

    #[test]
    fn leading_space_filters_to_commands_with_notes() {
        let history = vec![
            "scoop install".to_owned(),
            "dir".to_owned(),
            "git status".to_owned(),
        ];
        let mut notes = notes_for_test("notes-only");
        notes.set("scoop install", "install package").unwrap();
        notes.set("git status", "check repo").unwrap();

        let app = App::new(&history, &mut notes, " ");
        assert_eq!(app.filtered, vec!["git status", "scoop install"]);
        remove_test_notes(&notes);
    }

    #[test]
    fn leading_space_keeps_search_after_the_space() {
        let history = vec![
            "scoop install".to_owned(),
            "scoop search".to_owned(),
            "dir".to_owned(),
        ];
        let mut notes = notes_for_test("space-query");
        notes.set("scoop install", "install package").unwrap();
        notes.set("dir", "list files").unwrap();

        let app = App::new(&history, &mut notes, " scoop");
        assert_eq!(app.filtered, vec!["scoop install"]);
        remove_test_notes(&notes);
    }

    #[test]
    fn ordinary_search_does_not_require_a_note() {
        let history = vec!["scoop install".to_owned(), "dir".to_owned()];
        let mut notes = notes_for_test("ordinary-query");
        notes.set("dir", "list files").unwrap();

        let app = App::new(&history, &mut notes, "scoop");
        assert_eq!(app.filtered, vec!["scoop install"]);
        remove_test_notes(&notes);
    }

    #[test]
    fn deleting_an_unnoted_command_does_not_ask_for_confirmation() {
        let history = vec!["dir".to_owned(), "ls".to_owned()];
        let mut notes = NoteStore::default();
        let app = App::new(&history, &mut notes, "dir");

        assert_eq!(
            app.delete_request(),
            Some(DeleteRequest::Immediate("dir".to_owned()))
        );
    }

    #[test]
    fn deleting_a_noted_command_requires_confirmation() {
        let history = vec!["dir".to_owned(), "ls".to_owned()];
        let mut notes = notes_for_test("delete-confirm");
        notes.set("dir", "list files").unwrap();
        let mut app = App::new(&history, &mut notes, "dir");

        app.request_delete();
        assert_eq!(app.mode, Mode::DeleteConfirm);
        assert_eq!(app.delete_command.as_deref(), Some("dir"));
        app.cancel_delete();
        assert_eq!(app.mode, Mode::Search);
        assert!(app.delete_command.is_none());
        remove_test_notes(&notes);
    }

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
        let history: Vec<String> = vec![
            "atuin".to_owned(),
            "scoop update".to_owned(),
            "dir".to_owned(),
            "atuin search --format json --limit 5".to_owned(),
            "uv run h.py".to_owned(),
            "atuin search --delete-it-all".to_owned(),
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
        // 测试数据按 Atuin 默认顺序排列：第一条最新。
        // 旧命令在上，最新命令紧贴输入行。
        assert_eq!(lines[0], "\u{f044} 备注  \u{f0c5} 复制  \u{f014} 删除".to_owned());
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
    fn selection_settles_on_the_band_after_scrolling_up() {
        let history: Vec<String> = (0..30).map(|index| format!("cmd {index}")).collect();
        let mut notes = NoteStore::default();
        let mut app = App::new(&history, &mut notes, "");
        let mut terminal = Terminal::new(TestBackend::new(40, 11)).unwrap();
        for _ in 0..10 {
            app.move_selection(-1);
        }
        terminal.draw(|frame| render(frame, &mut app)).unwrap();
        // 列表区域高 8 行、留白 1 行，向上滚时选中项停在第 2 行，不会再往上跑。
        assert_eq!(app.list_area.height, 8);
        assert_eq!(app.selected, 19);
        assert_eq!(app.selected - app.scroll_offset, 1);
        terminal.draw(|frame| render(frame, &mut app)).unwrap();
        assert_eq!(app.selected - app.scroll_offset, 1);
    }

    #[test]
    fn selection_rests_on_the_lower_band_edge_while_scrolling_down() {
        let history: Vec<String> = (0..30).map(|index| format!("cmd {index}")).collect();
        let mut notes = NoteStore::default();
        let mut app = App::new(&history, &mut notes, "");
        let mut terminal = Terminal::new(TestBackend::new(40, 11)).unwrap();
        app.selected = 15;
        terminal.draw(|frame| render(frame, &mut app)).unwrap();
        // 列表区域高 8 行，滚动后选中项停在第 7 行（4/5 边界）。
        assert_eq!(app.list_area.height, 8);
        assert_eq!(app.selected - app.scroll_offset, 6);
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
    fn selects_the_newest_command_by_default() {
        let history = vec!["newest".to_owned(), "older".to_owned()];
        let mut notes = NoteStore::default();
        let app = App::new(&history, &mut notes, "");
        assert_eq!(app.filtered, vec!["older", "newest"]);
        assert_eq!(app.selected_command(), Some("newest"));
    }

    #[test]
    fn backspace_removes_the_last_search_character() {
        let history = vec!["git status".to_owned(), "git switch".to_owned()];
        let mut notes = NoteStore::default();
        let mut app = App::new(&history, &mut notes, "git s");
        app.remove_query_char();
        assert_eq!(app.query, "git ");
        assert_eq!(app.filtered, vec!["git switch", "git status"]);

        app.remove_query_char();
        assert_eq!(app.query, "git");
        assert_eq!(app.filtered, vec!["git switch", "git status"]);
    }

    #[test]
    fn backspace_on_empty_query_is_safe() {
        let history = vec!["dir".to_owned()];
        let mut notes = NoteStore::default();
        let mut app = App::new(&history, &mut notes, "");
        app.remove_query_char();
        assert_eq!(app.query, "");
        assert_eq!(app.filtered, vec!["dir"]);
    }

    #[test]
    fn down_from_the_newest_command_requests_exit() {
        let history = vec!["newest".to_owned(), "older".to_owned()];
        let mut notes = NoteStore::default();
        let mut app = App::new(&history, &mut notes, "");
        assert_eq!(app.selected_command(), Some("newest"));
        assert!(!app.move_down());
        assert_eq!(app.selected_command(), Some("newest"));
    }

    #[test]
    fn down_from_an_older_command_moves_toward_the_newest() {
        let history = vec!["newest".to_owned(), "older".to_owned()];
        let mut notes = NoteStore::default();
        let mut app = App::new(&history, &mut notes, "");
        app.move_selection(-1);
        assert_eq!(app.selected_command(), Some("older"));
        assert!(app.move_down());
        assert_eq!(app.selected_command(), Some("newest"));
    }

    fn app_with_lines(lines: usize, height: u16) -> App<'static> {
        let history: &'static [String] = Box::leak(
            (0..lines)
                .map(|index| format!("cmd {index}"))
                .collect::<Vec<_>>()
                .into_boxed_slice(),
        );
        let notes: &'static mut NoteStore = Box::leak(Box::new(NoteStore::default()));
        let mut app = App::new(history, notes, "");
        app.list_area = Rect {
            x: 0,
            y: 0,
            width: 40,
            height,
        };
        app.selected = 0;
        app.scroll_offset = 0;
        app
    }

    /// 选中项相对列表顶部的行号；列表不满一屏时按底部对齐换算。
    fn selected_row(app: &App) -> usize {
        let height = app.visible_height();
        let offset = app.scroll_offset;
        (height - (app.filtered.len() - offset).min(height)) + (app.selected - offset)
    }

    #[test]
    fn selection_holds_its_row_while_scrolling_through_the_middle() {
        let mut app = app_with_lines(30, 9);
        // 列表高 9 行、留白 1 行：窗口不动时选中项能走到第 8 行，之后再跟着滚动。
        for row in 1..=7 {
            app.move_selection(1);
            assert_eq!(app.scroll_offset, 0);
            assert_eq!(selected_row(&app), row);
        }
        for _ in 0..14 {
            app.move_selection(1);
            assert_eq!(selected_row(&app), 7);
            assert_eq!(app.scroll_offset, app.selected - 7);
        }
    }

    #[test]
    fn selection_parks_on_the_upper_band_edge_when_scrolling_back_up() {
        let mut app = app_with_lines(30, 9);
        app.selected = 15;
        app.clamp_scroll();
        assert_eq!(selected_row(&app), 7);
        for _ in 0..10 {
            app.move_selection(-1);
        }
        // 向上滚时选中项停在第 2 行（1/5 边界），只有走到最旧命令才贴顶。
        assert_eq!(app.selected, 5);
        assert_eq!(selected_row(&app), 1);
        while app.selected > 0 {
            app.move_selection(-1);
            let expected = if app.selected == 0 { 0 } else { 1 };
            assert_eq!(selected_row(&app), expected, "selected={}", app.selected);
        }
    }

    #[test]
    fn oldest_and_newest_still_hug_the_edges() {
        let mut app = app_with_lines(30, 9);
        app.move_selection(-1);
        assert_eq!(app.selected, 0);
        assert_eq!(app.scroll_offset, 0);
        assert_eq!(app.selected - app.scroll_offset, 0);

        let newest = app.filtered.len() - 1;
        for _ in 0..app.filtered.len() {
            app.move_selection(1);
        }
        assert_eq!(app.selected, newest);
        assert_eq!(app.scroll_offset, newest + 1 - 9);
        assert_eq!(app.selected - app.scroll_offset, 8);
    }

    #[test]
    fn short_lists_stay_glued_to_the_bottom() {
        let mut app = app_with_lines(3, 8);
        app.selected = app.filtered.len() - 1;
        app.clamp_scroll();
        assert_eq!(app.scroll_offset, 0);
        assert_eq!(app.list_top(), 5);
        app.move_selection(-1);
        assert_eq!(app.scroll_offset, 0);
    }
    #[test]
    fn deleting_middle_entry_keeps_selection_on_the_same_row() {
        let history = vec![
            "newest".to_owned(),
            "middle".to_owned(),
            "oldest".to_owned(),
        ];
        let mut notes = NoteStore::default();
        let mut app = App::new(&history, &mut notes, "");
        assert_eq!(app.filtered, vec!["oldest", "middle", "newest"]);
        app.selected = 1;
        app.scroll_offset = 1;
        app.remove_local(&DeleteJob {
            command: "middle".to_owned(),
            history_index: 1,
            filtered_index: 1,
            scroll_offset: 1,
        });
        assert_eq!(app.filtered, vec!["oldest", "newest"]);
        assert_eq!(app.selected, 0);
        assert_eq!(app.selected_command(), Some("oldest"));
        assert_eq!(app.scroll_offset, 0);
    }

    #[test]
    fn deleting_newest_entry_selects_the_next_newest() {
        let history = vec![
            "newest".to_owned(),
            "middle".to_owned(),
            "oldest".to_owned(),
        ];
        let mut notes = NoteStore::default();
        let mut app = App::new(&history, &mut notes, "");
        assert_eq!(app.selected_command(), Some("newest"));
        app.remove_local(&DeleteJob {
            command: "newest".to_owned(),
            history_index: 0,
            filtered_index: 2,
            scroll_offset: 0,
        });
        assert_eq!(app.filtered, vec!["oldest", "middle"]);
        assert_eq!(app.selected_command(), Some("middle"));
    }

    #[test]
    fn deleting_oldest_entry_keeps_selection_on_the_next_row() {
        let history = vec![
            "newest".to_owned(),
            "middle".to_owned(),
            "oldest".to_owned(),
        ];
        let mut notes = NoteStore::default();
        let mut app = App::new(&history, &mut notes, "");
        app.selected = 0;
        assert_eq!(app.selected_command(), Some("oldest"));
        app.remove_local(&DeleteJob {
            command: "oldest".to_owned(),
            history_index: 2,
            filtered_index: 0,
            scroll_offset: 0,
        });
        assert_eq!(app.selected_command(), Some("middle"));
    }

    #[test]
    fn failed_delete_rolls_back_history_and_selection() {
        let history = vec![
            "newest".to_owned(),
            "middle".to_owned(),
            "oldest".to_owned(),
        ];
        let mut notes = NoteStore::default();
        let mut app = App::new(&history, &mut notes, "");
        app.selected = 1;
        app.scroll_offset = 1;
        let job = DeleteJob {
            command: "middle".to_owned(),
            history_index: 1,
            filtered_index: 1,
            scroll_offset: 1,
        };
        app.remove_local(&job);
        app.finish_delete(DeleteOutcome {
            job,
            result: Err("failed".to_owned()),
        });
        assert_eq!(app.filtered, vec!["oldest", "middle", "newest"]);
        assert_eq!(app.selected_command(), Some("middle"));
        assert_eq!(app.scroll_offset, 1);
        assert!(app.notice.is_some());
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
