//! Modal dialogs built on top of the existing [`App`] state machines.
//!
//! This is the desktop presentation layer for the application's modal state.
//! Buttons call the same public actions as the keyboard handlers, so
//! validation, persistence and worker orchestration stay in one place.

use eframe::egui::{
    self, Align, Button, ComboBox, Frame, Grid, Id, Key, Layout, Margin, RichText, ScrollArea,
    Stroke, TextEdit, Ui,
};

use crate::app::{
    App, ConnectionPicker, ExportDialog, FilterDialog, InputPurpose, Modal, ProfileForm,
    SettingsDialog,
};
use crate::config::{Settings, SettingsField};
use crate::export::{ExportFormat, ExportScope};
use crate::model::{format_count, Driver, LogLevel, SslMode};

use super::theme::Palette;

const MODAL_IDENTITY: &str = "rustquery_modal_identity";

pub fn show(app: &mut App, ctx: &egui::Context) {
    let key = Id::new(MODAL_IDENTITY);
    let Some(modal) = app.modal.take() else {
        ctx.data_mut(|data| data.insert_temp(key, String::new()));
        return;
    };
    let identity = modal_identity(&modal);
    let previous = ctx
        .data(|data| data.get_temp::<String>(key))
        .unwrap_or_default();
    let new_modal = previous != identity;
    ctx.data_mut(|data| data.insert_temp(key, identity));
    let palette = Palette::for_theme(app.theme_kind());
    match modal {
        Modal::Help { scroll } => help(app, ctx, &palette, scroll),
        Modal::Profile(form) => profile(app, ctx, &palette, form, new_modal),
        Modal::Confirm(dialog) => confirm(app, ctx, &palette, dialog),
        Modal::Export(dialog) => export(app, ctx, &palette, dialog, new_modal),
        Modal::Filter(dialog) => filter(app, ctx, &palette, dialog, new_modal),
        Modal::Cell(dialog) => cell(app, ctx, &palette, dialog),
        Modal::History(browser) => history(app, ctx, &palette, browser, new_modal),
        Modal::Settings(dialog) => settings(app, ctx, &palette, dialog, new_modal),
        Modal::Input(dialog) => input(app, ctx, &palette, dialog, new_modal),
        Modal::Info(dialog) => info(app, ctx, &palette, dialog),
        Modal::ConnectionPicker(picker) => connection_picker(app, ctx, &palette, picker, new_modal),
    }
}

fn modal_identity(modal: &Modal) -> String {
    match modal {
        Modal::Help { .. } => "help".to_string(),
        Modal::Profile(form) => format!("profile:{}", form.token),
        Modal::Confirm(dialog) => format!("confirm:{}", dialog.title),
        Modal::Export(dialog) => format!("export:{}", dialog.tab_id),
        Modal::Filter(dialog) => format!("filter:{}", dialog.tab_id),
        Modal::Cell(dialog) => format!("cell:{}:{}", dialog.row_number, dialog.column),
        Modal::History(_) => "history".to_string(),
        Modal::Settings(_) => "settings".to_string(),
        Modal::Input(dialog) => format!("input:{}", dialog.title),
        Modal::Info(dialog) => format!("info:{}", dialog.title),
        Modal::ConnectionPicker(_) => "connection_picker".to_string(),
    }
}

/// Build a centered modal with the application palette. Returns `true` when
/// the backdrop was clicked or Escape was pressed while this was the topmost
/// modal.
fn shell(
    ctx: &egui::Context,
    id: &'static str,
    palette: &Palette,
    width: f32,
    add: impl FnOnce(&mut Ui),
) -> bool {
    let response = egui::Modal::new(Id::new(id))
        .backdrop_color(palette.modal_scrim())
        .frame(
            Frame::new()
                .fill(palette.panel)
                .stroke(Stroke::new(1.0_f32, palette.border))
                .corner_radius(10)
                .inner_margin(Margin::symmetric(18, 14)),
        )
        .show(ctx, |ui| {
            ui.set_width(width);
            add(ui);
        });
    response.should_close()
}

fn heading(ui: &mut Ui, palette: &Palette, title: &str, subtitle: &str) {
    ui.label(RichText::new(title).size(19.0).strong().color(palette.text));
    if !subtitle.is_empty() {
        ui.label(RichText::new(subtitle).small().color(palette.muted));
    }
    ui.add_space(6.0);
}

fn primary(ui: &mut Ui, palette: &Palette, text: &str) -> egui::Response {
    ui.add(
        Button::new(RichText::new(text).color(palette.on_accent()).strong())
            .fill(palette.accent)
            .min_size(egui::vec2(88.0, 28.0)),
    )
}

fn secondary(ui: &mut Ui, text: &str) -> egui::Response {
    ui.add(Button::new(text).min_size(egui::vec2(78.0, 28.0)))
}

fn destructive(ui: &mut Ui, palette: &Palette, text: &str) -> egui::Response {
    ui.add(
        Button::new(RichText::new(text).color(palette.on_destructive()).strong())
            .fill(palette.red)
            .min_size(egui::vec2(88.0, 28.0)),
    )
}

fn footer_buttons(ui: &mut Ui, _palette: &Palette, right: impl FnOnce(&mut Ui)) {
    ui.add_space(8.0);
    ui.separator();
    ui.add_space(6.0);
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| right(ui));
}

// ---------------------------------------------------------------------------
// Help
// ---------------------------------------------------------------------------

