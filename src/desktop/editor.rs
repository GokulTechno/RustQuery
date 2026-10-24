//! Custom SQL editor widget.
//!
//! The desktop build deliberately keeps [`crate::editor::Editor`] as the source
//! of truth. Painting only the visible lines and forwarding text/key events to
//! that buffer preserves undo, selection, completion and the existing dirty
//! tracking without maintaining a second copy inside `egui::TextEdit`.

use eframe::egui::{
    self, text::LayoutJob, Align2, Color32, CursorIcon, Event, FontId, Id, Key, Modifiers, Pos2,
    Rect, ScrollArea, Sense, Stroke, TextFormat, Ui, Vec2,
};

use crate::app::{App, Focus};
use crate::editor::highlight::TokenKind;

use super::theme::Palette;

const FONT_SIZE: f32 = 13.0;
const PAD_X: f32 = 8.0;
const PAD_Y: f32 = 6.0;

#[derive(Clone, Copy)]
struct Geometry {
    rect: Rect,
    text_rect: Rect,
    text_left: f32,
    text_top: f32,
    char_width: f32,
    row_height: f32,
    visible_rows: usize,
}

pub fn show(app: &mut App, ui: &mut Ui, palette: &Palette) {
    let Some(tab_id) = app.active_tab().map(|tab| tab.id) else {
        return;
    };
    let modal_open = app.modal.is_some();

    let rect = ui.available_rect_before_wrap();
    if rect.width() < 20.0 || rect.height() < 20.0 {
        return;
    }
    ui.allocate_rect(rect, Sense::hover());
    let response = ui.interact(
        rect,
        Id::new(("rustquery_sql_editor", tab_id)),
        Sense::click_and_drag(),
    );
    let painter = ui.painter_at(rect);

    let font = FontId::monospace(FONT_SIZE);
    let char_width = ui.fonts(|fonts| fonts.glyph_width(&font, '0')).max(6.0);
    let row_height = ui.fonts(|fonts| fonts.row_height(&font)).max(14.0) + 1.0;
    let line_count = app
        .active_tab()
        .map(|tab| tab.editor.line_count())
        .unwrap_or(1)
        .max(1);
    let digits = line_count.to_string().len().max(2);
    let gutter = (digits as f32 * char_width + 18.0).max(42.0);
    let text_rect = Rect::from_min_max(
        Pos2::new(rect.left() + gutter + PAD_X, rect.top() + PAD_Y),
        Pos2::new(rect.right() - 14.0, rect.bottom() - 10.0),
    );
    let visible_rows = ((text_rect.height() / row_height).floor() as usize).max(1);
    let visible_cols = ((text_rect.width() / char_width).floor() as usize).max(1);
    let geometry = Geometry {
        rect,
        text_rect,
        text_left: text_rect.left(),
        text_top: text_rect.top(),
        char_width,
        row_height,
        visible_rows,
    };

    if let Some(tab) = app.active_tab_mut() {
        tab.editor
            .set_viewport(visible_rows as u16, visible_cols as u16);
    }

    // -------------------------------------------------------------
    // Pointer interaction
    // -------------------------------------------------------------
    let shift = ui.input(|input| input.modifiers.shift);
    let pointer = response.interact_pointer_pos();
    if !modal_open {
        if response.clicked() {
            if let Some(pos) = pointer {
                let (row, col) = position_to_cell(pos, &geometry, app);
                if let Some(tab) = app.active_tab_mut() {
                    tab.editor.set_cursor_from_click(row, col, shift);
                }
            }
            app.focus = Focus::Editor;
            response.request_focus();
        } else if response.drag_started() {
            if let Some(pos) = pointer {
                let (row, col) = position_to_cell(pos, &geometry, app);
                if let Some(tab) = app.active_tab_mut() {
                    tab.editor.set_cursor_from_click(row, col, false);
                }
            }
            app.focus = Focus::Editor;
            response.request_focus();
        } else if response.dragged() {
            if let Some(pos) = pointer {
                let (row, col) = position_to_cell(pos, &geometry, app);
                if let Some(tab) = app.active_tab_mut() {
                    tab.editor.set_cursor_from_click(row, col, true);
                }
            }
        }
    }

    if response.hovered() {
        ui.output_mut(|output| output.cursor_icon = CursorIcon::Text);
        let scroll = ui.input(|input| input.smooth_scroll_delta);
        if scroll != Vec2::ZERO {
            if let Some(tab) = app.active_tab_mut() {
                let editor = &mut tab.editor;
                let max_row = editor.line_count().saturating_sub(1);
                let row_delta = (-scroll.y / row_height).round() as isize;
                if row_delta != 0 {
                    editor.scroll_row = (editor.scroll_row as isize + row_delta)
                        .clamp(0, max_row as isize)
                        as usize;
                }
                let col_delta = (-scroll.x / char_width).round() as isize;
                if col_delta != 0 {
                    let max_col = editor
                        .lines
                        .iter()
                        .map(|line| line.chars().count())
                        .max()
                        .unwrap_or(0);
                    editor.scroll_col = (editor.scroll_col as isize + col_delta)
                        .clamp(0, max_col as isize)
                        as usize;
                }
            }
        }
    }

    // The editor pane owns the keyboard whenever it is the selected pane and
    // no modal dialog is open. This makes Ctrl+Space completion usable even
    // after clicking a toolbar button.
    let keyboard_owner =
        !modal_open && app.focus == Focus::Editor && app.view == crate::app::View::Workspace;
    if keyboard_owner {
        if !response.has_focus() {
            response.request_focus();
        }
        let before = app.active_tab().map(|tab| tab.editor.cursor);
        handle_events(app, ui, &response);
        let after = app.active_tab().map(|tab| tab.editor.cursor);
        if before != after {
            if let Some(tab) = app.active_tab_mut() {
                tab.editor
                    .ensure_visible(visible_rows as u16, visible_cols as u16);
            }
        }
    }

    // -------------------------------------------------------------
    // Painting
    // -------------------------------------------------------------
    painter.rect_filled(rect, 0.0, palette.bg);
    painter.rect_filled(
        Rect::from_min_max(rect.min, Pos2::new(rect.left() + gutter, rect.bottom())),
        0.0,
        palette.panel,
    );
    painter.line_segment(
        [
            Pos2::new(rect.left() + gutter, rect.top()),
            Pos2::new(rect.left() + gutter, rect.bottom()),
        ],
        Stroke::new(1.0_f32, palette.border),
    );

    let Some(tab) = app.active_tab() else {
        return;
    };
    let editor = &tab.editor;
    let first_line = editor.scroll_row.min(editor.line_count().saturating_sub(1));
    let last_line = (first_line + visible_rows + 1).min(editor.line_count());
    let selection = editor.selection();
    let cursor_row = editor.cursor.row;

    // Syntax cache is populated once per edit, not once per visible line.
    if let Some(tab) = app.active_tab_mut() {
        tab.editor.ensure_syntax();
    }
    let editor = &app.active_tab().expect("active tab").editor;

    for row in first_line..last_line {
        let y = geometry.text_top + (row - first_line) as f32 * row_height;
        if y > rect.bottom() {
            break;
        }
        let row_rect = Rect::from_min_max(
            Pos2::new(rect.left() + gutter, y),
            Pos2::new(rect.right(), y + row_height),
        );
        if row == cursor_row && keyboard_owner {
            painter.rect_filled(row_rect, 0.0, palette.surface.gamma_multiply(0.45));
        }

        // Selection is painted before the text so glyphs stay legible.
        if let Some((start, end)) = selection {
            if row >= start.row && row <= end.row {
                let line_len = editor.line_len(row);
                let start_col = if row == start.row { start.col } else { 0 };
                let end_col = if row == end.row { end.col } else { line_len };
                if end_col > start_col || (row == end.row && row != start.row) {
                    let x0 = geometry.text_left
                        + (start_col as f32 - editor.scroll_col as f32) * char_width;
                    let x1 = geometry.text_left
                        + (end_col as f32 - editor.scroll_col as f32) * char_width;
                    let x0 = x0.max(geometry.text_rect.left());
                    let x1 = x1.min(geometry.text_rect.right());
                    if x1 > x0 {
                        painter.rect_filled(
                            Rect::from_min_max(Pos2::new(x0, y), Pos2::new(x1, y + row_height)),
                            0.0,
                            palette.selection,
                        );
                    }
                }
            }
        }

        // Line number.
        let line_no = format!("{}", row + 1);
        let number_color = if row == cursor_row {
            palette.text
        } else {
            palette.muted
        };
        painter.text(
            Pos2::new(rect.left() + gutter - 7.0, y),
            Align2::RIGHT_TOP,
            line_no,
            font.clone(),
            number_color,
        );

        // Syntax-highlighted source line.
        if let Some(line) = editor.lines.get(row) {
            let job = highlight_job(line, editor.syntax_line(row), palette, &font);
            let galley = ui.fonts(|fonts| fonts.layout_job(job));
            let pos = Pos2::new(
                geometry.text_left - editor.scroll_col as f32 * char_width,
                y,
            );
            if pos.x + galley.size().x > geometry.text_rect.left() {
                let clipped = geometry.text_rect.intersect(Rect::from_min_size(
                    pos,
                    Vec2::new(galley.size().x.max(1.0), row_height),
                ));
                if clipped.is_positive() {
                    let text_painter = ui.painter_at(geometry.text_rect);
                    text_painter.galley(pos, galley, palette.text);
                }
            }
        }
    }

    // Caret (blinking at roughly 1.8 Hz).
    if keyboard_owner && response.has_focus() {
        let time = ui.input(|input| input.time);
        if (time * 1.8).fract() < 0.65 {
            let x = geometry.text_left
                + (editor.cursor.col as f32 - editor.scroll_col as f32) * char_width;
            let y = geometry.text_top + (editor.cursor.row as f32 - first_line as f32) * row_height;
            if geometry.text_rect.contains(Pos2::new(x, y))
                || (x >= geometry.text_rect.left()
                    && x <= geometry.text_rect.right()
                    && y >= geometry.text_rect.top()
                    && y <= geometry.text_rect.bottom())
            {
                ui.painter_at(geometry.text_rect).line_segment(
                    [Pos2::new(x, y + 1.0), Pos2::new(x, y + row_height - 2.0)],
                    Stroke::new(1.6_f32, palette.accent),
                );
            }
        }
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(300));
    }

    // Thin scroll indicators (the editor is wheel/keyboard scrolled).
    paint_scrollbars(ui, &geometry, editor, palette);

    // -------------------------------------------------------------
    // Completion popup
    // -------------------------------------------------------------
    if keyboard_owner {
        show_completion(app, ui, &geometry, palette);
    }
}

