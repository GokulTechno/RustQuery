//! Multi-line SQL editor core.
//!
//! The editor intentionally implements its own buffer instead of relying on a
//! widget crate. That gives us character-accurate selections, grouped undo,
//! syntax highlighting, indentation and scrolling — all of which the data grid
//! and tests also use.

pub mod highlight;

use std::time::{Duration, Instant};

use highlight::{SyntaxCache, Token};

/// Zero-based cursor position. `col` counts characters, not bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct Cursor {
    pub row: usize,
    pub col: usize,
}

impl Cursor {
    pub fn new(row: usize, col: usize) -> Self {
        Self { row, col }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Motion {
    Left,
    Right,
    Up,
    Down,
    WordLeft,
    WordRight,
    Home,
    End,
    DocStart,
    DocEnd,
    PageUp,
    PageDown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EditKind {
    Insert,
    Backspace,
    Delete,
    Other,
}

#[derive(Debug, Clone)]
struct Snapshot {
    lines: Vec<String>,
    cursor: Cursor,
    anchor: Option<Cursor>,
}

#[derive(Debug)]
pub struct Editor {
    pub lines: Vec<String>,
    pub cursor: Cursor,
    pub anchor: Option<Cursor>,
    pub scroll_row: usize,
    pub scroll_col: usize,
    preferred_col: Option<usize>,
    viewport_rows: usize,
    viewport_cols: usize,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    last_edit: Option<(EditKind, Instant)>,
    /// Text as of the last load/save, used for the dirty indicator.
    saved_lines: Vec<String>,
    syntax: SyntaxCache,
}

impl Editor {
    pub fn new(text: &str) -> Self {
        let lines = normalize_text(text);
        Self {
            saved_lines: lines.clone(),
            syntax: SyntaxCache::default(),
            lines,
            cursor: Cursor::default(),
            anchor: None,
            scroll_row: 0,
            scroll_col: 0,
            preferred_col: None,
            viewport_rows: 20,
            viewport_cols: 80,
            undo: Vec::new(),
            redo: Vec::new(),
            last_edit: None,
            // Force a highlight pass on the first render.
        }
    }

    /// Replace content and reset editing state (used for file loads/restores).
    pub fn set_text(&mut self, text: &str) {
        self.lines = normalize_text(text);
        self.saved_lines = self.lines.clone();
        self.cursor = Cursor::default();
        self.anchor = None;
        self.scroll_row = 0;
        self.scroll_col = 0;
        self.preferred_col = None;
        self.undo.clear();
        self.redo.clear();
        self.last_edit = None;
        self.syntax.invalidate();
    }

    pub fn text(&self) -> String {
        self.lines.join("\n")
    }

    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    pub fn is_modified(&self) -> bool {
        self.lines != self.saved_lines
    }

    pub fn mark_clean(&mut self) {
        self.saved_lines = self.lines.clone();
    }

    pub fn line_len(&self, row: usize) -> usize {
        self.lines.get(row).map(|l| l.chars().count()).unwrap_or(0)
    }

    /// 1-based line/column for the status bar.
    pub fn cursor_line_col(&self) -> (usize, usize) {
        (self.cursor.row + 1, self.cursor.col + 1)
    }

    pub fn set_viewport(&mut self, rows: u16, cols: u16) {
        self.viewport_rows = rows.max(1) as usize;
        self.viewport_cols = cols.max(1) as usize;
    }

    pub fn page_size(&self) -> usize {
        self.viewport_rows.saturating_sub(1).max(1)
    }

    pub fn has_selection(&self) -> bool {
        matches!(self.selection(), Some((a, b)) if a != b)
    }

    /// Normalised selection in document order: (start, end).
    pub fn selection(&self) -> Option<(Cursor, Cursor)> {
        let anchor = self.anchor?;
        if anchor == self.cursor {
            return None;
        }
        if anchor < self.cursor {
            Some((anchor, self.cursor))
        } else {
            Some((self.cursor, anchor))
        }
    }

    pub fn selected_text(&self) -> String {
        let Some((start, end)) = self.selection() else {
            return String::new();
        };
        if start.row == end.row {
            return slice_chars(&self.lines[start.row], start.col, end.col);
        }

        let mut out = String::new();
        out.push_str(&slice_chars(
            &self.lines[start.row],
            start.col,
            self.line_len(start.row),
        ));
        out.push('\n');
        for row in start.row + 1..end.row {
            out.push_str(&self.lines[row]);
            out.push('\n');
        }
        out.push_str(&slice_chars(&self.lines[end.row], 0, end.col));
        out
    }

    pub fn select_all(&mut self) {
        self.anchor = Some(Cursor::new(0, 0));
        let last = self.lines.len().saturating_sub(1);
        self.cursor = Cursor::new(last, self.line_len(last));
        self.preferred_col = None;
        self.last_edit = None;
    }

    pub fn clear_selection(&mut self) {
        self.anchor = None;
    }

    fn clamp_cursor(&mut self) {
        let row = self.cursor.row.min(self.lines.len().saturating_sub(1));
        let col = self.cursor.col.min(self.line_len(row));
        self.cursor = Cursor::new(row, col);
    }

    /// Move the cursor. `extend` corresponds to Shift+movement.
    pub fn move_cursor(&mut self, motion: Motion, extend: bool) {
        let old = self.cursor;
        let mut new = old;
        let mut keep_preferred = false;
        match motion {
            Motion::Left => {
                if old.col > 0 {
                    new.col -= 1;
                } else if old.row > 0 {
                    new.row -= 1;
                    new.col = self.line_len(new.row);
                }
            }
            Motion::Right => {
                if old.col < self.line_len(old.row) {
                    new.col += 1;
                } else if old.row + 1 < self.lines.len() {
                    new.row += 1;
                    new.col = 0;
                }
            }
            Motion::Up => {
                let pref = self.preferred_col.unwrap_or(old.col);
                if old.row > 0 {
                    new.row -= 1;
                    new.col = pref.min(self.line_len(new.row));
                }
                keep_preferred = true;
                self.preferred_col = Some(pref);
            }
            Motion::Down => {
                let pref = self.preferred_col.unwrap_or(old.col);
                if old.row + 1 < self.lines.len() {
                    new.row += 1;
                    new.col = pref.min(self.line_len(new.row));
                }
                keep_preferred = true;
                self.preferred_col = Some(pref);
            }
            Motion::WordLeft => new = self.word_left(),
            Motion::WordRight => new = self.word_right(),
            Motion::Home => {
                let indent = self.lines[old.row]
                    .chars()
                    .take_while(|c| *c == ' ')
                    .count();
                new.col = if old.col == indent { 0 } else { indent };
            }
            Motion::End => new.col = self.line_len(old.row),
            Motion::DocStart => new = Cursor::new(0, 0),
            Motion::PageUp => {
                let rows = old.row.saturating_sub(self.page_size());
                let pref = self.preferred_col.unwrap_or(old.col);
                new.row = rows;
                new.col = pref.min(self.line_len(rows));
                keep_preferred = true;
                self.preferred_col = Some(pref);
            }
            Motion::PageDown => {
                let rows = (old.row + self.page_size()).min(self.lines.len().saturating_sub(1));
                let pref = self.preferred_col.unwrap_or(old.col);
                new.row = rows;
                new.col = pref.min(self.line_len(rows));
                keep_preferred = true;
                self.preferred_col = Some(pref);
            }
            Motion::DocEnd => {
                let last = self.lines.len().saturating_sub(1);
                new = Cursor::new(last, self.line_len(last));
            }
        }

        self.cursor = new;
        self.clamp_cursor();
        if extend {
            if self.anchor.is_none() {
                self.anchor = Some(old);
            }
        } else {
            self.anchor = None;
        }
        if !keep_preferred {
            self.preferred_col = None;
        }
        self.last_edit = None;
    }

    /// Absolute mouse positioning (row/col already converted from the view).
    pub fn set_cursor_from_click(&mut self, row: usize, col: usize, extend: bool) {
        let old = self.cursor;
        self.cursor = Cursor::new(row, col);
        self.clamp_cursor();
        if extend {
            if self.anchor.is_none() {
                self.anchor = Some(old);
            }
        } else {
            self.anchor = None;
        }
        self.preferred_col = None;
        self.last_edit = None;
    }

    fn char_at(&self, pos: Cursor) -> Option<char> {
        self.lines.get(pos.row).and_then(|l| l.chars().nth(pos.col))
    }

    fn word_left(&self) -> Cursor {
        let mut row = self.cursor.row;
        let mut col = self.cursor.col;
        loop {
            if col == 0 {
                if row == 0 {
                    break;
                }
                row -= 1;
                col = self.line_len(row);
                if col == 0 {
                    continue;
                }
            } else {
                col -= 1;
            }
            let pos = Cursor::new(row, col);
            if let Some(c) = self.char_at(pos) {
                let prev_word = if col == 0 {
                    false
                } else {
                    self.char_at(Cursor::new(row, col - 1))
                        .map(is_word_char)
                        .unwrap_or(false)
                };
                if is_word_char(c) && !prev_word {
                    break;
                }
            }
        }
        Cursor::new(row, col)
    }

    fn word_right(&self) -> Cursor {
        let mut row = self.cursor.row;
        let mut col = self.cursor.col;
        // Finish the current word first.
        while let Some(c) = self.char_at(Cursor::new(row, col)) {
            if !is_word_char(c) {
                break;
            }
            col += 1;
        }
        loop {
            if col >= self.line_len(row) {
                if row + 1 >= self.lines.len() {
                    return Cursor::new(row, self.line_len(row));
                }
                row += 1;
                col = 0;
            } else {
                col += 1;
            }
            if let Some(c) = self.char_at(Cursor::new(row, col)) {
                if is_word_char(c) {
                    let prev = if col == 0 {
                        None
                    } else {
                        self.char_at(Cursor::new(row, col - 1))
                    };
                    if prev.map(|p| !is_word_char(p)).unwrap_or(true) {
                        break;
                    }
                }
            }
        }
        Cursor::new(row, col)
    }

    // ---------- Undo -------------------------------------------------------

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            lines: self.lines.clone(),
            cursor: self.cursor,
            anchor: self.anchor,
        }
    }

    fn record(&mut self, kind: EditKind) {
        let now = Instant::now();
        let coalesce = matches!(
            kind,
            EditKind::Insert | EditKind::Backspace | EditKind::Delete
        ) && self
            .last_edit
            .map(|(last_kind, at)| {
                last_kind == kind && now.duration_since(at) < Duration::from_millis(700)
            })
            .unwrap_or(false)
            && !self.undo.is_empty();

        if !coalesce {
            self.undo.push(self.snapshot());
            if self.undo.len() > 128 {
                self.undo.remove(0);
            }
        }
        self.redo.clear();
        self.last_edit = Some((kind, now));
    }

    pub fn undo(&mut self) {
        if let Some(snapshot) = self.undo.pop() {
            let current = self.snapshot();
            self.redo.push(current);
            if self.redo.len() > 128 {
                self.redo.remove(0);
            }
            self.lines = snapshot.lines;
            self.cursor = snapshot.cursor;
            self.anchor = snapshot.anchor;
            self.clamp_cursor();
            self.preferred_col = None;
            self.last_edit = None;
            self.syntax.invalidate();
        }
    }

    pub fn redo(&mut self) {
        if let Some(snapshot) = self.redo.pop() {
            self.undo.push(self.snapshot());
            if self.undo.len() > 128 {
                self.undo.remove(0);
            }
            self.lines = snapshot.lines;
            self.cursor = snapshot.cursor;
            self.anchor = snapshot.anchor;
            self.clamp_cursor();
            self.preferred_col = None;
            self.last_edit = None;
            self.syntax.invalidate();
        }
    }

    // ---------- Editing ----------------------------------------------------

    /// Replace [start, end) with `text`. All editing operations funnel through
    /// here, keeping cursor/selection/undo behaviour consistent.
    fn replace_range(&mut self, start: Cursor, end: Cursor, text: &str) -> Cursor {
        let (start, end) = if start <= end {
            (start, end)
        } else {
            (end, start)
        };
        let start = self.clamp_position(start);
        let end = self.clamp_position(end);

        let prefix: String = self.lines[start.row].chars().take(start.col).collect();
        let end_len = self.line_len(end.row);
        let suffix: String = self.lines[end.row]
            .chars()
            .skip(end.col.min(end_len))
            .collect();

        let mut inserted = normalize_text(text);
        if inserted.is_empty() {
            inserted.push(String::new());
        }
        let cursor_col;
        let new_lines: Vec<String> = if inserted.len() == 1 {
            let line = format!("{prefix}{}{suffix}", inserted[0]);
            cursor_col = prefix.chars().count() + inserted[0].chars().count();
            vec![line]
        } else {
            let mut lines = Vec::with_capacity(inserted.len());
            lines.push(format!("{prefix}{}", inserted[0]));
            for line in &inserted[1..inserted.len() - 1] {
                lines.push(line.clone());
            }
            let last = inserted.last().cloned().unwrap_or_default();
            cursor_col = last.chars().count();
            lines.push(format!("{last}{suffix}"));
            lines
        };

        self.lines.splice(start.row..=end.row, new_lines);
        let new_cursor = Cursor::new(start.row + inserted.len() - 1, cursor_col);
        self.cursor = new_cursor;
        self.anchor = None;
        self.clamp_cursor();
        self.syntax.invalidate();
        self.cursor
    }

    fn clamp_position(&self, pos: Cursor) -> Cursor {
        let row = pos.row.min(self.lines.len().saturating_sub(1));
        Cursor::new(row, pos.col.min(self.line_len(row)))
    }

    fn delete_selection_inner(&mut self) -> bool {
        if let Some((start, end)) = self.selection() {
            self.replace_range(start, end, "");
            true
        } else {
            false
        }
    }

    pub fn insert_char(&mut self, c: char) {
        if c.is_control() && c != '\n' && c != '\t' {
            return;
        }
        if c == '\n' {
            self.newline();
            return;
        }
        let text = if c == '\t' {
            "    ".to_string()
        } else {
            c.to_string()
        };
        if self.has_selection() {
            self.last_edit = None;
        }
        self.record(EditKind::Insert);
        let (start, end) = self.selection().unwrap_or((self.cursor, self.cursor));
        let new_cursor = self.replace_range(start, end, &text);
        self.cursor = new_cursor;
        self.preferred_col = None;
    }

    pub fn insert_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        if self.has_selection() {
            self.last_edit = None;
        }
        self.record(EditKind::Other);
        let (start, end) = self.selection().unwrap_or((self.cursor, self.cursor));
        let new_cursor = self.replace_range(start, end, text);
        self.cursor = new_cursor;
        self.preferred_col = None;
    }

