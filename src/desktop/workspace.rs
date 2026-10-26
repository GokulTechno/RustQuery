//! Workspace: tab strip, SQL editor, results grid, console and explorer.

use eframe::egui::{
    self, Align, Button, Color32, Frame, Layout, Margin, RichText, ScrollArea, Sense, TextEdit, Ui,
};

use crate::app::{App, Focus, TxAction, View};
use crate::model::{format_count, LogLevel};
use crate::schema_tree::RowKind;

use super::theme::Palette;
use super::{editor, results};

const TAB_BAR_HEIGHT: f32 = 38.0;

pub fn show(app: &mut App, ui: &mut Ui) {
    let palette = Palette::for_theme(app.theme_kind());

    egui::TopBottomPanel::top("rustquery_tabs")
        .exact_height(TAB_BAR_HEIGHT)
        .frame(
            Frame::new()
                .fill(palette.panel)
                .inner_margin(Margin::symmetric(8, 4)),
        )
        .show_inside(ui, |ui| show_tabs(app, ui, &palette));

    // Console is shown first so it occupies the very bottom; the results
    // panel then stacks directly above it.
    if app.settings.show_logs {
        let max_height = (ui.available_height() * 0.5).max(110.0);
        egui::TopBottomPanel::bottom("rustquery_console")
            .resizable(true)
            .default_height(150.0)
            .height_range(80.0..=max_height)
            .frame(
                Frame::new()
                    .fill(palette.panel)
                    .inner_margin(Margin::symmetric(8, 6)),
            )
            .show_inside(ui, |ui| show_console(app, ui, &palette));
    }

    let available = ui.available_height();
    let rule_ratio = app.settings.editor_ratio.clamp(20, 80) as f32 / 100.0;
    let desired_results = (available * (1.0 - rule_ratio)).clamp(90.0, available * 0.8);
    let show_results = app.results_visible()
        || app
            .active_tab()
            .map(|t| t.last_error.is_some())
            .unwrap_or(false);
    if show_results {
        let response = egui::TopBottomPanel::bottom("rustquery_results")
            .resizable(true)
            .default_height(desired_results)
            .height_range(80.0..=(available * 0.8).max(100.0))
            .frame(
                Frame::new()
                    .fill(palette.bg)
                    .inner_margin(Margin::symmetric(6, 4)),
            )
            .show_inside(ui, |ui| results::show(app, ui, &palette));
        let actual = response.response.rect.height();
        if available > 40.0 {
            let ratio = ((available - actual) / available * 100.0).clamp(20.0, 80.0) as u16;
            if ratio != app.settings.editor_ratio {
                app.settings.editor_ratio = ratio;
            }
        }
    }

    egui::CentralPanel::default()
        .frame(Frame::new().fill(palette.bg))
        .show_inside(ui, |ui| {
            show_toolbar(app, ui, &palette);
            ui.add_space(2.0);
            editor::show(app, ui, &palette);
        });
}

// ---------------------------------------------------------------------------
// Tabs
// ---------------------------------------------------------------------------