fn handle_events(app: &mut App, ui: &mut Ui, response: &egui::Response) {
    // Keyboard focus may move after egui's own tab navigation; keep it here.
    if !response.has_focus() {
        return;
    }
    let events = ui.input(|input| input.events.clone());
    let mut paste_seen = false;
    let mut paste_requested = false;
    for event in events {
        match event {
            Event::Text(text) => {
                if !text.is_empty() {
                    app.completion = None;
                    if let Some(tab) = app.active_tab_mut() {
                        tab.editor.insert_text(&text);
                    }
                    app.workspace_dirty = true;
                }
            }
            Event::Paste(text) => {
                paste_seen = true;
                app.completion = None;
                if let Some(tab) = app.active_tab_mut() {
                    tab.editor.insert_text(&text);
                }
                app.workspace_dirty = true;
            }
            Event::Copy => {
                app.copy_selection_or_line();
            }
            Event::Cut => {
                app.cut_selection();
            }
            Event::Key {
                key: Key::V,
                pressed: true,
                modifiers,
                ..
            } if modifiers.command => paste_requested = true,
            Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } => handle_key(app, key, modifiers),
            _ => {}
        }
    }
    // Native backends normally synthesise `Event::Paste`. The internal
    // clipboard fallback needs a manual path for sessions without a system
    // clipboard, so only use it when no Paste event arrived this frame.
    if paste_requested && !paste_seen {
        app.paste_into_editor();
    }
}

