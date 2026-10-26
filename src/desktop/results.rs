//! Virtualised result grid with sortable headers and inline filtering.

use eframe::egui::{
    self, Align2, FontId, Frame, Id, Key, Layout, Margin, Pos2, Rect, RichText, ScrollArea, Sense,
    Stroke, StrokeKind, TextEdit, Ui, Vec2,
};

use crate::app::{App, Focus};
use crate::grid::truncate_display;
use crate::model::{format_count, LogLevel, QueryResult};

use super::theme::{metrics, Palette};

pub fn show(app: &mut App, ui: &mut Ui, palette: &Palette) {
    let ui_metrics = metrics(app.settings.ui_scale_percent);
    let row_height = ui_metrics.grid_row_height;
    let header_height = ui_metrics.grid_header_height;
    let gutter = ui_metrics.grid_gutter;
    let mono_font = FontId::monospace(ui_metrics.monospace);

    let Some(tab) = app.active_tab() else {
        return;
    };
    let tab_id = tab.id;
    let has_result = tab.result.is_some();
    let error = tab.last_error.clone();

    if !has_result {
        show_placeholder(app, ui, palette, error.as_ref());
        return;
    }
    let statement_only = app
        .active_tab()
        .and_then(|tab| tab.result.as_ref())
        .map(|result| result.columns.is_empty())
        .unwrap_or(false);
    if statement_only {
        show_statement_result(app, ui, palette);
        return;
    }

    // ---- toolbar and inline filter --------------------------------------
    let mut filter = app
        .active_tab()
        .and_then(|tab| tab.grid.as_ref())
        .map(|grid| grid.filter.clone())
        .unwrap_or_default();
    let filter_id = Id::new(("rustquery_result_filter", tab_id));
    let mut toolbar_action: Option<ToolbarAction> = None;
    Frame::new()
        .fill(palette.panel)
        .inner_margin(Margin::symmetric(6, 4))
        .corner_radius(6)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Results").strong().color(palette.text));
                let (visible, total, sorted) = app
                    .active_tab()
                    .and_then(|tab| tab.grid.as_ref().zip(tab.result.as_ref()))
                    .map(|(grid, result)| {
                        (
                            grid.visible_count(),
                            result.rows.len(),
                            grid.sort.map(|sort| {
                                let name = result
                                    .columns
                                    .get(sort.column)
                                    .map(|column| column.name.as_str())
                                    .unwrap_or("?");
                                format!("{name} {}", sort.direction.arrow())
                            }),
                        )
                    })
                    .unwrap_or((0, 0, None));
                ui.label(
                    RichText::new(format!(
                        "{} of {} row{}",
                        format_count(visible as u64),
                        format_count(total as u64),
                        if total == 1 { "" } else { "s" }
                    ))
                    .small()
                    .color(palette.muted),
                );
                if let Some(sorted) = sorted {
                    ui.label(
                        RichText::new(format!("sorted by {sorted}"))
                            .small()
                            .color(palette.accent),
                    );
                }
                ui.separator();
                let response = ui
                    .add(
                        TextEdit::singleline(&mut filter)
                            .id(filter_id)
                            .hint_text("Filter all columns…")
                            .desired_width(190.0)
                            .text_color(palette.text),
                    )
                    .on_hover_text("Filter every visible column (case-insensitive)");
                if response.has_focus() || response.gained_focus() {
                    app.focus = Focus::Results;
                }
                if response.changed() {
                    if let Some(tab) = app.active_tab_mut() {
                        if let (Some(result), Some(grid)) = (&tab.result, &mut tab.grid) {
                            grid.set_filter(result, filter.clone());
                        }
                    }
                }
                if !filter.is_empty()
                    && ui.small_button("×").on_hover_text("Clear filter").clicked()
                {
                    filter.clear();
                    if let Some(tab) = app.active_tab_mut() {
                        if let (Some(result), Some(grid)) = (&tab.result, &mut tab.grid) {
                            grid.clear_filter(result);
                        }
                    }
                }

                ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Export").clicked() {
                        toolbar_action = Some(ToolbarAction::Export);
                    }
                    if ui.button("Cell details").clicked() {
                        toolbar_action = Some(ToolbarAction::Detail);
                    }
                    if ui.button("Copy all").clicked() {
                        toolbar_action = Some(ToolbarAction::CopyAll);
                    }
                    if ui.button("Copy row").clicked() {
                        toolbar_action = Some(ToolbarAction::CopyRow);
                    }
                    if ui.button("Copy cell").clicked() {
                        toolbar_action = Some(ToolbarAction::CopyCell);
                    }
                });
            });
        });

    if let Some(action) = toolbar_action {
        apply_toolbar(app, action);
    }

    // ---- keyboard navigation --------------------------------------------
    let grid_id = Id::new(("rustquery_grid", tab_id));
    let filter_focused = ui.memory(|memory| memory.has_focus(filter_id));
    let grid_has_focus = ui.memory(|memory| memory.has_focus(grid_id));
    let mut keyboard_action: Option<KeyAction> = None;
    let mut keyboard_moved = false;
    if app.modal.is_none() && app.focus == Focus::Results && grid_has_focus {
        let events = ui.input(|input| input.events.clone());
        for event in events {
            match event {
                egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } => match key {
                    Key::ArrowUp => {
                        grid_move(app, 0, -1);
                        keyboard_moved = true;
                    }
                    Key::ArrowDown => {
                        grid_move(app, 0, 1);
                        keyboard_moved = true;
                    }
                    Key::ArrowLeft => {
                        grid_move(app, -1, 0);
                        keyboard_moved = true;
                    }
                    Key::ArrowRight => {
                        grid_move(app, 1, 0);
                        keyboard_moved = true;
                    }
                    Key::PageUp => {
                        grid_page(app, -1);
                        keyboard_moved = true;
                    }
                    Key::PageDown => {
                        grid_page(app, 1);
                        keyboard_moved = true;
                    }
                    Key::Home => {
                        if modifiers.command {
                            grid_jump_row(app, false);
                        } else {
                            grid_jump_col(app, false);
                        }
                        keyboard_moved = true;
                    }
                    Key::End => {
                        if modifiers.command {
                            grid_jump_row(app, true);
                        } else {
                            grid_jump_col(app, true);
                        }
                        keyboard_moved = true;
                    }
                    Key::Enter => keyboard_action = Some(KeyAction::Detail),
                    Key::S => grid_sort(app, modifiers.shift),
                    Key::C => keyboard_action = Some(KeyAction::CopyCell),
                    Key::Y => {
                        keyboard_action = Some(if modifiers.command {
                            KeyAction::CopyAll
                        } else {
                            KeyAction::CopyRow
                        });
                    }
                    Key::Escape => keyboard_action = Some(KeyAction::FocusEditor),
                    _ => {}
                },
                egui::Event::Copy => keyboard_action = Some(KeyAction::CopyCell),
                _ => {}
            }
        }
    }

    // ---- geometry / scrolling state -------------------------------------
    let widths: Vec<f32> = app
        .active_tab()
        .and_then(|tab| tab.grid.as_ref())
        .map(|grid| {
            grid.widths
                .iter()
                .map(|width| (*width as f32 * 7.4).clamp(58.0, 460.0))
                .collect()
        })
        .unwrap_or_default();
    let total_width = gutter + widths.iter().sum::<f32>() + 2.0;
    let scroll_id = Id::new(("rustquery_grid_scroll", tab_id));
    let header_h_id = Id::new(("rustquery_grid_header_scroll", tab_id));

    let header_offset = ui.data(|data| data.get_temp::<f32>(header_h_id).unwrap_or(0.0));

    // Header --------------------------------------------------------------
    let mut header_clicked: Option<usize> = None;
    ScrollArea::horizontal()
        .id_salt(Id::new(("rustquery_grid_header", tab_id)))
        .horizontal_scroll_offset(header_offset)
        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.set_min_width(total_width);
            let (rect, response) =
                ui.allocate_exact_size(Vec2::new(total_width, header_height), Sense::click());
            let painter = ui.painter_at(rect);
            painter.rect_filled(rect, 0.0, palette.panel);
            painter.line_segment(
                [
                    Pos2::new(rect.left(), rect.bottom() - 0.5),
                    Pos2::new(rect.right(), rect.bottom() - 0.5),
                ],
                Stroke::new(1.0_f32, palette.border),
            );
            let Some(tab) = app.active_tab() else {
                return;
            };
            let Some(result) = &tab.result else {
                return;
            };
            painter.text(
                Pos2::new(rect.left() + gutter - 8.0, rect.center().y),
                Align2::RIGHT_CENTER,
                "#",
                mono_font.clone(),
                palette.muted,
            );
            let mut x = rect.left() + gutter;
            for (index, column) in result.columns.iter().enumerate() {
                let width = widths.get(index).copied().unwrap_or(100.0);
                let cell =
                    Rect::from_min_size(Pos2::new(x, rect.top()), Vec2::new(width, header_height));
                painter.line_segment(
                    [
                        Pos2::new(cell.right(), cell.top() + 6.0),
                        Pos2::new(cell.right(), cell.bottom() - 6.0),
                    ],
                    Stroke::new(1.0_f32, palette.border),
                );
                let sorted = tab
                    .grid
                    .as_ref()
                    .and_then(|grid| grid.sort)
                    .filter(|sort| sort.column == index);
                let label = if let Some(sort) = sorted {
                    format!("{} {}", column.name, sort.direction.arrow())
                } else {
                    column.name.clone()
                };
                let color = if sorted.is_some() {
                    palette.accent
                } else {
                    palette.text
                };
                painter
                    .with_clip_rect(cell.shrink2(Vec2::new(6.0, 0.0)))
                    .text(
                        Pos2::new(cell.left() + 6.0, cell.center().y),
                        Align2::LEFT_CENTER,
                        truncate_display(&label, (width / 7.0) as usize),
                        mono_font.clone(),
                        color,
                    );
                x += width;
            }
            if response.clicked() {
                if let Some(pos) = response.interact_pointer_pos() {
                    header_clicked = header_column_at(pos.x, rect.left(), gutter, &widths);
                }
            }
            if let Some(index) = header_clicked {
                if let Some(tab) = app.active_tab_mut() {
                    if let Some(grid) = &mut tab.grid {
                        grid.sel_col = index;
                        if let Some(result) = &tab.result {
                            grid.cycle_sort(result);
                        }
                    }
                }
                app.focus = Focus::Results;
            }
            response.on_hover_text("Click a column to cycle its sort order");
        });

    // Body ----------------------------------------------------------------
    let mut click: Option<(usize, usize, bool)> = None;
    let mut body_scroll_rect: Option<Rect> = None;
    let available_height = ui.available_height().max(60.0);
    let visible_rows = ((available_height - 4.0) / row_height).max(1.0) as u16;
    if let Some(tab) = app.active_tab_mut() {
        if let Some(grid) = &mut tab.grid {
            grid.view_rows = visible_rows;
            grid.view_width = (ui.available_width() / 7.0) as u16;
        }
    }

    let body_output = ScrollArea::both()
        .id_salt(scroll_id)
        .auto_shrink([false, false])
        .show_viewport(ui, |ui, viewport| {
            let Some(tab) = app.active_tab() else {
                return;
            };
            let Some(result) = &tab.result else {
                return;
            };
            let Some(grid) = &tab.grid else {
                return;
            };
            let rows = grid.visible_count();
            let content_height = (rows as f32 * row_height + 4.0).max(row_height);
            ui.set_min_size(Vec2::new(total_width, content_height));
            let origin = ui.max_rect().min;
            let first = (viewport.min.y / row_height).floor().max(0.0) as usize;
            let last = (((viewport.max.y / row_height).ceil() as usize) + 2).min(rows);
            let font = mono_font.clone();
            let body_rect = Rect::from_min_size(origin, Vec2::new(total_width, content_height));
            let response = ui.interact(body_rect, grid_id, Sense::click_and_drag());

            if app.modal.is_none() && app.focus == Focus::Results && !filter_focused {
                response.request_focus();
            }
            if response.clicked() || response.drag_started() || response.double_clicked() {
                response.request_focus();
                if let Some(pos) = response.interact_pointer_pos() {
                    if let Some(col) = body_column_at(pos.x, origin.x, gutter, &widths) {
                        let row = ((pos.y - origin.y) / row_height).floor().max(0.0) as usize;
                        if row < rows {
                            click = Some((row, col, response.double_clicked()));
                        }
                    }
                }
            }

            if rows == 0 {
                ui.painter().text(
                    origin + Vec2::new(gutter + 12.0, 10.0),
                    Align2::LEFT_TOP,
                    if grid.filter.is_empty() {
                        "No rows returned."
                    } else {
                        "No rows match the current filter."
                    },
                    FontId::proportional(ui_metrics.body),
                    palette.muted,
                );
                return;
            }

            let painter = ui.painter();
            painter.rect_filled(
                Rect::from_min_max(
                    Pos2::new(origin.x, origin.y),
                    Pos2::new(origin.x + gutter, origin.y + content_height),
                ),
                0.0,
                palette.panel,
            );
            for row in first..last {
                let y = origin.y + row as f32 * row_height;
                let row_rect =
                    Rect::from_min_size(Pos2::new(origin.x, y), Vec2::new(total_width, row_height));
                let selected_row = row == grid.sel_row;
                if selected_row {
                    painter.rect_filled(row_rect, 0.0, palette.elevated);
                } else if row % 2 == 1 {
                    painter.rect_filled(row_rect, 0.0, palette.surface.gamma_multiply(0.28));
                }
                painter.text(
                    Pos2::new(origin.x + gutter - 8.0, y + row_height / 2.0),
                    Align2::RIGHT_CENTER,
                    format!("{}", row + 1),
                    font.clone(),
                    palette.muted,
                );
                let Some(raw_row) = grid.row_at(row) else {
                    continue;
                };
                let Some(values) = result.rows.get(raw_row) else {
                    continue;
                };
                let mut x = origin.x + gutter;
                for (col, column) in result.columns.iter().enumerate() {
                    let width = widths.get(col).copied().unwrap_or(100.0);
                    let cell = Rect::from_min_size(Pos2::new(x, y), Vec2::new(width, row_height));
                    if selected_row && col == grid.sel_col {
                        painter.rect_stroke(
                            cell,
                            0.0,
                            Stroke::new(1.2_f32, palette.accent),
                            StrokeKind::Inside,
                        );
                    }
                    let value = values.get(col).and_then(|value| value.as_deref());
                    let numeric = column.is_numeric();
                    let (text, color) = match value {
                        Some(value) => (
                            truncate_display(value, ((width - 10.0) / 7.0).max(1.0) as usize),
                            if selected_row {
                                palette.text
                            } else {
                                palette.subtext
                            },
                        ),
                        None => ("NULL".to_string(), palette.muted),
                    };
                    let (align, text_x) = if numeric {
                        (Align2::RIGHT_CENTER, cell.right() - 5.0)
                    } else {
                        (Align2::LEFT_CENTER, cell.left() + 5.0)
                    };
                    painter
                        .with_clip_rect(cell.shrink2(Vec2::new(4.0, 2.0)))
                        .text(
                            Pos2::new(text_x, cell.center().y),
                            align,
                            text,
                            font.clone(),
                            color,
                        );
                    painter.line_segment(
                        [
                            Pos2::new(cell.right(), cell.top() + 3.0),
                            Pos2::new(cell.right(), cell.bottom() - 3.0),
                        ],
                        Stroke::new(1.0_f32, palette.surface.gamma_multiply(0.7)),
                    );
                    x += width;
                }
            }

            if keyboard_moved {
                let selected_x = origin.x + gutter + widths.iter().take(grid.sel_col).sum::<f32>();
                let selected_width = widths.get(grid.sel_col).copied().unwrap_or(100.0);
                body_scroll_rect = Some(Rect::from_min_size(
                    Pos2::new(
                        selected_x,
                        origin.y + grid.sel_row as f32 * row_height - row_height,
                    ),
                    Vec2::new(selected_width, row_height * 3.0),
                ));
            }
            if let Some(rect) = body_scroll_rect {
                ui.scroll_to_rect(rect, Some(egui::Align::Center));
            }
        });

    // Persist the horizontal scroll position so the header follows. Scroll
    // area offsets are negative when the content moves left; the header's
    // `horizontal_scroll_offset` uses the positive convention.
    let new_x = -body_output.state.offset.x;
    ui.data_mut(|data| data.insert_temp(header_h_id, new_x));

    if let Some((row, col, double)) = click {
        if let Some(tab) = app.active_tab_mut() {
            if let Some(grid) = &mut tab.grid {
                grid.sel_row = row;
                grid.sel_col = col;
                grid.ensure_visible();
            }
        }
        app.focus = Focus::Results;
        if double {
            app.open_cell_detail();
        }
    }

    if let Some(action) = keyboard_action {
        apply_key_action(app, action);
    }
}