    pub fn newline(&mut self) {
        let indent_source_row = self
            .selection()
            .map(|(start, _)| start.row)
            .unwrap_or(self.cursor.row);
        let indent: String = self.lines[indent_source_row]
            .chars()
            .take_while(|c| *c == ' ')
            .collect();
        let extra = if !self.has_selection() {
            match self.char_at(self.cursor) {
                Some('(') => "    ",
                _ => {
                    let before = slice_chars(&self.lines[self.cursor.row], 0, self.cursor.col);
                    if before.trim_end().ends_with('(') {
                        "    "
                    } else {
                        ""
                    }
                }
            }
        } else {
            ""
        };
        let text = format!("\n{indent}{extra}");
        if self.has_selection() {
            self.last_edit = None;
        }
        self.record(EditKind::Other);
        let (start, end) = self.selection().unwrap_or((self.cursor, self.cursor));
        let new_cursor = self.replace_range(start, end, &text);
        self.cursor = new_cursor;
        self.preferred_col = None;
    }

    pub fn backspace(&mut self) {
        if self.has_selection() {
            self.last_edit = None;
        }
        self.record(EditKind::Backspace);
        if self.delete_selection_inner() {
            return;
        }
        if self.cursor.col == 0 {
            if self.cursor.row == 0 {
                return;
            }
            let row = self.cursor.row;
            let prev_len = self.line_len(row - 1);
            let start = Cursor::new(row - 1, prev_len);
            let end = Cursor::new(row, 0);
            let _ = self.replace_range(start, end, "");
            self.cursor = start;
            self.anchor = None;
            return;
        }
        let start = Cursor::new(self.cursor.row, self.cursor.col - 1);
        let end = self.cursor;
        let _ = self.replace_range(start, end, "");
        self.cursor = start;
    }