const HELP_SECTIONS: &[(&str, &[(&str, &str)])] = &[
    (
        "Global",
        &[
            ("Ctrl+Q", "quit (confirmation when work is running)"),
            ("F1", "keyboard reference"),
            ("F2", "dashboard / workspace"),
            (
                "F3 / F4 / F6 / F7",
                "focus explorer / editor / results / console",
            ),
            ("F11 / Ctrl+,", "settings"),
            ("F12", "cycle colour theme"),
            ("Ctrl+H", "query history"),
            ("Ctrl+D", "disconnect"),
            ("Esc", "close the current dialog"),
        ],
    ),
    (
        "Connections",
        &[
            ("Ctrl+Shift+N", "new connection profile"),
            ("Ctrl+Shift+O", "switch connection (while connected)"),
            ("Enter", "connect to the selected profile"),
            ("Double-click", "connect to a profile"),
            ("Ctrl+O / Ctrl+S", "open / save a SQL file"),
            ("Ctrl+E", "export the current result set"),
        ],
    ),
    (
        "SQL editor",
        &[
            ("F5 / Ctrl+Enter", "execute selection or whole buffer"),
            ("Ctrl+C", "cancel the running query"),
            ("Ctrl+T / Ctrl+W", "new tab / close tab"),
            ("Ctrl+Tab / Ctrl+Shift+Tab", "next / previous tab"),
            ("Ctrl+Space", "schema and keyword completion"),
            ("Ctrl+Z / Ctrl+Y", "undo / redo"),
            ("Ctrl+/", "toggle -- line comment"),
            ("Tab / Shift+Tab", "indent / outdent"),
            ("Ctrl+Left / Ctrl+Right", "move by word"),
            ("Ctrl+A", "select all"),
        ],
    ),
    (
        "Result grid",
        &[
            ("Arrow keys", "move the selected cell"),
            ("Home / End", "first / last column"),
            ("Ctrl+Home / Ctrl+End", "first / last row"),
            ("PgUp / PgDn", "page through rows"),
            ("s", "cycle sort on the selected column"),
            ("Shift+S", "clear sorting"),
            ("Ctrl+F", "filter all columns"),
            ("Enter", "open the full cell value"),
            ("c / y", "copy cell / row"),
            ("Ctrl+Y", "copy the visible result set"),
        ],
    ),
    (
        "Transactions",
        &[
            ("F8", "BEGIN"),
            ("F9", "COMMIT"),
            ("F10", "ROLLBACK"),
            ("Status bar", "shows when a transaction is open"),
        ],
    ),
    (
        "Mouse",
        &[
            (
                "Click",
                "select tabs, toolbar buttons, tree rows and grid cells",
            ),
            (
                "Double-click",
                "preview a table, open a cell, connect to a profile",
            ),
            ("Header click", "sort by column"),
            ("Middle-click tab", "close the tab"),
        ],
    ),
];

fn help_text() -> String {
    let mut text = String::from("RustQuery keyboard reference\n\n");
    for (section, entries) in HELP_SECTIONS {
        text.push_str(section);
        text.push('\n');
        for (keys, action) in *entries {
            text.push_str(&format!("  {keys:<26} {action}\n"));
        }
        text.push('\n');
    }
    text
}

fn help(app: &mut App, ctx: &egui::Context, palette: &Palette, _scroll: u16) {
    let mut copy = false;
    let mut close_now = false;
    let close = shell(ctx, "rustquery_help", palette, 640.0, |ui| {
        heading(
            ui,
            palette,
            "Help · keyboard reference",
            "Every action is also available from the menu bar.",
        );
        ScrollArea::vertical()
            .id_salt("rustquery_help_scroll")
            .max_height(420.0)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for (section, entries) in HELP_SECTIONS {
                    ui.add_space(4.0);
                    ui.label(RichText::new(*section).strong().color(palette.accent));
                    Grid::new(("help_grid", *section))
                        .num_columns(2)
                        .spacing([18.0, 3.0])
                        .show(ui, |ui| {
                            for (keys, action) in *entries {
                                ui.label(
                                    RichText::new(*keys)
                                        .monospace()
                                        .small()
                                        .color(palette.yellow),
                                );
                                ui.label(RichText::new(*action).small().color(palette.subtext));
                                ui.end_row();
                            }
                        });
                }
            });
        footer_buttons(ui, palette, |ui| {
            if primary(ui, palette, "Close").clicked() {
                close_now = true;
            }
            if ui.button("Copy reference").clicked() {
                copy = true;
            }
        });
    });
    if copy {
        let message = app.clipboard.copy(&help_text());
        app.toast(LogLevel::Info, message);
    }
    if close || close_now {
        return;
    }
    app.modal = Some(Modal::Help { scroll: 0 });
}

// ---------------------------------------------------------------------------
// Connection picker (switch while connected)
// ---------------------------------------------------------------------------