fn show_tabs(app: &mut App, ui: &mut Ui, palette: &Palette) {
    let tabs: Vec<(usize, String, bool, bool)> = app
        .tabs
        .iter()
        .enumerate()
        .map(|(index, tab)| {
            (
                index,
                tab.display_title(),
                tab.running.is_some(),
                tab.is_preview,
            )
        })
        .collect();
    let active = app.active_tab;
    let mut activate: Option<usize> = None;
    let mut close: Option<usize> = None;

    ui.horizontal(|ui| {
        ScrollArea::horizontal()
            .id_salt("rustquery_tab_scroll")
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    for (index, title, running, preview) in &tabs {
                        let selected = *index == active;
                        let text = if *running {
                            format!("◌ {title}")
                        } else if *preview {
                            format!("▤ {title}")
                        } else {
                            title.clone()
                        };
                        let response = ui.add(
                            Button::new(RichText::new(text).color(if selected {
                                palette.text
                            } else {
                                palette.subtext
                            }))
                            .selected(selected)
                            .small(),
                        );
                        if response.clicked() {
                            activate = Some(*index);
                        }
                        if response.middle_clicked() {
                            close = Some(*index);
                        }
                        let close_response = ui.add(
                            Button::new(RichText::new("×").color(palette.muted))
                                .small()
                                .frame(false),
                        );
                        if close_response.clicked() {
                            close = Some(*index);
                        }
                        if close_response.on_hover_text("Close tab (Ctrl+W)").clicked() {
                            close = Some(*index);
                        }
                        ui.add_space(2.0);
                    }
                });
            });
        if ui
            .add(
                Button::new(RichText::new("+").color(palette.accent).strong())
                    .small()
                    .frame(false),
            )
            .on_hover_text("New query tab (Ctrl+T)")
            .clicked()
        {
            app.new_query_tab();
            app.focus = Focus::Editor;
        }

        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if let Some(session) = &app.session {
                let color = palette.driver_color(session.info.driver);
                ui.label(
                    RichText::new(format!(
                        "{} · {}",
                        session.info.driver.label(),
                        session.info.current_database
                    ))
                    .color(color)
                    .small(),
                );
            } else {
                ui.label(RichText::new("Not connected").color(palette.muted).small());
            }
        });
    });

    if let Some(index) = activate {
        app.set_active_tab(index);
        app.view = View::Workspace;
        app.focus = Focus::Editor;
    }
    if let Some(index) = close {
        app.request_close_tab_by_index(index);
    }
}

// ---------------------------------------------------------------------------
// Toolbar
// ---------------------------------------------------------------------------