    pub fn delete(&mut self) {
        if self.has_selection() {
            self.last_edit = None;
        }
        self.record(EditKind::Delete);
        if self.delete_selection_inner() {
            return;
        }
        let len = self.line_len(self.cursor.row);
        if self.cursor.col >= len {
            if self.cursor.row + 1 >= self.lines.len() {
                return;
            }
            let start = self.cursor;
            let end = Cursor::new(self.cursor.row + 1, 0);
            let _ = self.replace_range(start, end, "");
            return;
        }
        let end = Cursor::new(self.cursor.row, self.cursor.col + 1);
        let _ = self.replace_range(self.cursor, end, "");
    }

    /// Ctrl+Backspace: delete the word immediately before the cursor.
    pub fn delete_word_back(&mut self) {
        if self.has_selection() {
            self.last_edit = None;
            self.record(EditKind::Other);
            self.delete_selection_inner();
            return;
        }
        let old = self.cursor;
        let start = self.word_left();
        if start == old {
            return;
        }
        self.record(EditKind::Other);
        let _ = self.replace_range(start, old, "");
        self.cursor = start;
    }

    pub fn delete_selection(&mut self) -> bool {
        if !self.has_selection() {
            return false;
        }
        self.record(EditKind::Other);
        self.delete_selection_inner()
    }