fn connection_picker(
    app: &mut App,
    ctx: &egui::Context,
    palette: &Palette,
    mut picker: ConnectionPicker,
    new_modal: bool,
) {
    let max = app.profiles.len().saturating_sub(1);
    let up = !new_modal && ctx.input(|input| input.key_pressed(Key::ArrowUp));
    let down = !new_modal && ctx.input(|input| input.key_pressed(Key::ArrowDown));
    if up {
        picker.selected = picker.selected.saturating_sub(1);
    }
    if down {
        picker.selected = (picker.selected + 1).min(max);
    }
    let enter = !new_modal && ctx.input(|input| input.key_pressed(Key::Enter));

    let active_id = app.session.as_ref().map(|session| session.info.profile_id);
    let busy = app.connect.is_some();
    let mut connect_index: Option<usize> = None;
    let mut manage = false;
    let mut new_profile = false;
    let mut close_now = false;

    let close = shell(ctx, "rustquery_connections", palette, 560.0, |ui| {
        heading(
            ui,
            palette,
            "Switch connection",
            "SQL editor tabs are kept. Result grids are cleared when the new session opens.",
        );
        if app.profiles.is_empty() {
            ui.label(
                RichText::new("No saved profiles yet.")
                    .small()
                    .color(palette.muted),
            );
        } else {
            ScrollArea::vertical()
                .id_salt("rustquery_connection_picker_scroll")
                .max_height(320.0)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for (index, profile) in app.profiles.iter().enumerate() {
                        let selected = index == picker.selected;
                        let is_active = active_id == Some(profile.id);
                        let label = format!(
                            "{}  {}  {}",
                            profile.driver.label(),
                            profile.name,
                            profile.describe()
                        );
                        let response = ui.add(
                            egui::Label::new(
                                RichText::new(if is_active {
                                    format!("● {label}")
                                } else {
                                    label
                                })
                                .monospace()
                                .small()
                                .color(if selected {
                                    palette.text
                                } else if is_active {
                                    palette.green
                                } else {
                                    palette.subtext
                                }),
                            )
                            .sense(egui::Sense::click())
                            .truncate(),
                        );
                        if response.clicked() {
                            picker.selected = index;
                        }
                        if response.double_clicked() {
                            picker.selected = index;
                            connect_index = Some(index);
                        }
                    }
                });
        }
        footer_buttons(ui, palette, |ui| {
            if primary(ui, palette, "Connect").clicked() {
                connect_index = Some(picker.selected.min(max));
            }
            if secondary(ui, "Cancel").clicked() {
                close_now = true;
            }
            if ui.button("Dashboard").clicked() {
                manage = true;
            }
            if ui.button("New connection").clicked() {
                new_profile = true;
            }
        });
    });

    if enter {
        connect_index = Some(picker.selected.min(max));
    }
    if let Some(index) = connect_index {
        app.connect_to_profile_at(index);
        return;
    }
    if manage {
        app.open_dashboard();
        return;
    }
    if new_profile {
        app.open_profile_form(None);
        return;
    }
    if close || close_now {
        return;
    }
    if busy {
        return;
    }
    app.modal = Some(Modal::ConnectionPicker(picker));
}

// ---------------------------------------------------------------------------
// Profile form
// ---------------------------------------------------------------------------

fn profile(
    app: &mut App,
    ctx: &egui::Context,
    palette: &Palette,
    mut form: ProfileForm,
    new_modal: bool,
) {
    let existing = form
        .edit_id
        .map(|id| app.profiles.iter().any(|profile| profile.id == id))
        .unwrap_or(false);
    let mut action: Option<ProfileAction> = None;

    let close = shell(ctx, "rustquery_profile", palette, 560.0, |ui| {
        heading(
            ui,
            palette,
            if existing {
                "Edit connection"
            } else {
                "New connection"
            },
            "Profiles are stored locally; passwords never leave this machine except to the database.",
        );

        let id = Id::new(("rustquery_profile_form", form.token));
        Grid::new((id, "grid"))
            .num_columns(2)
            .spacing([14.0, 8.0])
            .show(ui, |ui| {
                ui.label(RichText::new("Driver").color(palette.subtext));
                let mut driver = form.driver;
                ComboBox::from_id_salt((id, "driver"))
                    .selected_text(driver.label())
                    .width(220.0)
                    .show_ui(ui, |ui| {
                        for candidate in Driver::ALL {
                            ui.selectable_value(&mut driver, candidate, candidate.label());
                        }
                    });
                if driver != form.driver {
                    form.select_driver(driver);
                }
                ui.end_row();

                ui.label(RichText::new("Name").color(palette.subtext));
                let response = ui.add(
                    TextEdit::singleline(&mut form.name.value)
                        .id_salt((id, "name"))
                        .hint_text("Production database")
                        .desired_width(360.0),
                );
                if new_modal {
                    response.request_focus();
                }
                ui.end_row();

                if form.driver == Driver::Sqlite {
                    ui.label(RichText::new("Database file").color(palette.subtext));
                    ui.horizontal(|ui| {
                        ui.add(
                            TextEdit::singleline(&mut form.database.value)
                                .id_salt((id, "database"))
                                .hint_text("rustquery.db or :memory:")
                                .desired_width(268.0),
                        );
                        if ui.button("Browse…").clicked() {
                            if let Some(path) = super::pick_sqlite_file(&form.database.value) {
                                form.database.set(path.display().to_string());
                            }
                        }
                    });
                    ui.end_row();
                } else {
                    ui.label(RichText::new("Host").color(palette.subtext));
                    ui.add(
                        TextEdit::singleline(&mut form.host.value)
                            .id_salt((id, "host"))
                            .desired_width(360.0),
                    );
                    ui.end_row();

                    ui.label(RichText::new("Port").color(palette.subtext));
                    ui.add(
                        TextEdit::singleline(&mut form.port.value)
                            .id_salt((id, "port"))
                            .desired_width(90.0),
                    );
                    ui.end_row();

                    ui.label(RichText::new("Database").color(palette.subtext));
                    ui.add(
                        TextEdit::singleline(&mut form.database.value)
                            .id_salt((id, "database"))
                            .desired_width(360.0),
                    );
                    ui.end_row();

                    ui.label(RichText::new("User").color(palette.subtext));
                    ui.add(
                        TextEdit::singleline(&mut form.user.value)
                            .id_salt((id, "user"))
                            .desired_width(360.0),
                    );
                    ui.end_row();

                    ui.label(RichText::new("Password").color(palette.subtext));
                    ui.horizontal(|ui| {
                        ui.add(
                            TextEdit::singleline(&mut form.password.value)
                                .id_salt((id, "password"))
                                .password(!form.show_password)
                                .desired_width(300.0),
                        );
                        ui.checkbox(&mut form.show_password, "Show");
                    });
                    ui.end_row();

                    ui.label(RichText::new("TLS mode").color(palette.subtext));
                    let mut ssl = form.ssl_mode;
                    ComboBox::from_id_salt((id, "ssl"))
                        .selected_text(ssl.label())
                        .width(180.0)
                        .show_ui(ui, |ui| {
                            for candidate in SslMode::ALL {
                                ui.selectable_value(&mut ssl, candidate, candidate.label());
                            }
                        });
                    form.ssl_mode = ssl;
                    ui.end_row();
                }
            });

        ui.add_space(5.0);
        if form.driver == Driver::Sqlite {
            ui.label(
                RichText::new(
                    "SQLite opens or creates the file. Use :memory: for a temporary in-memory database.",
                )
                .small()
                .color(palette.muted),
            );
        }

        if form.testing {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(
                    RichText::new("Testing connection…")
                        .small()
                        .color(palette.yellow),
                );
            });
        } else if let Some((ok, message)) = &form.test_result {
            let colour = if *ok { palette.green } else { palette.red };
            ui.label(
                RichText::new(format!("{} {message}", if *ok { "OK" } else { "Failed" }))
                    .small()
                    .color(colour),
            );
        }
        if let Some(error) = &form.error {
            ui.label(
                RichText::new(format!("Failed: {error}"))
                    .small()
                    .color(palette.red),
            );
        }

        footer_buttons(ui, palette, |ui| {
            if primary(ui, palette, "Save").clicked() {
                action = Some(ProfileAction::Save);
            }
            if secondary(ui, "Cancel").clicked() {
                action = Some(ProfileAction::Cancel);
            }
            if ui
                .add_enabled(!form.testing, Button::new("Test connection"))
                .clicked()
            {
                action = Some(ProfileAction::Test);
            }
        });
    });

    match action {
        Some(ProfileAction::Save) => {
            app.modal = Some(Modal::Profile(form));
            app.save_profile_form();
            return;
        }
        Some(ProfileAction::Test) => {
            app.modal = Some(Modal::Profile(form));
            app.test_profile_form();
            return;
        }
        Some(ProfileAction::Cancel) => return,
        None => {}
    }
    if close {
        return;
    }
    app.modal = Some(Modal::Profile(form));
}