fn grid_move(app: &mut App, d_col: isize, d_row: isize) {
    if let Some(tab) = app.active_tab_mut() {
        if let (Some(result), Some(grid)) = (&tab.result, &mut tab.grid) {
            grid.move_cell(d_row, d_col, result);
        }
    }
}

fn grid_page(app: &mut App, direction: isize) {
    if let Some(tab) = app.active_tab_mut() {
        if let (Some(result), Some(grid)) = (&tab.result, &mut tab.grid) {
            grid.page(direction, result);
        }
    }
}

fn grid_jump_row(app: &mut App, end: bool) {
    if let Some(tab) = app.active_tab_mut() {
        if let Some(grid) = &mut tab.grid {
            grid.jump_row(if end {
                grid.visible_count().saturating_sub(1)
            } else {
                0
            });
        }
    }
}

fn grid_jump_col(app: &mut App, end: bool) {
    if let Some(tab) = app.active_tab_mut() {
        if let (Some(result), Some(grid)) = (&tab.result, &mut tab.grid) {
            grid.jump_column(
                if end {
                    result.columns.len().saturating_sub(1)
                } else {
                    0
                },
                result,
            );
        }
    }
}

fn grid_sort(app: &mut App, clear: bool) {
    if let Some(tab) = app.active_tab_mut() {
        if let (Some(result), Some(grid)) = (&tab.result, &mut tab.grid) {
            if clear {
                let raw = grid.selected_raw_row();
                grid.sort = None;
                grid.rebuild(result, raw);
            } else {
                grid.cycle_sort(result);
            }
        }
    }
}