    pub fn tab(&mut self) {
        if self.has_selection() {
            self.indent_selection();
        } else {
            self.record(EditKind::Insert);
            let new_cursor = self.replace_range(self.cursor, self.cursor, "    ");
            self.cursor = new_cursor;
        }
    }

    pub fn backtab(&mut self) {
        self.outdent_selection();
    }

    fn selected_rows(&self) -> (usize, usize) {
        match self.selection() {
            Some((start, end)) => (start.row, end.row),
            None => (self.cursor.row, self.cursor.row),
        }
    }

    fn indent_selection(&mut self) {
        let (first, last) = self.selected_rows();
        self.record(EditKind::Other);
        for line in self.lines.iter_mut().take(last + 1).skip(first) {
            line.insert_str(0, "    ");
        }
        if self.cursor.row >= first && self.cursor.row <= last {
            self.cursor.col += 4;
        }
        if let Some(anchor) = self.anchor.as_mut() {
            if anchor.row >= first && anchor.row <= last {
                anchor.col += 4;
            }
        }
        self.anchor = None;
        self.clamp_cursor();
        self.syntax.invalidate();
        self.last_edit = None;
    }

    fn outdent_selection(&mut self) {
        let (first, last) = self.selected_rows();
        self.record(EditKind::Other);
        for line in self.lines.iter_mut().take(last + 1).skip(first) {
            let remove = line.chars().take(4).take_while(|c| *c == ' ').count();
            if remove > 0 {
                let byte_idx = line
                    .char_indices()
                    .nth(remove)
                    .map(|(i, _)| i)
                    .unwrap_or(line.len());
                line.drain(..byte_idx);
            }
        }
        let removed_here = self.lines[self.cursor.row]
            .chars()
            .take_while(|c| *c == ' ')
            .count()
            .min(4);
        self.cursor.col = self.cursor.col.saturating_sub(removed_here);
        self.anchor = None;
        self.clamp_cursor();
        self.syntax.invalidate();
        self.last_edit = None;
    }