#[derive(Debug, Clone, Copy)]
enum ProfileAction {
    Save,
    Test,
    Cancel,
}

// ---------------------------------------------------------------------------
// Confirmation
// ---------------------------------------------------------------------------

fn confirm(
    app: &mut App,
    ctx: &egui::Context,
    palette: &Palette,
    mut dialog: crate::app::ConfirmDialog,
) {
    let mut yes: Option<bool> = None;
    let pressed_y = ctx.input(|input| input.key_pressed(Key::Y));
    let pressed_n = ctx.input(|input| input.key_pressed(Key::N));
    if pressed_y {
        yes = Some(true);
    } else if pressed_n {
        yes = Some(false);
    }

    let destructive_action = dialog.action.is_destructive();
    let close = shell(ctx, "rustquery_confirm", palette, 480.0, |ui| {
        heading(ui, palette, &dialog.title, "");
        ui.label(RichText::new(&dialog.message).color(palette.text));
        ui.add_space(6.0);
        footer_buttons(ui, palette, |ui| {
            let confirmed = if destructive_action {
                destructive(ui, palette, "Yes").clicked()
            } else {
                primary(ui, palette, "Yes").clicked()
            };
            if confirmed {
                yes = Some(true);
            }
            if secondary(ui, "No").clicked() {
                yes = Some(false);
            }
        });
    });

    match yes {
        Some(true) => {
            app.apply_confirm(dialog.action);
            return;
        }
        Some(false) => return,
        None => {}
    }
    if close {
        return;
    }
    dialog.yes = false;
    app.modal = Some(Modal::Confirm(dialog));
}

// ---------------------------------------------------------------------------
// Export
// ---------------------------------------------------------------------------