fn apply_toolbar(app: &mut App, action: ToolbarAction) {
    match action {
        ToolbarAction::CopyCell => app.copy_result_cell(),
        ToolbarAction::CopyRow => app.copy_result_row(),
        ToolbarAction::CopyAll => app.copy_all_results(),
        ToolbarAction::Detail => app.open_cell_detail(),
        ToolbarAction::Export => app.open_export_dialog(),
    }
}

fn apply_key_action(app: &mut App, action: KeyAction) {
    match action {
        KeyAction::CopyCell => app.copy_result_cell(),
        KeyAction::CopyRow => app.copy_result_row(),
        KeyAction::CopyAll => app.copy_all_results(),
        KeyAction::Detail => app.open_cell_detail(),
        KeyAction::FocusEditor => app.focus = Focus::Editor,
    }
}

#[derive(Debug, Clone, Copy)]
enum ToolbarAction {
    CopyCell,
    CopyRow,
    CopyAll,
    Detail,
    Export,
}

#[derive(Debug, Clone, Copy)]
enum KeyAction {
    CopyCell,
    CopyRow,
    CopyAll,
    Detail,
    FocusEditor,
}

fn header_column_at(x: f32, origin_x: f32, gutter: f32, widths: &[f32]) -> Option<usize> {
    if x < origin_x + gutter {
        return None;
    }
    let mut cursor = origin_x + gutter;
    for (index, width) in widths.iter().enumerate() {
        if x >= cursor && x < cursor + width {
            return Some(index);
        }
        cursor += width;
    }
    None
}

