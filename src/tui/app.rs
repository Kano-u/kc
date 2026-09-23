use crate::data_paths::history_db;
use crate::history;
use crate::notes::NoteStore;
use ratatui::layout::{Position, Rect};

use super::clipboard::copy_to_clipboard;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Mode {
    Search,
    Note,
    DeleteConfirm,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum DeleteRequest {
    Immediate(String),
    Confirm(String),
}

pub(super) struct App<'a> {
    pub(super) history: Vec<String>,
    pub(super) notes: &'a mut NoteStore,
    pub(super) filtered: Vec<String>,
    pub(super) selected: usize,
    pub(super) scroll_offset: usize,
    pub(super) query: String,
    pub(super) mode: Mode,
    pub(super) note: String,
    pub(super) note_cursor: usize,
    pub(super) note_command: Option<String>,
    pub(super) list_area: Rect,
    pub(super) toolbar_note_area: Rect,
    pub(super) toolbar_copy_area: Rect,
    pub(super) toolbar_delete_area: Rect,
    pub(super) save_area: Rect,
    pub(super) delete_command: Option<String>,
    pub(super) confirm_area: Rect,
    pub(super) cancel_area: Rect,
    pub(super) notice: Option<String>,
}

impl<'a> App<'a> {
    pub(super) fn new(history: &'a [String], notes: &'a mut NoteStore, query: &str) -> Self {
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
        };
        app.refresh();
        app
    }

    pub(super) fn refresh(&mut self) {
        self.rebuild_filtered();
        // 旧命令在上、最新命令在下；默认选中最新的一条。
        self.selected = self.filtered.len().saturating_sub(1);
        self.scroll_offset = 0;
        // 首次构建时列表尺寸还没确定，等渲染阶段拿到真实高度后再校正。
        if self.list_area.height > 0 {
            self.clamp_scroll();
        }
    }

    pub(super) fn rebuild_filtered(&mut self) {
        let (notes_only, raw_query) = split_query(&self.query);
        let query = raw_query.to_lowercase();
        // history 由最旧到最新排列，列表同样按这个顺序铺开：
        // 最旧的在上面，最新的贴着输入行。
        self.filtered = self
            .history
            .iter()
            .filter(|command| matches_query(command, self.notes.get(command), &query, notes_only))
            .cloned()
            .collect();
    }

    pub(super) fn visible_height(&self) -> usize {
        self.list_area.height.max(1) as usize
    }

    pub(super) fn clamp_scroll(&mut self) {
        let height = self.visible_height();
        if self.selected < self.scroll_offset {
            self.scroll_offset = self.selected;
        } else if self.selected >= self.scroll_offset + height {
            self.scroll_offset = self.selected + 1 - height;
        }
    }

    pub(super) fn move_selection(&mut self, delta: isize) {
        if self.filtered.is_empty() {
            return;
        }
        let next = self.selected as isize + delta;
        self.selected = next.clamp(0, self.filtered.len() as isize - 1) as usize;
        self.clamp_scroll();
    }

    /// Returns false when Down moves past the newest command and should exit.
    pub(super) fn move_down(&mut self) -> bool {
        if self.filtered.is_empty() || self.selected + 1 >= self.filtered.len() {
            return false;
        }
        self.move_selection(1);
        true
    }

    pub(super) fn remove_query_char(&mut self) {
        if self.query.pop().is_some() {
            self.notice = None;
            self.refresh();
        }
    }

    /// 列表底部对齐，让最新命令贴着输入行显示。
    pub(super) fn list_top(&self) -> u16 {
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

    pub(super) fn index_at(&self, row: u16) -> Option<usize> {
        let top = self.list_top();
        if row < top {
            return None;
        }
        let index = self.scroll_offset + usize::from(row - top);
        (index < self.filtered.len()).then_some(index)
    }

    pub(super) fn selected_command(&self) -> Option<&str> {
        self.filtered.get(self.selected).map(String::as_str)
    }

    pub(super) fn activate_toolbar(&mut self, position: Position) -> bool {
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

    pub(super) fn delete_request(&self) -> Option<DeleteRequest> {
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

    pub(super) fn request_delete(&mut self) {
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

    pub(super) fn confirm_delete(&mut self) {
        if let Some(command) = self.delete_command.take() {
            self.start_delete(&command);
        }
        self.mode = Mode::Search;
    }

    pub(super) fn cancel_delete(&mut self) {
        self.delete_command = None;
        self.mode = Mode::Search;
    }

    pub(super) fn start_delete(&mut self, command: &str) {
        let result = history_db().and_then(|path| history::delete(&path, command));
        self.apply_delete(command, result);
    }

    /// 删除失败时列表与选中项都不动，只把错误显示出来。
    pub(super) fn apply_delete(&mut self, command: &str, result: Result<(), String>) {
        if let Err(error) = result {
            self.notice = Some(error);
            return;
        }
        let Some(history_index) = self.history.iter().position(|item| item == command) else {
            return;
        };

        let filtered_index = self.selected;
        let scroll_offset = self.scroll_offset;
        self.history.remove(history_index);
        self.rebuild_filtered();
        self.selected = if filtered_index > 0 {
            (filtered_index - 1).min(self.filtered.len().saturating_sub(1))
        } else {
            0
        };
        self.scroll_offset = scroll_offset.min(self.selected);
        self.clamp_scroll();

        if let Err(error) = self.notes.remove(command) {
            self.notice = Some(error);
        }
    }

    pub(super) fn copy_selected_command(&mut self) {
        let Some(command) = self.selected_command() else {
            return;
        };
        self.notice = match copy_to_clipboard(command) {
            Ok(()) => Some("已复制命令".to_owned()),
            Err(error) => Some(error),
        };
    }

    pub(super) fn enter_note_mode(&mut self) {
        if let Some(command) = self.selected_command().map(str::to_owned) {
            self.note = self.notes.get(&command).unwrap_or_default().to_owned();
            self.note_cursor = self.note.chars().count();
            self.note_command = Some(command);
            self.mode = Mode::Note;
        }
    }

    pub(super) fn save_note(&mut self) {
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

#[cfg(test)]
mod tests {
    use super::super::test_support::{notes_for_test, remove_test_notes};
    use super::*;

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
        assert_eq!(app.filtered, vec!["scoop install", "git status"]);
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
    fn selects_the_newest_command_by_default() {
        let history = vec!["older".to_owned(), "newest".to_owned()];
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
        assert_eq!(app.filtered, vec!["git status", "git switch"]);

        app.remove_query_char();
        assert_eq!(app.query, "git");
        assert_eq!(app.filtered, vec!["git status", "git switch"]);
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
        let history = vec!["older".to_owned(), "newest".to_owned()];
        let mut notes = NoteStore::default();
        let mut app = App::new(&history, &mut notes, "");
        assert_eq!(app.selected_command(), Some("newest"));
        assert!(!app.move_down());
        assert_eq!(app.selected_command(), Some("newest"));
    }

    #[test]
    fn down_from_an_older_command_moves_toward_the_newest() {
        let history = vec!["older".to_owned(), "newest".to_owned()];
        let mut notes = NoteStore::default();
        let mut app = App::new(&history, &mut notes, "");
        app.move_selection(-1);
        assert_eq!(app.selected_command(), Some("older"));
        assert!(app.move_down());
        assert_eq!(app.selected_command(), Some("newest"));
    }

    #[test]
    fn deleting_middle_entry_keeps_selection_on_the_same_row() {
        let history = vec![
            "oldest".to_owned(),
            "middle".to_owned(),
            "newest".to_owned(),
        ];
        let mut notes = NoteStore::default();
        let mut app = App::new(&history, &mut notes, "");
        assert_eq!(app.filtered, vec!["oldest", "middle", "newest"]);
        app.selected = 1;
        app.scroll_offset = 1;
        app.apply_delete("middle", Ok(()));
        assert_eq!(app.filtered, vec!["oldest", "newest"]);
        assert_eq!(app.selected, 0);
        assert_eq!(app.selected_command(), Some("oldest"));
        assert_eq!(app.scroll_offset, 0);
    }

    #[test]
    fn deleting_newest_entry_selects_the_next_newest() {
        let history = vec![
            "oldest".to_owned(),
            "middle".to_owned(),
            "newest".to_owned(),
        ];
        let mut notes = NoteStore::default();
        let mut app = App::new(&history, &mut notes, "");
        assert_eq!(app.selected_command(), Some("newest"));
        app.apply_delete("newest", Ok(()));
        assert_eq!(app.filtered, vec!["oldest", "middle"]);
        assert_eq!(app.selected_command(), Some("middle"));
    }

    #[test]
    fn deleting_oldest_entry_keeps_selection_on_the_next_row() {
        let history = vec![
            "oldest".to_owned(),
            "middle".to_owned(),
            "newest".to_owned(),
        ];
        let mut notes = NoteStore::default();
        let mut app = App::new(&history, &mut notes, "");
        app.selected = 0;
        assert_eq!(app.selected_command(), Some("oldest"));
        app.apply_delete("oldest", Ok(()));
        assert_eq!(app.selected_command(), Some("middle"));
    }

    #[test]
    fn failed_delete_keeps_the_list_and_selects_nothing() {
        let history = vec![
            "oldest".to_owned(),
            "middle".to_owned(),
            "newest".to_owned(),
        ];
        let mut notes = NoteStore::default();
        let mut app = App::new(&history, &mut notes, "");
        app.selected = 1;
        app.scroll_offset = 1;
        app.apply_delete("middle", Err("failed".to_owned()));
        assert_eq!(app.filtered, vec!["oldest", "middle", "newest"]);
        assert_eq!(app.selected_command(), Some("middle"));
        assert_eq!(app.scroll_offset, 1);
        assert_eq!(app.notice.as_deref(), Some("failed"));
    }

    #[test]
    fn deleting_a_noted_command_removes_the_note() {
        let history = vec!["dir".to_owned(), "ls".to_owned()];
        let mut notes = notes_for_test("delete-note");
        notes.set("dir", "list files").unwrap();
        let mut app = App::new(&history, &mut notes, "");
        app.selected = 1;

        app.apply_delete("dir", Ok(()));
        assert_eq!(app.filtered, vec!["ls"]);
        assert!(app.notice.is_none());

        let reloaded = NoteStore::load_from(app.notes.path_for_test().unwrap()).unwrap();
        assert_eq!(reloaded.len(), 0, "备注应一并删除");
        remove_test_notes(&notes);
    }
}