fn export(
    app: &mut App,
    ctx: &egui::Context,
    palette: &Palette,
    mut dialog: ExportDialog,
    new_modal: bool,
) {
    let mut action: Option<ExportAction> = None;
    let id = Id::new(("rustquery_export", dialog.tab_id));

    let close = shell(ctx, "rustquery_export_modal", palette, 600.0, |ui| {
        heading(
            ui,
            palette,
            "Export result set",
            "CSV, JSON and SQL INSERT exports are generated off the UI thread.",
        );
        Grid::new((id, "grid"))
            .num_columns(2)
            .spacing([14.0, 8.0])
            .show(ui, |ui| {
                ui.label(RichText::new("Format").color(palette.subtext));
                let mut format = dialog.format;
                ComboBox::from_id_salt((id, "format"))
                    .selected_text(format.label())
                    .width(190.0)
                    .show_ui(ui, |ui| {
                        for candidate in ExportFormat::ALL {
                            ui.selectable_value(&mut format, candidate, candidate.label());
                        }
                    });
                if format != dialog.format {
                    dialog.set_format(format);
                }
                ui.end_row();

                ui.label(RichText::new("Rows").color(palette.subtext));
                let mut scope = dialog.scope;
                ComboBox::from_id_salt((id, "scope"))
                    .selected_text(scope.label())
                    .width(190.0)
                    .show_ui(ui, |ui| {
                        for candidate in [ExportScope::Visible, ExportScope::All] {
                            ui.selectable_value(&mut scope, candidate, candidate.label());
                        }
                    });
                dialog.scope = scope;
                ui.end_row();

                ui.label(RichText::new("Headers").color(palette.subtext));
                ui.checkbox(&mut dialog.include_headers, "Include column headers");
                ui.end_row();

                ui.label(RichText::new("Path").color(palette.subtext));
                ui.horizontal(|ui| {
                    let response = ui.add(
                        TextEdit::singleline(&mut dialog.path.value)
                            .id_salt((id, "path"))
                            .hint_text("~/export.csv")
                            .desired_width(360.0),
                    );
                    if new_modal {
                        response.request_focus();
                    }
                    if ui.button("Browse…").clicked() {
                        if let Some(path) =
                            super::pick_export_path(&dialog.path.value, dialog.format.extension())
                        {
                            dialog.path.set(path.display().to_string());
                        }
                    }
                });
                ui.end_row();

                ui.label(RichText::new("Table").color(palette.subtext));
                ui.add_enabled(
                    dialog.format == ExportFormat::Sql,
                    TextEdit::singleline(&mut dialog.table.value)
                        .id_salt((id, "table"))
                        .hint_text("table name for SQL INSERT")
                        .desired_width(360.0),
                );
                ui.end_row();
            });

        ui.add_space(4.0);
        ui.label(
            RichText::new("The correct extension is appended automatically. ~ is expanded.")
                .small()
                .color(palette.muted),
        );
        if let Some(error) = &dialog.error {
            ui.label(
                RichText::new(format!("Failed: {error}"))
                    .small()
                    .color(palette.red),
            );
        }

        footer_buttons(ui, palette, |ui| {
            if primary(ui, palette, "Export").clicked() {
                action = Some(ExportAction::Export);
            }
            if secondary(ui, "Cancel").clicked() {
                action = Some(ExportAction::Cancel);
            }
            if ui.button("Reset path").clicked() {
                let extension = dialog.format.extension();
                let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
                dialog.path.set(format!("~/rustquery-{stamp}.{extension}"));
            }
        });
    });

    match action {
        Some(ExportAction::Export) => {
            app.modal = Some(Modal::Export(dialog));
            app.start_export();
            return;
        }
        Some(ExportAction::Cancel) => return,
        None => {}
    }
    if close {
        return;
    }
    app.modal = Some(Modal::Export(dialog));
}

#[derive(Debug, Clone, Copy)]
enum ExportAction {
    Export,
    Cancel,
}

// ---------------------------------------------------------------------------
// Filter
// ---------------------------------------------------------------------------

fn filter(
    app: &mut App,
    ctx: &egui::Context,
    palette: &Palette,
    mut dialog: FilterDialog,
    new_modal: bool,
) {
    let mut action: Option<FilterAction> = None;
    let id = Id::new(("rustquery_filter", dialog.tab_id));

    let close = shell(ctx, "rustquery_filter_modal", palette, 520.0, |ui| {
        heading(
            ui,
            palette,
            "Filter results",
            "Case-insensitive match across every column. Applied locally, never sent to the database.",
        );
        let response = ui.add(
            TextEdit::singleline(&mut dialog.input.value)
                .id_salt((id, "input"))
                .hint_text("text to find")
                .desired_width(f32::INFINITY),
        );
        if new_modal {
            response.request_focus();
        }
        if response.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter)) {
            action = Some(FilterAction::Apply);
        }
        if let Some(error) = &dialog.error {
            ui.label(RichText::new(error).small().color(palette.red));
        }
        footer_buttons(ui, palette, |ui| {
            if primary(ui, palette, "Apply").clicked() {
                action = Some(FilterAction::Apply);
            }
            if secondary(ui, "Cancel").clicked() {
                action = Some(FilterAction::Cancel);
            }
            if ui.button("Clear").clicked() {
                dialog.input.set(String::new());
            }
        });
    });

    match action {
        Some(FilterAction::Apply) => {
            app.modal = Some(Modal::Filter(dialog));
            app.apply_filter_dialog();
            app.modal = None;
            return;
        }
        Some(FilterAction::Cancel) => {
            app.modal = Some(Modal::Filter(dialog));
            app.cancel_filter_dialog();
            return;
        }
        None => {}
    }
    if close {
        app.modal = Some(Modal::Filter(dialog));
        app.cancel_filter_dialog();
        return;
    }
    app.modal = Some(Modal::Filter(dialog));
}

#[derive(Debug, Clone, Copy)]
enum FilterAction {
    Apply,
    Cancel,
}

// ---------------------------------------------------------------------------
// Cell details
// ---------------------------------------------------------------------------

fn cell(
    app: &mut App,
    ctx: &egui::Context,
    palette: &Palette,
    dialog: crate::app::CellDetailDialog,
) {
    let value = dialog.value.clone().unwrap_or_else(|| "NULL".to_string());
    let mut copy = false;
    let mut close_now = false;
    let close = shell(ctx, "rustquery_cell", palette, 640.0, |ui| {
        heading(
            ui,
            palette,
            &format!("Cell · {}", dialog.column),
            &format!(
                "Row {} · type {} · {} character{} · {}",
                format_count(dialog.row_number as u64),
                dialog.type_name,
                value.chars().count(),
                if value.chars().count() == 1 { "" } else { "s" },
                if dialog.value.is_some() {
                    "value"
                } else {
                    "SQL NULL"
                }
            ),
        );
        Frame::new()
            .fill(palette.bg)
            .stroke(Stroke::new(1.0_f32, palette.border))
            .corner_radius(6)
            .inner_margin(Margin::same(8))
            .show(ui, |ui| {
                ScrollArea::both()
                    .id_salt("rustquery_cell_scroll")
                    .max_height(320.0)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.add(
                            egui::Label::new(RichText::new(&value).monospace().color(
                                if dialog.value.is_some() {
                                    palette.text
                                } else {
                                    palette.muted
                                },
                            ))
                            .selectable(true)
                            .wrap(),
                        );
                    });
            });
        footer_buttons(ui, palette, |ui| {
            if primary(ui, palette, "Close").clicked() {
                close_now = true;
            }
            if ui.button("Copy value").clicked() {
                copy = true;
            }
        });
    });
    if copy {
        let message = app.clipboard.copy(&value);
        app.toast(LogLevel::Info, message);
    }
    if close || close_now {
        return;
    }
    app.modal = Some(Modal::Cell(dialog));
}