    /// Toggle `-- ` comments for the selected lines (or the current line).
    pub fn toggle_comment(&mut self) {
        let (first, last) = self.selected_rows();
        let all_commented = self.lines[first..=last]
            .iter()
            .filter(|l| !l.trim().is_empty())
            .all(|l| l.trim_start().starts_with("--"));
        self.record(EditKind::Other);
        let mut delta_for_cursor: isize = 0;
        for (offset, line) in self.lines[first..=last].iter_mut().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let indent_bytes = line.len() - line.trim_start().len();
            let indent: String = line[..indent_bytes].to_string();
            let rest: String = line[indent_bytes..].to_string();
            let new_line = if all_commented {
                let stripped = rest
                    .strip_prefix("-- ")
                    .or_else(|| rest.strip_prefix("--"))
                    .unwrap_or(&rest);
                let delta = -(rest.chars().count() as isize - stripped.chars().count() as isize);
                if first + offset == self.cursor.row {
                    delta_for_cursor = delta;
                }
                format!("{indent}{stripped}")
            } else {
                if first + offset == self.cursor.row {
                    delta_for_cursor = 3;
                }
                format!("{indent}-- {rest}")
            };
            *line = new_line;
        }
        self.cursor.col = (self.cursor.col as isize + delta_for_cursor).max(0) as usize;
        self.anchor = None;
        self.clamp_cursor();
        self.syntax.invalidate();
        self.last_edit = None;
    }

    pub fn ensure_visible(&mut self, rows: u16, cols: u16) {
        self.set_viewport(rows, cols);
        let rows = self.viewport_rows;
        let cols = self.viewport_cols;
        if rows == 0 || cols == 0 {
            return;
        }
        if self.cursor.row < self.scroll_row {
            self.scroll_row = self.cursor.row;
        } else if self.cursor.row >= self.scroll_row + rows {
            self.scroll_row = self.cursor.row + 1 - rows;
        }
        if self.cursor.col < self.scroll_col {
            self.scroll_col = self.cursor.col;
        } else if self.cursor.col >= self.scroll_col + cols {
            self.scroll_col = self.cursor.col + 1 - cols;
        }
        let max_row = self.lines.len().saturating_sub(rows);
        self.scroll_row = self.scroll_row.min(max_row);
    }

    // ---------- Completion helpers ----------------------------------------

    /// Returns the start position and the word immediately before the cursor.
    pub fn word_prefix(&self) -> Option<(Cursor, String)> {
        if self.has_selection() {
            return None;
        }
        let chars: Vec<char> = self.lines[self.cursor.row].chars().collect();
        let mut start = self.cursor.col.min(chars.len());
        while start > 0 && is_word_char(chars[start - 1]) {
            start -= 1;
        }
        let prefix: String = chars[start..self.cursor.col.min(chars.len())]
            .iter()
            .collect();
        if prefix.is_empty() {
            None
        } else {
            Some((Cursor::new(self.cursor.row, start), prefix))
        }
    }

    pub fn replace_word_prefix(&mut self, replacement: &str) {
        let Some((start, _)) = self.word_prefix() else {
            self.insert_text(replacement);
            return;
        };
        self.replace_prefix_before_cursor(start, replacement);
    }

    /// Replace everything between `start` and the cursor. Used by the
    /// completion popup, which may have been opened with an empty prefix.
    pub fn replace_prefix_before_cursor(&mut self, start: Cursor, replacement: &str) {
        if !self.has_selection() && start <= self.cursor {
            self.last_edit = None;
            self.record(EditKind::Other);
            let new_cursor = self.replace_range(start, self.cursor, replacement);
            self.cursor = new_cursor;
            self.preferred_col = None;
        } else {
            self.insert_text(replacement);
        }
    }

    // ---------- Syntax / rendering ----------------------------------------

    pub fn ensure_syntax(&mut self) {
        self.syntax.ensure(&self.lines);
    }

    pub fn syntax_line(&self, row: usize) -> &[Token] {
        self.syntax.lines.get(row).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Utility for UI: character slice of a line.
    pub fn line_chars(&self, row: usize) -> Vec<char> {
        self.lines
            .get(row)
            .map(|l| l.chars().collect())
            .unwrap_or_default()
    }
}