fn show_toolbar(app: &mut App, ui: &mut Ui, palette: &Palette) {
    let connected = app.session.is_some();
    let tx_active = app
        .session
        .as_ref()
        .map(|session| session.tx_active)
        .unwrap_or(false);
    let running = app.current_running();
    let has_result = app.results_visible();

    Frame::new()
        .fill(palette.panel)
        .inner_margin(Margin::symmetric(8, 5))
        .corner_radius(6)
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                if running {
                    if ui
                        .add(
                            Button::new(RichText::new("■ Stop").color(palette.bg))
                                .fill(palette.red),
                        )
                        .on_hover_text("Cancel the running query (Ctrl+C)")
                        .clicked()
                    {
                        app.cancel_current_query();
                    }
                } else if ui
                    .add_enabled(
                        connected,
                        Button::new(RichText::new("▶ Run").color(palette.bg))
                            .fill(palette.accent),
                    )
                    .on_hover_text("Execute selection or buffer (Ctrl+Enter / F5)")
                    .clicked()
                {
                    app.execute_current();
                }

                ui.separator();

                if ui
                    .add_enabled(connected && !tx_active, Button::new("BEGIN"))
                    .on_hover_text("Start a transaction (F8)")
                    .clicked()
                {
                    app.run_transaction(TxAction::Begin);
                }
                if ui
                    .add_enabled(connected && tx_active, Button::new("COMMIT"))
                    .on_hover_text("Commit the open transaction (F9)")
                    .clicked()
                {
                    app.run_transaction(TxAction::Commit);
                }
                if ui
                    .add_enabled(connected && tx_active, Button::new("ROLLBACK"))
                    .on_hover_text("Roll back the open transaction (F10)")
                    .clicked()
                {
                    app.run_transaction(TxAction::Rollback);
                }

                ui.separator();

                if ui
                    .add_enabled(has_result, Button::new("Export"))
                    .on_hover_text("Export CSV, JSON or SQL INSERT (Ctrl+E)")
                    .clicked()
                {
                    app.open_export_dialog();
                }
                if ui
                    .add_enabled(has_result, Button::new("Filter"))
                    .on_hover_text("Filter all result columns (Ctrl+F)")
                    .clicked()
                {
                    app.open_filter_dialog();
                }
                if ui
                    .add_enabled(
                        app.active_tab()
                            .map(|tab| !tab.editor.text().trim().is_empty())
                            .unwrap_or(false),
                        Button::new("Save SQL"),
                    )
                    .on_hover_text("Save the editor buffer to a file (Ctrl+S)")
                    .clicked()
                {
                    app.open_save_query_dialog();
                }
                if ui
                    .button("History")
                    .on_hover_text("Browse query history (Ctrl+H)")
                    .clicked()
                {
                    app.open_history();
                }

                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if let Some(tab) = app.active_tab() {
                        if let Some(running) = &tab.running {
                            let elapsed = running.started.elapsed().as_secs_f32();
                            ui.label(
                                RichText::new(format!("Running {elapsed:.1}s"))
                                    .color(palette.yellow)
                                    .small(),
                            );
                            ui.spinner();
                        } else if let Some(result) = &tab.result {
                            let mut summary = if result.columns.is_empty() {
                                format!(
                                    "{} row{} affected",
                                    format_count(result.affected),
                                    if result.affected == 1 { "" } else { "s" }
                                )
                            } else {
                                format!(
                                    "{} row{} · {} ms",
                                    format_count(result.rows.len() as u64),
                                    if result.rows.len() == 1 { "" } else { "s" },
                                    result.duration_ms
                                )
                            };
                            if result.truncated {
                                summary.push_str(" · limit");
                            }
                            ui.label(RichText::new(summary).color(palette.green).small());
                        } else if let Some(error) = &tab.last_error {
                            ui.label(RichText::new(&error.summary).color(palette.red).small());
                        } else {
                            ui.label(
                                RichText::new(format!(
                                    "{} line{} · row limit {}",
                                    tab.editor.line_count(),
                                    if tab.editor.line_count() == 1 {
                                        ""
                                    } else {
                                        "s"
                                    },
                                    format_count(app.settings.row_limit as u64)
                                ))
                                .color(palette.muted)
                                .small(),
                            );
                        }
                    }
                    if tx_active {
                        ui.label(
                            RichText::new("TX OPEN")
                                .color(palette.yellow)
                                .small()
                                .strong(),
                        );
                    }
                });
            });
        });
}

// ---------------------------------------------------------------------------
// Console
// ---------------------------------------------------------------------------

fn show_console(app: &mut App, ui: &mut Ui, palette: &Palette) {
    ui.horizontal(|ui| {
        ui.label(RichText::new("Console").strong().color(palette.text));
        ui.label(
            RichText::new(format!("{} entries", app.logs.len()))
                .small()
                .color(palette.muted),
        );
        ui.separator();
        let mut follow = app.console.follow;
        if ui.checkbox(&mut follow, "Follow").changed() {
            app.console.follow = follow;
        }
        let mut errors_only = app.console.errors_only;
        if ui.checkbox(&mut errors_only, "Errors only").changed() {
            app.console.errors_only = errors_only;
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.small_button("Clear").clicked() {
                app.logs.clear();
            }
            if ui.small_button("Hide").clicked() {
                app.settings.show_logs = false;
            }
        });
    });
    ui.separator();

    let entries: Vec<_> = app
        .logs
        .iter()
        .filter(|entry| !app.console.errors_only || entry.level == LogLevel::Error)
        .collect();
    ScrollArea::vertical()
        .id_salt("rustquery_console_scroll")
        .stick_to_bottom(app.console.follow)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if entries.is_empty() {
                ui.label(
                    RichText::new("No log entries.")
                        .color(palette.muted)
                        .italics(),
                );
            }
            for entry in entries {
                let color = palette.log_color(entry.level);
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new(entry.at.format("%H:%M:%S").to_string())
                            .monospace()
                            .small()
                            .color(palette.muted),
                    );
                    ui.label(
                        RichText::new(entry.level.label())
                            .monospace()
                            .small()
                            .strong()
                            .color(color),
                    );
                    ui.label(RichText::new(&entry.message).color(palette.text));
                    if let Some(source) = &entry.source {
                        ui.label(
                            RichText::new(format!("[{source}]"))
                                .small()
                                .color(palette.muted),
                        );
                    }
                });
                if let Some(detail) = &entry.detail {
                    for line in detail.lines().take(8) {
                        ui.label(
                            RichText::new(line)
                                .monospace()
                                .small()
                                .color(palette.subtext),
                        );
                    }
                }
            }
        });
}