// ---------------------------------------------------------------------------
// History
// ---------------------------------------------------------------------------

fn history(
    app: &mut App,
    ctx: &egui::Context,
    palette: &Palette,
    mut browser: crate::app::HistoryBrowser,
    new_modal: bool,
) {
    let max = app.history.len().saturating_sub(1);
    let up = !new_modal && ctx.input(|input| input.key_pressed(Key::ArrowUp));
    let down = !new_modal && ctx.input(|input| input.key_pressed(Key::ArrowDown));
    if up {
        browser.selected = browser.selected.saturating_sub(1);
    }
    if down {
        browser.selected = (browser.selected + 1).min(max);
    }
    let enter = !new_modal && ctx.input(|input| input.key_pressed(Key::Enter));
    if enter && !app.history.is_empty() {
        app.load_history_entry(browser.selected.min(max));
        return;
    }

    let mut load: Option<usize> = None;
    let mut copy: Option<String> = None;
    let mut close_now = false;
    let close = shell(ctx, "rustquery_history", palette, 760.0, |ui| {
        heading(
            ui,
            palette,
            "Query history",
            "Press Enter or double-click to open an entry in a new tab.",
        );
        if app.history.is_empty() {
            ui.label(
                RichText::new("No queries have been executed yet.")
                    .small()
                    .color(palette.muted),
            );
        } else {
            ScrollArea::vertical()
                .id_salt("rustquery_history_scroll")
                .max_height(300.0)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for (index, entry) in app.history.iter().enumerate() {
                        let selected = index == browser.selected;
                        let status_colour = if entry.success {
                            palette.green
                        } else {
                            palette.red
                        };
                        let label = format!(
                            "{}  {}  {:<20}  {:>7} ms  {:>8} rows   {}",
                            entry
                                .at
                                .with_timezone(&chrono::Local)
                                .format("%m-%d %H:%M:%S"),
                            if entry.success { "OK" } else { "ERR" },
                            truncate(&entry.connection, 20),
                            format_count(entry.duration_ms),
                            format_count(entry.rows),
                            entry.snippet(64)
                        );
                        let response = ui.add(
                            egui::Label::new(RichText::new(label).monospace().small().color(
                                if selected {
                                    palette.text
                                } else if entry.success {
                                    palette.subtext
                                } else {
                                    status_colour
                                },
                            ))
                            .sense(egui::Sense::click())
                            .truncate(),
                        );
                        if response.clicked() {
                            browser.selected = index;
                        }
                        if response.double_clicked() {
                            load = Some(index);
                        }
                    }
                });
            ui.add_space(6.0);
            if let Some(entry) = app.history.get(browser.selected) {
                ui.label(
                    RichText::new(format!(
                        "{} · {} rows · {} ms",
                        entry.connection,
                        format_count(entry.rows),
                        format_count(entry.duration_ms)
                    ))
                    .small()
                    .color(palette.muted),
                );
                Frame::new()
                    .fill(palette.bg)
                    .stroke(Stroke::new(1.0_f32, palette.border))
                    .corner_radius(6)
                    .inner_margin(Margin::same(8))
                    .show(ui, |ui| {
                        ScrollArea::vertical()
                            .id_salt("rustquery_history_sql")
                            .max_height(150.0)
                            .show(ui, |ui| {
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(&entry.sql)
                                            .monospace()
                                            .small()
                                            .color(palette.subtext),
                                    )
                                    .selectable(true)
                                    .wrap(),
                                );
                            });
                    });
            }
        }
        footer_buttons(ui, palette, |ui| {
            if primary(ui, palette, "Open in new tab").clicked() {
                load = Some(browser.selected);
            }
            if secondary(ui, "Close").clicked() {
                close_now = true;
            }
            if ui.button("Copy SQL").clicked() {
                copy = app
                    .history
                    .get(browser.selected)
                    .map(|entry| entry.sql.clone());
            }
        });
    });

    if let Some(sql) = copy {
        let message = app.clipboard.copy(&sql);
        app.toast(LogLevel::Info, format!("SQL · {message}"));
    }
    if let Some(index) = load {
        app.load_history_entry(index);
        return;
    }
    if close || close_now {
        return;
    }
    app.modal = Some(Modal::History(browser));
}

fn truncate(value: &str, width: usize) -> String {
    if value.chars().count() <= width {
        value.to_string()
    } else {
        let prefix: String = value.chars().take(width.saturating_sub(1)).collect();
        format!("{prefix}…")
    }
}

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