fn handle_key(app: &mut App, key: Key, modifiers: Modifiers) {
    if app.completion.is_some() {
        match key {
            Key::Escape => {
                app.completion = None;
                return;
            }
            Key::ArrowDown => {
                app.completion_step(1, false);
                return;
            }
            Key::ArrowUp => {
                app.completion_step(-1, false);
                return;
            }
            Key::PageDown => {
                app.completion_step(1, true);
                return;
            }
            Key::PageUp => {
                app.completion_step(-1, true);
                return;
            }
            Key::Enter | Key::Tab => {
                app.accept_completion();
                return;
            }
            _ => app.completion = None,
        }
    }

    let command = modifiers.command;
    let shift = modifiers.shift;
    if command {
        match key {
            Key::A => {
                if let Some(tab) = app.active_tab_mut() {
                    tab.editor.select_all();
                }
            }
            Key::Z => {
                if let Some(tab) = app.active_tab_mut() {
                    if shift {
                        tab.editor.redo();
                    } else {
                        tab.editor.undo();
                    }
                }
                app.workspace_dirty = true;
            }
            Key::Y => {
                if let Some(tab) = app.active_tab_mut() {
                    tab.editor.redo();
                }
                app.workspace_dirty = true;
            }
            Key::Slash => {
                if let Some(tab) = app.active_tab_mut() {
                    tab.editor.toggle_comment();
                }
                app.workspace_dirty = true;
            }
            Key::Space => app.open_completion(),
            Key::ArrowLeft => app.editor_move(crate::editor::Motion::WordLeft, shift),
            Key::ArrowRight => app.editor_move(crate::editor::Motion::WordRight, shift),
            Key::Home => app.editor_move(crate::editor::Motion::DocStart, shift),
            Key::End => app.editor_move(crate::editor::Motion::DocEnd, shift),
            Key::Backspace => {
                if let Some(tab) = app.active_tab_mut() {
                    tab.editor.delete_word_back();
                }
                app.workspace_dirty = true;
            }
            _ => {}
        }
        return;
    }

    match key {
        Key::Enter => {
            if let Some(tab) = app.active_tab_mut() {
                tab.editor.newline();
            }
            app.workspace_dirty = true;
        }
        Key::Tab => {
            if let Some(tab) = app.active_tab_mut() {
                if shift {
                    tab.editor.backtab();
                } else {
                    tab.editor.tab();
                }
            }
            app.workspace_dirty = true;
        }
        Key::Backspace => {
            if let Some(tab) = app.active_tab_mut() {
                tab.editor.backspace();
            }
            app.workspace_dirty = true;
        }
        Key::Delete => {
            if let Some(tab) = app.active_tab_mut() {
                tab.editor.delete();
            }
            app.workspace_dirty = true;
        }
        Key::ArrowLeft => app.editor_move(crate::editor::Motion::Left, shift),
        Key::ArrowRight => app.editor_move(crate::editor::Motion::Right, shift),
        Key::ArrowUp => app.editor_move(crate::editor::Motion::Up, shift),
        Key::ArrowDown => app.editor_move(crate::editor::Motion::Down, shift),
        Key::Home => app.editor_move(crate::editor::Motion::Home, shift),
        Key::End => app.editor_move(crate::editor::Motion::End, shift),
        Key::PageUp => app.editor_move(crate::editor::Motion::PageUp, shift),
        Key::PageDown => app.editor_move(crate::editor::Motion::PageDown, shift),
        Key::Escape => {
            if let Some(tab) = app.active_tab_mut() {
                if tab.editor.has_selection() {
                    tab.editor.clear_selection();
                }
            }
        }
        _ => {}
    }
}