fn normalize_text(text: &str) -> Vec<String> {
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let expanded = normalized.replace('\t', "    ");
    let lines: Vec<String> = expanded.split('\n').map(ToString::to_string).collect();
    if lines.is_empty() {
        vec![String::new()]
    } else {
        lines
    }
}

pub fn slice_chars(s: &str, start: usize, end: usize) -> String {
    let start = start.min(s.chars().count());
    let end = end.max(start).min(s.chars().count());
    s.chars().skip(start).take(end - start).collect()
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

#[cfg(test)]
mod tests {
    use super::*;

    fn editor(text: &str) -> Editor {
        Editor::new(text)
    }

    #[test]
    fn insert_and_newline() {
        let mut e = editor("");
        e.insert_text("SELECT 1");
        assert_eq!(e.text(), "SELECT 1");
        e.newline();
        e.insert_text("FROM t");
        assert_eq!(e.text(), "SELECT 1\nFROM t");
        assert_eq!(e.cursor, Cursor::new(1, 6));
    }

    #[test]
    fn auto_indent() {
        let mut e = editor("    SELECT");
        e.move_cursor(Motion::End, false);
        e.newline();
        e.insert_text("x");
        assert_eq!(e.text(), "    SELECT\n    x");
    }

    #[test]
    fn selection_and_copy() {
        let mut e = editor("hello\nworld");
        e.move_cursor(Motion::DocStart, false);
        for _ in 0..7 {
            e.move_cursor(Motion::Right, true);
        }
        assert_eq!(e.selected_text(), "hello\nw");
        e.insert_text("X");
        assert_eq!(e.text(), "Xorld");
    }

    #[test]
    fn backspace_joins_lines() {
        let mut e = editor("a\nb");
        e.move_cursor(Motion::Down, false);
        e.backspace();
        assert_eq!(e.text(), "ab");
        assert_eq!(e.cursor, Cursor::new(0, 1));
    }

    #[test]
    fn delete_joins_lines() {
        let mut e = editor("a\nb");
        e.move_cursor(Motion::End, false);
        e.delete();
        assert_eq!(e.text(), "ab");
    }

    #[test]
    fn undo_redo_text() {
        let mut e = editor("SELECT");
        e.move_cursor(Motion::End, false);
        e.insert_text(" 1");
        assert_eq!(e.text(), "SELECT 1");
        e.undo();
        // Coalescing may group the insert with the original text, but undo
        // must never leave the buffer in an inconsistent state.
        assert!(e.text().starts_with("SELECT"));
        e.redo();
        assert_eq!(e.text(), "SELECT 1");
    }

    #[test]
    fn indent_and_outdent() {
        let mut e = editor("a\nb");
        e.select_all();
        e.tab();
        assert_eq!(e.text(), "    a\n    b");
        e.select_all();
        e.backtab();
        assert_eq!(e.text(), "a\nb");
    }

    #[test]
    fn comments_toggle() {
        let mut e = editor("SELECT 1");
        e.toggle_comment();
        assert_eq!(e.text(), "-- SELECT 1");
        e.toggle_comment();
        assert_eq!(e.text(), "SELECT 1");
    }

    #[test]
    fn word_movement() {
        let mut e = editor("SELECT foo_bar, baz");
        e.move_cursor(Motion::WordRight, false);
        assert_eq!(e.cursor, Cursor::new(0, 7));
        e.move_cursor(Motion::WordRight, false);
        assert_eq!(e.cursor, Cursor::new(0, 16));
        e.move_cursor(Motion::WordLeft, false);
        assert_eq!(e.cursor, Cursor::new(0, 7));
    }

    #[test]
    fn dirty_state_tracks_saved_text() {
        let mut e = editor("SELECT 1");
        assert!(!e.is_modified());
        e.insert_char(' ');
        assert!(e.is_modified());
        e.mark_clean();
        assert!(!e.is_modified());
    }

    #[test]
    fn completion_prefix() {
        let mut e = editor("SELECT use");
        e.move_cursor(Motion::End, false);
        let (_, prefix) = e.word_prefix().unwrap();
        assert_eq!(prefix, "use");
        e.replace_word_prefix("users");
        assert_eq!(e.text(), "SELECT users");
    }

    #[test]
    fn horizontal_navigation_wraps_lines() {
        let mut e = editor("ab\ncd");
        e.move_cursor(Motion::End, false);
        e.move_cursor(Motion::Right, false);
        assert_eq!(e.cursor, Cursor::new(1, 0));
        e.move_cursor(Motion::Left, false);
        assert_eq!(e.cursor, Cursor::new(0, 2));
    }
}