fn settings(
    app: &mut App,
    ctx: &egui::Context,
    palette: &Palette,
    mut dialog: SettingsDialog,
    new_modal: bool,
) {
    let field_count = SettingsField::ALL.len();
    let up = !new_modal && ctx.input(|input| input.key_pressed(Key::ArrowUp));
    let down = !new_modal && ctx.input(|input| input.key_pressed(Key::ArrowDown));
    if up {
        dialog.selected = dialog.selected.saturating_sub(1);
    }
    if down {
        dialog.selected = (dialog.selected + 1).min(field_count.saturating_sub(1));
    }
    let mut adjust: Option<isize> = None;
    if !new_modal && ctx.input(|input| input.key_pressed(Key::ArrowLeft)) {
        adjust = Some(-1);
    }
    if !new_modal && ctx.input(|input| input.key_pressed(Key::ArrowRight)) {
        adjust = Some(1);
    }
    if let Some(direction) = adjust {
        if let Some(field) = SettingsField::ALL.get(dialog.selected) {
            adjust_setting(&mut app.settings, *field, direction);
        }
    }

    let mut action: Option<SettingsAction> = None;
    let enter = !new_modal && ctx.input(|input| input.key_pressed(Key::Enter));
    if enter {
        action = Some(SettingsAction::Save);
    }

    let close = shell(ctx, "rustquery_settings", palette, 560.0, |ui| {
        heading(
            ui,
            palette,
            "Settings",
            "Changes apply immediately and are saved when you press Save.",
        );
        let mut selected = dialog.selected;
        let mut rows: Vec<(usize, egui::Rect, Vec<egui::Rect>)> = Vec::new();
        for (index, field) in SettingsField::ALL.iter().enumerate() {
            let is_selected = index == selected;
            let mut controls = Vec::new();
            let frame = Frame::new()
                .fill(if is_selected {
                    palette.surface
                } else {
                    egui::Color32::TRANSPARENT
                })
                .corner_radius(5)
                .inner_margin(Margin::symmetric(8, 5))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let response = ui.add(
                            egui::Label::new(
                                RichText::new(field.label())
                                    .color(if is_selected {
                                        palette.text
                                    } else {
                                        palette.subtext
                                    })
                                    .strong(),
                            )
                            .sense(egui::Sense::click()),
                        );
                        if response.clicked() {
                            selected = index;
                        }
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let plus = ui.small_button("+");
                            plus.widget_info(|| {
                                egui::WidgetInfo::labeled(
                                    egui::WidgetType::Button,
                                    true,
                                    "Increase setting",
                                )
                            });
                            if plus.clicked() {
                                selected = index;
                                adjust_setting(&mut app.settings, *field, 1);
                            }
                            let minus = ui.small_button("−");
                            minus.widget_info(|| {
                                egui::WidgetInfo::labeled(
                                    egui::WidgetType::Button,
                                    true,
                                    "Decrease setting",
                                )
                            });
                            if minus.clicked() {
                                selected = index;
                                adjust_setting(&mut app.settings, *field, -1);
                            }
                            controls.push(plus.rect);
                            controls.push(minus.rect);
                            ui.label(
                                RichText::new(setting_value(&app.settings, *field))
                                    .monospace()
                                    .color(palette.text),
                            );
                        });
                    });
                });
            rows.push((index, frame.response.rect, controls));
            ui.add_space(2.0);
        }
        // Clicking anywhere on a row selects it, not just the label. Buttons
        // are excluded so they keep their own click behaviour.
        let clicked = ui.input(|input| {
            if input.pointer.primary_clicked() {
                input.pointer.interact_pos()
            } else {
                None
            }
        });
        if let Some(pos) = clicked {
            if let Some((index, _, _)) = rows.iter().find(|(_, rect, controls)| {
                rect.contains(pos) && !controls.iter().any(|control| control.contains(pos))
            }) {
                selected = *index;
            }
        }
        dialog.selected = selected;
        ui.add_space(6.0);
        ui.label(
            RichText::new(
                "Use ← and → on the selected row (or the +/− buttons). Colour theme updates live; Esc or Cancel reverts unsaved changes.",
            )
            .small()
            .color(palette.muted),
        );
        footer_buttons(ui, palette, |ui| {
            if primary(ui, palette, "Save").clicked() {
                action = Some(SettingsAction::Save);
            }
            if secondary(ui, "Cancel").clicked() {
                action = Some(SettingsAction::Cancel);
            }
            if ui.button("Restore defaults").clicked() {
                action = Some(SettingsAction::Defaults);
            }
        });
    });

    match action {
        Some(SettingsAction::Save) => {
            app.settings.sanitize();
            app.save_settings_now();
            app.toast(LogLevel::Success, "Settings saved");
            return;
        }
        Some(SettingsAction::Cancel) => {
            app.settings = dialog.backup;
            app.settings.sanitize();
            app.toast(LogLevel::Info, "Settings changes reverted");
            return;
        }
        Some(SettingsAction::Defaults) => {
            app.settings = Settings::default();
            app.settings.sanitize();
            app.modal = Some(Modal::Settings(dialog));
            return;
        }
        None => {}
    }
    if close {
        app.settings = dialog.backup;
        app.settings.sanitize();
        return;
    }
    app.modal = Some(Modal::Settings(dialog));
}

#[derive(Debug, Clone, Copy)]
enum SettingsAction {
    Save,
    Cancel,
    Defaults,
}

fn setting_value(settings: &Settings, field: SettingsField) -> String {
    match field {
        SettingsField::RowLimit => format_count(settings.row_limit as u64),
        SettingsField::Timeout => format!("{} s", settings.query_timeout_secs),
        SettingsField::EditorRatio => format!("{} %", settings.editor_ratio),
        SettingsField::SidebarWidth => format!("{} cols", settings.sidebar_width),
        SettingsField::UiScale => format!("{} %", settings.ui_scale_percent),
        SettingsField::Theme => settings.theme.label().to_string(),
        SettingsField::ShowLogs => yes_no(settings.show_logs),
    }
}

fn yes_no(value: bool) -> String {
    if value { "On" } else { "Off" }.to_string()
}