fn body_column_at(x: f32, origin_x: f32, gutter: f32, widths: &[f32]) -> Option<usize> {
    header_column_at(x, origin_x, gutter, widths)
}

/// Empty result pane: either a query error or a gentle hint that nothing has
/// been executed yet.
fn show_placeholder(
    app: &mut App,
    ui: &mut Ui,
    palette: &Palette,
    error: Option<&crate::db::DbError>,
) {
    let Some(error) = error else {
        Frame::new()
            .fill(palette.panel)
            .inner_margin(Margin::symmetric(10, 8))
            .corner_radius(6)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Results").strong().color(palette.text));
                    ui.label(
                        RichText::new("Run a query to see its result set here.")
                            .small()
                            .color(palette.muted),
                    );
                });
            });
        return;
    };

    Frame::new()
        .fill(palette.panel)
        .inner_margin(Margin::symmetric(10, 8))
        .corner_radius(6)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Query error").strong().color(palette.red));
                ui.label(RichText::new(&error.summary).small().color(palette.text));
            });
            ScrollArea::vertical()
                .id_salt("rustquery_query_error")
                .max_height(160.0)
                .show(ui, |ui| {
                    for line in error.console_lines() {
                        ui.label(
                            RichText::new(line)
                                .monospace()
                                .small()
                                .color(palette.subtext),
                        );
                    }
                });
            ui.horizontal(|ui| {
                if ui.button("Copy error").clicked() {
                    let message = app.clipboard.copy(&error.to_string());
                    app.toast(LogLevel::Info, message);
                }
            });
        });
}

