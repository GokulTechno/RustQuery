//! Workspace: tab strip, SQL editor, results grid, console and explorer.

use std::collections::HashSet;

use eframe::egui::{
    self, Align, Button, Color32, Frame, Layout, Margin, RichText, ScrollArea, Sense, Stroke,
    TextEdit, Ui,
};
use uuid::Uuid;

use crate::app::{App, Focus, TxAction, View};
use crate::explorer::CatalogKind;
use crate::model::{format_count, LogLevel};
use crate::schema_tree::RowKind;

use super::theme::{metrics, Palette};
use super::{editor, results};

pub fn show(app: &mut App, ui: &mut Ui) {
    let palette = Palette::for_theme(app.theme_kind());
    let ui_metrics = metrics(app.settings.ui_scale_percent);

    egui::TopBottomPanel::top("rustquery_tabs")
        .exact_height(ui_metrics.tab_bar_height)
        .frame(
            Frame::new()
                .fill(palette.panel)
                .inner_margin(Margin::symmetric(8, 4)),
        )
        .show_inside(ui, |ui| {
            ui.with_layout(
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| show_tabs(app, ui, &palette),
            );
        });

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
                    ui.spacing_mut().item_spacing.x = 6.0;
                    for (index, title, running, preview) in &tabs {
                        let selected = *index == active;
                        let text = if *running {
                            format!("◌ {title}")
                        } else if *preview {
                            format!("▤ {title}")
                        } else {
                            title.clone()
                        };
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 0.0;
                            let response = ui.add(
                                Button::new(RichText::new(text).color(if selected {
                                    palette.text
                                } else {
                                    palette.subtext
                                }))
                                    .fill(if selected {
                                        palette.selection
                                    } else {
                                        Color32::TRANSPARENT
                                    })
                                    .stroke(
                                        Stroke::new(
                                            if selected { 1.0 } else { 0.0 },
                                            palette.accent.gamma_multiply(0.75),
                                        ),
                                    ),
                            );
                            if response.clicked() {
                                activate = Some(*index);
                            }
                            if response.middle_clicked() {
                                close = Some(*index);
                            }
                            let close_response = ui.add(
                                Button::new(RichText::new("×").color(palette.muted)).frame(false),
                            );
                            if close_response
                                .on_hover_text("Close tab (Ctrl+W)")
                                .clicked()
                            {
                                close = Some(*index);
                            }
                        });
                    }
                });
            });
        if ui
            .add(
                Button::new(RichText::new("+").color(palette.accent).strong()).frame(false),
            )
            .on_hover_text("New query tab (Ctrl+T)")
            .clicked()
        {
            app.new_query_tab();
            app.focus = Focus::Editor;
        }

        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if app.connect.is_some() {
                ui.spinner();
                ui.label(
                    RichText::new("Connecting…")
                        .small()
                        .color(palette.yellow),
                );
            }
            if let Some(session) = &app.session {
                let color = palette.driver_color(session.info.driver);
                let label = format!(
                    "{} · {} · {}",
                    session.info.profile_name,
                    session.info.driver.label(),
                    session.info.current_database
                );
                if ui
                    .add(
                        Button::new(RichText::new(label).color(color))
                            .frame(false),
                    )
                    .on_hover_text("Switch connection (Ctrl+Shift+O)")
                    .clicked()
                {
                    app.open_connection_picker();
                }
            } else if ui
                .add(Button::new(RichText::new("Connect…").color(palette.accent)).frame(false))
                .on_hover_text("Choose a connection")
                .clicked()
            {
                app.open_connection_picker();
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
                            Button::new(RichText::new("■ Stop").color(palette.on_destructive()))
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
                        Button::new(RichText::new("▶ Run").color(palette.on_accent()))
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
                if ui
                    .add_enabled(connected, Button::new("Switch DB"))
                    .on_hover_text("Connect to another saved profile (Ctrl+Shift+O)")
                    .clicked()
                {
                    app.open_connection_picker();
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
                .on_hover_text("Refresh schema for the active connection")
                .clicked()
            {
                app.refresh_schema();
            }
            if ui
                .small_button("Folder")
                .on_hover_text("Create a folder for connections")
                .clicked()
            {
                app.prompt_new_folder();
            }
            if ui
                .small_button("Connection")
                .on_hover_text("Save a new connection")
                .clicked()
            {
                app.open_profile_form(None);
            }
        });
    });

    ui.horizontal(|ui| {
        let mut filter = app.catalog.filter.clone();
        let response = ui.add(
            TextEdit::singleline(&mut filter)
                .hint_text("Filter connections…")
                .desired_width(f32::INFINITY)
                .text_color(palette.text),
        );
        if response.gained_focus() || response.has_focus() {
            app.focus = Focus::Sidebar;
        }
        if response.changed() {
            app.catalog.filter = filter.clone();
        }
        if !filter.is_empty() && ui.small_button("×").clicked() {
            app.catalog.filter.clear();
        }
    });
    ui.label(
        RichText::new("Expand a host to connect. The session stays open until you quit.")
            .small()
            .color(palette.muted),
    );
    ui.separator();

    let live: HashSet<Uuid> = app
        .profiles
        .iter()
        .filter(|profile| app.profile_is_live(profile.id))
        .map(|profile| profile.id)
        .collect();
    let connecting = app.connect.as_ref().map(|connect| connect.profile.id);
    let rows = app.catalog.rows(&app.profiles, &live, connecting);
    let selected = app.catalog.selected.clone();
    let folders: Vec<(Uuid, String)> = app
        .catalog
        .folders
        .iter()
        .map(|folder| (folder.id, folder.name.clone()))
        .collect();
    let needle = app.catalog.filter.trim().to_lowercase();

    let mut toggle_catalog: Option<String> = None;
    let mut select_catalog: Option<String> = None;
    let mut schema_toggle: Option<(Uuid, usize)> = None;
    let mut schema_activate: Option<(Uuid, usize)> = None;
    let mut schema_copy: Option<(Uuid, usize)> = None;
    let mut schema_preview: Option<(Uuid, usize)> = None;
    let mut new_in_folder: Option<Option<Uuid>> = None;
    let mut rename_folder: Option<Uuid> = None;
    let mut delete_folder: Option<Uuid> = None;
    let mut edit_profile: Option<Uuid> = None;
    let mut delete_profile: Option<Uuid> = None;
    let mut move_profile: Option<(Uuid, Option<Uuid>)> = None;
    let mut disconnect_profile: Option<Uuid> = None;

    ScrollArea::vertical()
        .id_salt("rustquery_tree_scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if rows.is_empty() {
                ui.add_space(6.0);
                ui.label(
                    RichText::new(if app.catalog.filter.trim().is_empty() {
                        "No saved connections yet. Use Connection to add one, or Folder to organize them."
                    } else {
                        "No connections match the filter."
                    })
                    .color(palette.muted)
                    .small(),
                );
            }
            for row in &rows {
                let is_selected = selected.as_deref() == Some(row.id.as_str());
                let profile_id = row.profile_id;
                let row_id = row.id.clone();
                let expanded = row.expanded;
                let kind = row.kind;
                draw_explorer_row(
                    ui,
                    &palette,
                    row.depth,
                    row.expandable,
                    row.loading,
                    row.error,
                    expanded,
                    is_selected,
                    match kind {
                        CatalogKind::Folder => "▤",
                        CatalogKind::Connection => "▣",
                        CatalogKind::Endpoint => "◎",
                        CatalogKind::Database => "◫",
                    },
                    &row.label,
                    &row.detail,
                    &mut |hit| {
                        select_catalog = Some(row_id.clone());
                        if !matches!(hit, ExplorerHit::DoubleClick) && row.expandable {
                            toggle_catalog = Some(row_id.clone());
                        }
                    },
                    &mut |ui| match kind {
                        CatalogKind::Folder => {
                            if ui.button("New connection here").clicked() {
                                new_in_folder = Some(row.folder_id);
                                ui.close_menu();
                            }
                            if ui.button("Rename folder").clicked() {
                                rename_folder = row.folder_id;
                                ui.close_menu();
                            }
                            if ui.button("Remove folder").clicked() {
                                delete_folder = row.folder_id;
                                ui.close_menu();
                            }
                        }
                        CatalogKind::Connection => {
                            let Some(profile_id) = profile_id else {
                                return;
                            };
                            if ui.button("Edit connection").clicked() {
                                edit_profile = Some(profile_id);
                                ui.close_menu();
                            }
                            ui.menu_button("Move to folder", |ui| {
                                if ui.button("Top level").clicked() {
                                    move_profile = Some((profile_id, None));
                                    ui.close_menu();
                                }
                                for (id, name) in &folders {
                                    if ui.button(name).clicked() {
                                        move_profile = Some((profile_id, Some(*id)));
                                        ui.close_menu();
                                    }
                                }
                                if folders.is_empty() {
                                    ui.label(RichText::new("No folders yet").small().weak());
                                }
                            });
                            if app.profile_is_live(profile_id) && ui.button("Disconnect").clicked()
                            {
                                disconnect_profile = Some(profile_id);
                                ui.close_menu();
                            }
                            if ui.button("Delete connection").clicked() {
                                delete_profile = Some(profile_id);
                                ui.close_menu();
                            }
                        }
                        CatalogKind::Endpoint | CatalogKind::Database => {
                            let Some(profile_id) = profile_id else {
                                return;
                            };
                            if app.profile_is_live(profile_id) && ui.button("Disconnect").clicked()
                            {
                                disconnect_profile = Some(profile_id);
                                ui.close_menu();
                            }
                        }
                    },
                );

                if kind == CatalogKind::Database && expanded {
                    let Some(profile_id) = profile_id else {
                        continue;
                    };
                    if app.schema_loading_for(profile_id) {
                        ui.horizontal(|ui| {
                            ui.add_space((row.depth as f32 + 1.0) * 13.0);
                            ui.spinner();
                            ui.label(
                                RichText::new("Loading schema…")
                                    .small()
                                    .color(palette.muted),
                            );
                        });
                    }
                    let schema_rows = app
                        .schema_rows_for(profile_id)
                        .map(|rows| rows.to_vec())
                        .unwrap_or_default();
                    let selected_schema = app.tree_selection_for(profile_id);
                    let connection_matched = needle.is_empty()
                        || row.label.to_lowercase().contains(&needle)
                        || rows.iter().any(|other| {
                            other.profile_id == Some(profile_id)
                                && other.kind != CatalogKind::Database
                                && (other.label.to_lowercase().contains(&needle)
                                    || other.detail.to_lowercase().contains(&needle))
                        });
                    for (index, schema) in schema_rows.iter().enumerate() {
                        if schema.kind == RowKind::Root {
                            continue;
                        }
                        let matches_schema = schema.label.to_lowercase().contains(&needle)
                            || schema.detail.to_lowercase().contains(&needle);
                        if !needle.is_empty() && !connection_matched && !matches_schema {
                            continue;
                        }
                        draw_explorer_row(
                            ui,
                            &palette,
                            row.depth + schema.depth,
                            schema.expandable,
                            schema.loading,
                            schema.error,
                            schema.expanded,
                            selected_schema.as_deref() == Some(schema.id.as_str()),
                            match schema.kind {
                                RowKind::Root => "▣",
                                RowKind::Schema => "◫",
                                RowKind::Folder => "▤",
                                RowKind::Table => "▦",
                                RowKind::View => "◉",
                                RowKind::Column => "•",
                                RowKind::Index => "#",
                                RowKind::ForeignKey => "↗",
                            },
                            &schema.label,
                            &schema.detail,
                            &mut |hit| match hit {
                                ExplorerHit::DoubleClick => {
                                    schema_activate = Some((profile_id, index));
                                }
                                ExplorerHit::Marker | ExplorerHit::Click => {
                                    if schema.expandable {
                                        schema_toggle = Some((profile_id, index));
                                    } else {
                                        schema_activate = Some((profile_id, index));
                                    }
                                }
                            },
                            &mut |ui| {
                                if ui.button("Copy name").clicked() {
                                    schema_copy = Some((profile_id, index));
                                    ui.close_menu();
                                }
                                if matches!(schema.kind, RowKind::Table | RowKind::View)
                                    && ui.button("Preview data").clicked()
                                {
                                    schema_preview = Some((profile_id, index));
                                    ui.close_menu();
                                }
                            },
                        );
                    }
                }
            }
        });

    if let Some(id) = select_catalog {
        app.catalog.select(id);
        app.focus = Focus::Sidebar;
    }
    if let Some(id) = toggle_catalog {
        app.catalog.select(id.clone());
        app.toggle_catalog_row(&id);
        app.focus = Focus::Sidebar;
    }
    if let Some((profile_id, index)) = schema_toggle {
        if app.use_schema_row(profile_id) {
            app.toggle_tree_row(index);
            app.focus = Focus::Sidebar;
        }
    }
    if let Some((profile_id, index)) = schema_activate {
        if app.use_schema_row(profile_id) {
            app.activate_tree_row(index);
            app.focus = Focus::Sidebar;
        }
    }
    if let Some((profile_id, index)) = schema_copy {
        if app.use_schema_row(profile_id) {
            app.tree.select_row(index);
            app.copy_tree_label();
        }
    }
    if let Some((profile_id, index)) = schema_preview {
        if app.use_schema_row(profile_id) {
            app.tree.select_row(index);
            app.handle_sidebar_activate();
        }
    }
    if let Some(folder_id) = new_in_folder {
        app.open_profile_form_in_folder(folder_id);
    }
    if let Some(id) = rename_folder {
        app.prompt_rename_folder(id);
    }
    if let Some(id) = delete_folder {
        app.confirm_delete_folder(id);
    }
    if let Some(id) = edit_profile {
        if let Some(profile) = app.profiles.iter().find(|profile| profile.id == id).cloned() {
            app.open_profile_form(Some(&profile));
        }
    }
    if let Some(id) = delete_profile {
        app.dashboard.selected = app
            .profiles
            .iter()
            .position(|profile| profile.id == id)
            .unwrap_or(0);
        app.confirm_delete_selected_profile();
    }
    if let Some((profile_id, folder_id)) = move_profile {
        app.move_profile_to_folder(profile_id, folder_id);
    }
    if let Some(profile_id) = disconnect_profile {
        app.disconnect_profile(profile_id);
    }

    if !app.profiles.iter().any(|profile| app.profile_is_live(profile.id)) {
        ui.separator();
        show_recent_history(app, ui, &palette);
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ExplorerHit {
    Marker,
    Click,
    DoubleClick,
}

fn draw_explorer_row(
    ui: &mut Ui,
    palette: &Palette,
    depth: u16,
    expandable: bool,
    loading: bool,
    error: bool,
    expanded: bool,
    selected: bool,
    icon: &str,
    label: &str,
    detail: &str,
    on_activate: &mut dyn FnMut(ExplorerHit),
    context: &mut dyn FnMut(&mut Ui),
) {
    let indent = depth as f32 * 13.0;
    let (bg, text_color) = if selected {
        (palette.selection, palette.text)
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
                if expandable {
                    let marker = if loading {
                        "◌"
                    } else if expanded {
                        "▾"
                    } else {
                        "▸"
                    };
                    let button = ui.add(
                        Button::new(
                            RichText::new(marker)
                                .color(if error { palette.red } else { palette.accent })
                                .monospace(),
                        )
                        .frame(false)
                        .small(),
                    );
                    if button.clicked() {
                        on_activate(ExplorerHit::Marker);
                    }
                } else {
                    ui.add_space(11.0);
                }
                ui.label(
                    RichText::new(icon)
                        .small()
                        .color(if error { palette.red } else { palette.accent }),
                );
                let text = ui.add(
                    egui::Label::new(
                        RichText::new(label)
                            .color(text_color)
                            .small()
                            .monospace(),
                    )
                    .sense(Sense::click()),
                );
                if !detail.is_empty() {
                    ui.label(
                        RichText::new(detail).small().color(if error {
                            palette.red
                        } else {
                            palette.muted
                        }),
                    );
                }
                if text.double_clicked() {
                    on_activate(ExplorerHit::DoubleClick);
                } else if text.clicked() {
                    on_activate(ExplorerHit::Click);
                }
                text.context_menu(|ui| context(ui));
            });
        })
        .response;
    let _ = response;
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