fn adjust_setting(settings: &mut Settings, field: SettingsField, direction: isize) {
    match field {
        SettingsField::RowLimit => {
            let next = (settings.row_limit as i64 + 100 * direction as i64).clamp(10, 100_000);
            settings.row_limit = next as usize;
        }
        SettingsField::Timeout => {
            let next = (settings.query_timeout_secs as i64 + 5 * direction as i64).clamp(1, 3_600);
            settings.query_timeout_secs = next as u64;
        }
        SettingsField::EditorRatio => {
            let next = (settings.editor_ratio as i64 + 2 * direction as i64).clamp(20, 80);
            settings.editor_ratio = next as u16;
        }
        SettingsField::SidebarWidth => {
            let next = (settings.sidebar_width as i64 + 2 * direction as i64).clamp(20, 80);
            settings.sidebar_width = next as u16;
        }
        SettingsField::UiScale => {
            let next = (settings.ui_scale_percent as i64 + 5 * direction as i64).clamp(90, 130);
            settings.ui_scale_percent = next as u16;
        }
        SettingsField::Theme => {
            settings.theme = if direction >= 0 {
                settings.theme.next()
            } else {
                let index = crate::config::ThemeKind::ALL
                    .iter()
                    .position(|theme| *theme == settings.theme)
                    .unwrap_or(0);
                crate::config::ThemeKind::ALL[(index + crate::config::ThemeKind::ALL.len() - 1)
                    % crate::config::ThemeKind::ALL.len()]
            };
        }
        SettingsField::ShowLogs => {
            if direction != 0 {
                settings.show_logs = !settings.show_logs;
            }
        }
    }
    settings.sanitize();
}

// ---------------------------------------------------------------------------
// Single-line input
// ---------------------------------------------------------------------------

fn input(
    app: &mut App,
    ctx: &egui::Context,
    palette: &Palette,
    mut dialog: crate::app::InputDialog,
    new_modal: bool,
) {
    let mut submit = false;
    let mut cancel = false;
    let close = shell(ctx, "rustquery_input", palette, 560.0, |ui| {
        heading(ui, palette, &dialog.title, &dialog.label);
        let response = ui.add(
            TextEdit::singleline(&mut dialog.input.value)
                .id_salt("rustquery_input_field")
                .hint_text(&dialog.hint)
                .desired_width(f32::INFINITY),
        );
        if new_modal {
            response.request_focus();
        }
        if response.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter)) {
            submit = true;
        }
        ui.label(RichText::new(&dialog.hint).small().color(palette.muted));
        if let Some(error) = &dialog.error {
            ui.label(
                RichText::new(format!("Failed: {error}"))
                    .small()
                    .color(palette.red),
            );
        }
        footer_buttons(ui, palette, |ui| {
            if primary(ui, palette, "OK").clicked() {
                submit = true;
            }
            if secondary(ui, "Cancel").clicked() {
                cancel = true;
            }
        });
    });

    if cancel || (close && !submit) {
        return;
    }
    if submit {
        match dialog.purpose {
            InputPurpose::SidebarFilter => {
                app.catalog.filter = dialog.input.value;
                app.toast(LogLevel::Info, "Explorer filter applied");
                return;
            }
            InputPurpose::SaveQuery { .. } => {
                app.modal = Some(Modal::Input(dialog));
                app.commit_save_query();
                return;
            }
            InputPurpose::CreateFolder => match app.create_folder(&dialog.input.value) {
                Ok(()) => return,
                Err(err) => dialog.error = Some(err),
            },
            InputPurpose::RenameFolder { id } => match app.rename_folder(id, &dialog.input.value) {
                Ok(()) => return,
                Err(err) => dialog.error = Some(err),
            },
        }
    }
    app.modal = Some(Modal::Input(dialog));
}

// ---------------------------------------------------------------------------
// Info
// ---------------------------------------------------------------------------

fn info(app: &mut App, ctx: &egui::Context, palette: &Palette, dialog: crate::app::InfoDialog) {
    let mut copy = false;
    let mut close_now = false;
    let close = shell(ctx, "rustquery_info", palette, 600.0, |ui| {
        heading(ui, palette, &dialog.title, "");
        ScrollArea::vertical()
            .id_salt("rustquery_info_scroll")
            .max_height(360.0)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for line in &dialog.lines {
                    if line.is_empty() {
                        ui.add_space(4.0);
                    } else {
                        ui.label(RichText::new(line).color(palette.text));
                    }
                }
            });
        footer_buttons(ui, palette, |ui| {
            if primary(ui, palette, "Close").clicked() {
                close_now = true;
            }
            if ui.button("Copy details").clicked() {
                copy = true;
            }
        });
    });
    if copy {
        let message = app.clipboard.copy(&dialog.lines.join("\n"));
        app.toast(LogLevel::Info, message);
    }
    if close || close_now {
        return;
    }
    app.modal = Some(Modal::Info(dialog));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn help_text_contains_core_shortcuts() {
        let text = help_text();
        assert!(text.contains("Ctrl+Enter"));
        assert!(text.contains("Ctrl+F"));
        assert!(text.contains("BEGIN"));
    }

    #[test]
    fn settings_adjustment_is_clamped() {
        let mut settings = Settings::default();
        for _ in 0..200 {
            adjust_setting(&mut settings, SettingsField::EditorRatio, 1);
        }
        assert_eq!(settings.editor_ratio, 80);
        for _ in 0..200 {
            adjust_setting(&mut settings, SettingsField::EditorRatio, -1);
        }
        assert_eq!(settings.editor_ratio, 20);

        for _ in 0..200 {
            adjust_setting(&mut settings, SettingsField::SidebarWidth, 1);
        }
        assert_eq!(settings.sidebar_width, 80);
    }
}