fn show_statement_result(app: &mut App, ui: &mut Ui, palette: &Palette) {
    let Some(result) = app.active_tab().and_then(|tab| tab.result.as_ref()) else {
        return;
    };
    let summary_line = summary(result);
    let affected = result.affected;
    let duration = result.duration_ms;
    let statements = result.result_sets;
    let truncated = result.truncated;
    Frame::new()
        .fill(palette.panel)
        .inner_margin(Margin::symmetric(10, 8))
        .corner_radius(6)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("Statement completed")
                        .strong()
                        .color(palette.green),
                );
                ui.label(RichText::new(&summary_line).small().color(palette.subtext));
            });
            ui.add_space(4.0);
            ui.label(
                RichText::new(format!(
                    "{} row{} affected · {duration} ms · {statements} statement{}",
                    format_count(affected),
                    if affected == 1 { "" } else { "s" },
                    if statements == 1 { "" } else { "s" }
                ))
                .small()
                .color(palette.muted),
            );
            if truncated {
                ui.label(
                    RichText::new("The configured row limit was reached.")
                        .small()
                        .color(palette.yellow),
                );
            }
            ui.add_space(4.0);
            if ui.button("Copy summary").clicked() {
                let message = app.clipboard.copy(&summary_line);
                app.toast(LogLevel::Info, message);
            }
        });
}

/// Show a compact summary line for a result set (used by tests and tooltips).
pub fn summary(result: &QueryResult) -> String {
    if result.columns.is_empty() {
        format!(
            "{} row{} affected",
            format_count(result.affected),
            if result.affected == 1 { "" } else { "s" }
        )
    } else {
        format!(
            "{} rows × {} columns · {} ms",
            format_count(result.rows.len() as u64),
            result.columns.len(),
            result.duration_ms
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ColumnMeta;

    fn result(columns: Vec<ColumnMeta>) -> QueryResult {
        QueryResult {
            columns,
            rows: Vec::new(),
            affected: 0,
            duration_ms: 12,
            truncated: false,
            sql: String::new(),
            result_sets: 1,
        }
    }

    #[test]
    fn summary_distinguishes_statements() {
        let mut statement = result(Vec::new());
        statement.affected = 3;
        assert_eq!(summary(&statement), "3 rows affected");

        let mut rows = result(vec![ColumnMeta {
            name: "id".to_string(),
            type_name: "int4".to_string(),
        }]);
        rows.rows = vec![vec![Some("1".to_string())]];
        assert_eq!(summary(&rows), "1 rows × 1 columns · 12 ms");
    }
}