fn position_to_cell(pos: Pos2, geometry: &Geometry, app: &App) -> (usize, usize) {
    let row = ((pos.y - geometry.text_top) / geometry.row_height).floor();
    let row = row.max(0.0) as usize;
    let col = ((pos.x - geometry.text_left) / geometry.char_width).round();
    let col = col.max(0.0) as usize;
    let Some(tab) = app.active_tab() else {
        return (0, 0);
    };
    let row = (tab.editor.scroll_row + row).min(tab.editor.line_count().saturating_sub(1));
    let col = (tab.editor.scroll_col + col).min(tab.editor.line_len(row));
    (row, col)
}

fn highlight_job(
    line: &str,
    tokens: &[crate::editor::highlight::Token],
    palette: &Palette,
    font: &FontId,
) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.wrap.max_width = f32::INFINITY;
    let chars: Vec<char> = line.chars().collect();
    let mut cursor = 0usize;
    for token in tokens {
        let start = token.start.min(chars.len());
        let end = token.end.min(chars.len());
        if start > cursor {
            append_chars(&mut job, &chars[cursor..start], palette.text, font);
        }
        if end > start {
            append_chars(
                &mut job,
                &chars[start..end],
                token_color(token.kind, palette),
                font,
            );
        }
        cursor = cursor.max(end);
    }
    if cursor < chars.len() {
        append_chars(&mut job, &chars[cursor..], palette.text, font);
    }
    if chars.is_empty() {
        job.append(" ", 0.0, format(palette.text, font));
    }
    job
}