// ---------------------------------------------------------------------------
// Explorer
// ---------------------------------------------------------------------------

pub fn show_sidebar(app: &mut App, ui: &mut Ui) {
    let palette = Palette::for_theme(app.theme_kind());
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new("Explorer").strong().color(palette.text));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let connected = app.session.is_some();
            if ui
                .add_enabled(connected, Button::new("⟳").frame(false))
                .on_hover_text("Refresh schema")
                .clicked()
            {
                app.refresh_schema();
            }
            if ui
                .add_enabled(connected, Button::new("+").frame(false))
                .on_hover_text("New query tab")
                .clicked()
            {
                app.new_query_tab();
                app.focus = Focus::Editor;
            }
        });
    });

    if app.session.is_none() {
        ui.add_space(8.0);
        ui.label(
            RichText::new("Connect to a database to browse its schema.")
                .color(palette.muted)
                .small(),
        );
        if ui.button("Open connections").clicked() {
            app.open_dashboard();
        }
        ui.separator();
        show_recent_history(app, ui, &palette);
        return;
    }

    ui.horizontal(|ui| {
        let mut filter = app.tree.filter.clone();
        let response = ui.add(
            TextEdit::singleline(&mut filter)
                .hint_text("Filter schema…")
                .desired_width(f32::INFINITY)
                .text_color(palette.text),
        );
        if response.gained_focus() || response.has_focus() {
            app.focus = Focus::Sidebar;
        }
        if response.changed() {
            app.tree.set_filter(filter.clone());
        }
        if !filter.is_empty() && ui.small_button("×").clicked() {
            app.tree.set_filter(String::new());
        }
    });
    if app.schema_loading {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(
                RichText::new("Loading schema…")
                    .small()
                    .color(palette.muted),
            );
        });
    }
    ui.separator();

    let rows = app.tree.rows.clone();
    let selected = app.tree.selected_id.clone();
    let mut toggle: Option<usize> = None;
    let mut activate: Option<usize> = None;
    let mut select_only: Option<usize> = None;
    let mut context_copy: Option<usize> = None;
    let mut context_preview: Option<usize> = None;

    ScrollArea::vertical()
        .id_salt("rustquery_tree_scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if rows.is_empty() {
                ui.add_space(6.0);
                ui.label(
                    RichText::new(if app.tree.is_filtered() {
                        "No schema objects match the filter."
                    } else {
                        "No schema objects loaded."
                    })
                    .color(palette.muted)
                    .small(),
                );
            }
            for (index, row) in rows.iter().enumerate() {
                let is_selected = selected.as_deref() == Some(row.id.as_str());
                let indent = row.depth as f32 * 13.0;
                let (bg, text_color) = if is_selected {
                    (palette.elevated, palette.text)
                } else {
                    (Color32::TRANSPARENT, palette.text)
                };
                let response = Frame::new()
                    .fill(bg)
                    .corner_radius(4)
                    .inner_margin(Margin::symmetric(4, 2))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.add_space(indent);
                            if row.expandable {
                                let marker = if row.loading {
                                    "◌"
                                } else if row.expanded {
                                    "▾"
                                } else {
                                    "▸"
                                };
                                let button = ui.add(
                                    Button::new(
                                        RichText::new(marker)
                                            .color(if row.error {
                                                palette.red
                                            } else {
                                                palette.accent
                                            })
                                            .monospace(),
                                    )
                                    .frame(false)
                                    .small(),
                                );
                                if button.clicked() {
                                    toggle = Some(index);
                                }
                            } else {
                                ui.add_space(11.0);
                            }
                            let icon = match row.kind {
                                RowKind::Root => "▣",
                                RowKind::Schema => "◫",
                                RowKind::Folder => "▤",
                                RowKind::Table => "▦",
                                RowKind::View => "◉",
                                RowKind::Column => "•",
                                RowKind::Index => "#",
                                RowKind::ForeignKey => "↗",
                            };
                            ui.label(
                                RichText::new(icon).small().color(
                                    palette.driver_color(
                                        app.session
                                            .as_ref()
                                            .map(|s| s.info.driver)
                                            .unwrap_or(crate::model::Driver::Postgres),
                                    ),
                                ),
                            );
                            let label = ui.add(
                                egui::Label::new(
                                    RichText::new(&row.label)
                                        .color(text_color)
                                        .small()
                                        .monospace(),
                                )
                                .sense(Sense::click()),
                            );
                            if !row.detail.is_empty() {
                                ui.label(RichText::new(&row.detail).small().color(palette.muted));
                            }
                            if label.clicked() {
                                select_only = Some(index);
                                if row.expandable {
                                    toggle = Some(index);
                                } else {
                                    activate = Some(index);
                                }
                            }
                            if label.double_clicked() {
                                activate = Some(index);
                            }
                            label.context_menu(|ui| {
                                if ui.button("Copy name").clicked() {
                                    context_copy = Some(index);
                                    ui.close_menu();
                                }
                                if matches!(row.kind, RowKind::Table | RowKind::View)
                                    && ui.button("Preview data").clicked()
                                {
                                    context_preview = Some(index);
                                    ui.close_menu();
                                }
                            });
                        });
                    })
                    .response;
                if response.clicked() {
                    select_only = Some(index);
                }
                if response.double_clicked() {
                    activate = Some(index);
                }
            }
        });

    if let Some(index) = select_only {
        app.tree.select_row(index);
        app.focus = Focus::Sidebar;
    }
    if let Some(index) = toggle {
        app.toggle_tree_row(index);
        app.focus = Focus::Sidebar;
    }
    if let Some(index) = activate {
        app.activate_tree_row(index);
        app.focus = Focus::Sidebar;
    }
    if let Some(index) = context_copy {
        app.tree.select_row(index);
        app.copy_tree_label();
    }
    if let Some(index) = context_preview {
        app.tree.select_row(index);
        app.handle_sidebar_activate();
    }
}

fn show_recent_history(app: &mut App, ui: &mut Ui, palette: &Palette) {
    ui.add_space(6.0);
    ui.label(
        RichText::new("Recent queries")
            .strong()
            .color(palette.subtext),
    );
    if app.history.is_empty() {
        ui.label(
            RichText::new("Query history appears here after you run SQL.")
                .small()
                .color(palette.muted),
        );
        return;
    }
    ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
        let entries: Vec<(usize, String, String)> = app
            .history
            .iter()
            .take(12)
            .enumerate()
            .map(|(index, entry)| {
                (
                    index,
                    entry.at.format("%H:%M").to_string(),
                    entry.snippet(48),
                )
            })
            .collect();
        for (index, time, snippet) in entries {
            let color = if app.history[index].success {
                palette.text
            } else {
                palette.red
            };
            if ui
                .add(
                    egui::Label::new(
                        RichText::new(format!("{time}  {snippet}"))
                            .small()
                            .monospace()
                            .color(color),
                    )
                    .sense(Sense::click())
                    .truncate(),
                )
                .clicked()
            {
                app.load_history_entry(index);
            }
        }
    });
}