fn append_chars(job: &mut LayoutJob, chars: &[char], color: Color32, font: &FontId) {
    if chars.is_empty() {
        return;
    }
    let text: String = chars.iter().collect();
    job.append(&text, 0.0, format(color, font));
}

fn format(color: Color32, font: &FontId) -> TextFormat {
    TextFormat {
        font_id: font.clone(),
        color,
        ..Default::default()
    }
}

fn token_color(kind: TokenKind, palette: &Palette) -> Color32 {
    match kind {
        TokenKind::Keyword => palette.accent,
        TokenKind::Function => palette.blue,
        TokenKind::Type => palette.yellow,
        TokenKind::String => palette.green,
        TokenKind::QuotedIdentifier => palette.purple,
        TokenKind::Number => palette.yellow,
        TokenKind::Comment => palette.muted,
        TokenKind::Operator => palette.subtext,
        TokenKind::Variable => palette.red,
        TokenKind::Punctuation => palette.subtext,
        TokenKind::Identifier => palette.text,
    }
}

fn paint_scrollbars(
    ui: &Ui,
    geometry: &Geometry,
    editor: &crate::editor::Editor,
    palette: &Palette,
) {
    let painter = ui.painter_at(geometry.rect);
    let content_rows = editor.line_count().max(1);
    if content_rows > geometry.visible_rows {
        let ratio = geometry.visible_rows as f32 / content_rows as f32;
        let track = Rect::from_min_max(
            Pos2::new(geometry.rect.right() - 7.0, geometry.rect.top() + 2.0),
            Pos2::new(geometry.rect.right() - 2.0, geometry.rect.bottom() - 2.0),
        );
        let height = (track.height() * ratio).max(18.0);
        let max_scroll = content_rows.saturating_sub(geometry.visible_rows) as f32;
        let top = if max_scroll <= 0.0 {
            0.0
        } else {
            (track.height() - height) * editor.scroll_row as f32 / max_scroll
        };
        painter.rect_filled(track, 3.0, palette.panel);
        painter.rect_filled(
            Rect::from_min_size(
                Pos2::new(track.left(), track.top() + top),
                Vec2::new(track.width(), height),
            ),
            3.0,
            palette.muted,
        );
    }
}

fn show_completion(app: &mut App, ui: &mut Ui, geometry: &Geometry, palette: &Palette) {
    let Some(completion) = &app.completion else {
        return;
    };
    let Some(tab) = app.active_tab() else {
        return;
    };
    let x = geometry.text_left
        + (tab.editor.cursor.col as f32 - tab.editor.scroll_col as f32) * geometry.char_width;
    let y = geometry.text_top
        + (tab.editor.cursor.row as f32 - tab.editor.scroll_row as f32 + 1.0) * geometry.row_height;
    let items = completion.items.clone();
    let selected = completion.selected;
    let pos = Pos2::new(
        x.clamp(geometry.rect.left() + 8.0, geometry.rect.right() - 280.0),
        (y + 2.0).min(geometry.rect.bottom() - 80.0),
    );

    let mut clicked: Option<usize> = None;
    egui::Area::new(Id::new(("rustquery_completion", tab.id)))
        .order(egui::Order::Foreground)
        .fixed_pos(pos)
        .constrain(true)
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style())
                .fill(palette.panel)
                .stroke(Stroke::new(1.0_f32, palette.border))
                .show(ui, |ui| {
                    ui.set_min_width(240.0);
                    ui.label(
                        egui::RichText::new("Completions · Up/Down then Enter")
                            .small()
                            .color(palette.muted),
                    );
                    ScrollArea::vertical().max_height(210.0).show(ui, |ui| {
                        for (index, item) in items.iter().enumerate() {
                            let response = ui.selectable_label(index == selected, item);
                            if response.clicked() {
                                clicked = Some(index);
                            }
                        }
                    });
                });
        });
    if let Some(index) = clicked {
        if let Some(completion) = &mut app.completion {
            completion.selected = index;
        }
        app.accept_completion();
    }
}
