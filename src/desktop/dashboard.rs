//! Connection dashboard: profile cards, test results and the active session.
//!
//! The dashboard is intentionally button-driven: it presents the saved
//! profiles as cards with enough room for endpoint details and
//! connection-test outcomes.

use std::time::Instant;

use eframe::egui::{
    self, Align, Button, Frame, Layout, Margin, RichText, ScrollArea, Sense, Stroke, Ui,
};
use uuid::Uuid;

use crate::app::{App, Focus, TestStatus};
use crate::model::{ConnectionProfile, Driver, LogLevel};

use super::theme::Palette;

pub fn show(app: &mut App, ui: &mut Ui) {
    let palette = Palette::for_theme(app.theme_kind());
    let mut action: Option<DashboardAction> = None;

    ScrollArea::vertical()
        .id_salt("rustquery_dashboard_scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.add_space(18.0);
            Frame::new()
                .inner_margin(Margin::symmetric(24, 0))
                .show(ui, |ui| {
                    header(app, ui, &palette, &mut action);
                    ui.add_space(14.0);
                    active_session(app, ui, &palette, &mut action);
                    connect_progress(app, ui, &palette, &mut action);
                    if let Some(error) = app.dashboard.connect_error.clone() {
                        ui.add_space(8.0);
                        error_banner(ui, &palette, &error, &mut action);
                    }
                    ui.add_space(16.0);
                    profiles(app, ui, &palette, &mut action);
                    ui.add_space(24.0);
                    footer(ui, &palette);
                    ui.add_space(18.0);
                });
        });

    if let Some(action) = action {
        apply(app, action);
    }
}

fn header(app: &mut App, ui: &mut Ui, palette: &Palette, action: &mut Option<DashboardAction>) {
    ui.vertical(|ui| {
        ui.label(
            RichText::new("RustQuery")
                .size(32.0)
                .strong()
                .color(palette.text),
        );
        ui.label(
            RichText::new("A fast desktop SQL workbench for PostgreSQL, MySQL and SQLite")
                .color(palette.subtext),
        );
    });
    ui.add_space(10.0);
    ui.horizontal_wrapped(|ui| {
        if ui
            .add(
                Button::new(
                    RichText::new("+  New connection")
                        .color(palette.on_accent())
                        .strong(),
                )
                .fill(palette.accent),
            )
            .on_hover_text("Create a PostgreSQL, MySQL or SQLite profile (Ctrl+Shift+N)")
            .clicked()
        {
            *action = Some(DashboardAction::NewProfile);
        }
        if ui
            .add(Button::new("New SQLite database"))
            .on_hover_text("Create a local SQLite profile")
            .clicked()
        {
            *action = Some(DashboardAction::NewSqliteProfile);
        }
        if app.session.is_some()
            && ui
                .button("Open workspace")
                .on_hover_text("Return to the editor (F2)")
                .clicked()
        {
            *action = Some(DashboardAction::OpenWorkspace);
        }
        if app.session.is_some() && ui.button("Disconnect").clicked() {
            *action = Some(DashboardAction::Disconnect);
        }
    });
}

fn active_session(app: &App, ui: &mut Ui, palette: &Palette, action: &mut Option<DashboardAction>) {
    let Some(session) = &app.session else {
        return;
    };
    ui.add_space(4.0);
    let frame = Frame::new()
        .fill(palette.panel)
        .stroke(Stroke::new(1.0_f32, palette.green.gamma_multiply(0.65)))
        .corner_radius(9)
        .inner_margin(Margin::symmetric(14, 11));
    frame.show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new("●").color(palette.green).size(16.0));
            ui.vertical(|ui| {
                ui.label(
                    RichText::new(format!("Connected to {}", session.info.profile_name))
                        .strong()
                        .color(palette.text),
                );
                ui.label(
                    RichText::new(format!(
                        "{} · {} · {}@{} · {} ms",
                        session.info.driver.label(),
                        session.info.current_database,
                        session.info.current_user,
                        session.info.server_version,
                        session.info.latency_ms
                    ))
                    .small()
                    .color(palette.subtext),
                );
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button("Disconnect").clicked() {
                    *action = Some(DashboardAction::Disconnect);
                }
                if ui.button("Open workspace").clicked() {
                    *action = Some(DashboardAction::OpenWorkspace);
                }
                if session.tx_active {
                    ui.label(
                        RichText::new("TRANSACTION OPEN")
                            .small()
                            .strong()
                            .color(palette.yellow),
                    );
                }
            });
        });
    });
}

fn connect_progress(
    app: &App,
    ui: &mut Ui,
    palette: &Palette,
    action: &mut Option<DashboardAction>,
) {
    let Some(connect) = &app.connect else {
        return;
    };
    ui.add_space(8.0);
    Frame::new()
        .fill(palette.panel)
        .stroke(Stroke::new(1.0_f32, palette.accent.gamma_multiply(0.65)))
        .corner_radius(9)
        .inner_margin(Margin::symmetric(14, 10))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.vertical(|ui| {
                    ui.label(
                        RichText::new(format!("Connecting to {}", connect.profile.name))
                            .strong()
                            .color(palette.text),
                    );
                    ui.label(
                        RichText::new(connect.profile.describe())
                            .small()
                            .color(palette.subtext),
                    );
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.button("Cancel").clicked() {
                        *action = Some(DashboardAction::CancelConnect);
                    }
                    ui.label(
                        RichText::new(format!("{:.1}s", connect.started.elapsed().as_secs_f32()))
                            .small()
                            .color(palette.muted),
                    );
                });
            });
        });
}

fn error_banner(ui: &mut Ui, palette: &Palette, error: &str, action: &mut Option<DashboardAction>) {
    Frame::new()
        .fill(palette.red.gamma_multiply(0.12))
        .stroke(Stroke::new(1.0_f32, palette.red.gamma_multiply(0.7)))
        .corner_radius(8)
        .inner_margin(Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("Connection failed")
                        .strong()
                        .color(palette.red),
                );
                ui.label(RichText::new(error).small().color(palette.text));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.small_button("Dismiss").clicked() {
                        *action = Some(DashboardAction::DismissError);
                    }
                });
            });
        });
}

fn profiles(app: &mut App, ui: &mut Ui, palette: &Palette, action: &mut Option<DashboardAction>) {
    ui.horizontal(|ui| {
        ui.label(
            RichText::new("Connections")
                .size(17.0)
                .strong()
                .color(palette.text),
        );
        ui.label(
            RichText::new(format!(
                "{} saved profile{}",
                app.profiles.len(),
                if app.profiles.len() == 1 { "" } else { "s" }
            ))
            .small()
            .color(palette.muted),
        );
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.small_button("New").clicked() {
                *action = Some(DashboardAction::NewProfile);
            }
        });
    });
    ui.add_space(6.0);

    if app.profiles.is_empty() {
        Frame::new()
            .fill(palette.panel)
            .stroke(Stroke::new(1.0_f32, palette.border))
            .corner_radius(9)
            .inner_margin(Margin::symmetric(18, 20))
            .show(ui, |ui| {
                ui.vertical_centered(|ui| {
                    ui.label(
                        RichText::new("No saved connections yet")
                            .size(16.0)
                            .strong()
                            .color(palette.text),
                    );
                    ui.label(
                        RichText::new(
                            "Create a profile to connect to PostgreSQL or MySQL, or open a local SQLite file.",
                        )
                        .small()
                        .color(palette.muted),
                    );
                    ui.add_space(10.0);
                    if ui
                        .add(
                            Button::new(
                                RichText::new("Create your first connection")
                                    .color(palette.on_accent()),
                            )
                            .fill(palette.accent),
                        )
                        .clicked()
                    {
                        *action = Some(DashboardAction::NewProfile);
                    }
                });
            });
        return;
    }

    let profiles = app.profiles.clone();
    let selected = app.dashboard.selected;
    let tests = app.dashboard.tests.clone();
    let active_session = app.session.as_ref().map(|session| session.info.profile_id);
    let busy = app.connect.is_some();

    for (index, profile) in profiles.iter().enumerate() {
        let is_selected = index == selected;
        let border = if is_selected {
            palette.accent
        } else {
            palette.border
        };
        let fill = if is_selected {
            palette.surface
        } else {
            palette.panel
        };
        Frame::new()
            .fill(fill)
            .stroke(Stroke::new(
                if is_selected { 1.5_f32 } else { 1.0_f32 },
                border,
            ))
            .corner_radius(9)
            .inner_margin(Margin::symmetric(14, 11))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let badge = Frame::new()
                        .fill(palette.driver_color(profile.driver).gamma_multiply(0.18))
                        .corner_radius(5)
                        .inner_margin(Margin::symmetric(6, 3))
                        .show(ui, |ui| {
                            ui.label(
                                RichText::new(profile.driver.label())
                                    .small()
                                    .strong()
                                    .color(palette.driver_color(profile.driver)),
                            );
                        });
                    let _ = badge;
                    let name = ui.add(
                        egui::Label::new(RichText::new(&profile.name).strong().color(palette.text))
                            .sense(Sense::click()),
                    );
                    if name.clicked() {
                        *action = Some(DashboardAction::Select(index));
                    }
                    if name.double_clicked() {
                        *action = Some(DashboardAction::Connect(index));
                    }
                    if active_session == Some(profile.id) {
                        ui.label(
                            RichText::new("active")
                                .small()
                                .color(palette.green)
                                .strong(),
                        );
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui
                            .add(Button::new("Delete").small().frame(false))
                            .on_hover_text("Delete this saved profile")
                            .clicked()
                        {
                            *action = Some(DashboardAction::Delete(index));
                        }
                        if ui
                            .add(Button::new("Edit").small().frame(false))
                            .on_hover_text("Edit this connection profile")
                            .clicked()
                        {
                            *action = Some(DashboardAction::Edit(index));
                        }
                        if ui
                            .add(Button::new("Test").small())
                            .on_hover_text("Open a temporary connection and report its version")
                            .clicked()
                        {
                            *action = Some(DashboardAction::Test(profile.id));
                        }
                        if ui
                            .add_enabled(
                                !busy,
                                Button::new(RichText::new("Connect").color(palette.on_accent())),
                            )
                            .on_hover_text("Connect to this database")
                            .clicked()
                        {
                            *action = Some(DashboardAction::Connect(index));
                        }
                    });
                });

                ui.label(
                    RichText::new(profile.describe())
                        .small()
                        .monospace()
                        .color(palette.subtext),
                );
                if profile.driver != Driver::Sqlite {
                    ui.label(
                        RichText::new(format!(
                            "host {}:{} · user {} · TLS {}",
                            profile.host,
                            profile.port,
                            profile.user,
                            profile.ssl_mode.label()
                        ))
                        .small()
                        .color(palette.muted),
                    );
                }

                if let Some(status) = tests.get(&profile.id) {
                    ui.add_space(2.0);
                    test_status(ui, palette, status);
                }
            });
        ui.add_space(7.0);
    }
}

fn test_status(ui: &mut Ui, palette: &Palette, status: &TestStatus) {
    match status {
        TestStatus::Testing(started) => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(
                    RichText::new(format!("Testing connection… {:.1}s", elapsed(started)))
                        .small()
                        .color(palette.yellow),
                );
            });
        }
        TestStatus::Ok(outcome) => {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("OK").small().strong().color(palette.green));
                ui.label(
                    RichText::new(format!(
                        "{} · {} ms · {}",
                        outcome.server_version, outcome.latency_ms, outcome.user
                    ))
                    .small()
                    .color(palette.green),
                );
            });
        }
        TestStatus::Err(error) => {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("Failed").small().strong().color(palette.red));
                ui.label(RichText::new(error).small().color(palette.red));
            });
        }
    }
}

fn elapsed(started: &Instant) -> f32 {
    started.elapsed().as_secs_f32()
}

fn footer(ui: &mut Ui, palette: &Palette) {
    ui.separator();
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        ui.label(
            RichText::new(format!("RustQuery {}", env!("CARGO_PKG_VERSION")))
                .small()
                .color(palette.muted),
        );
        ui.separator();
        ui.label(
            RichText::new(
                "Profiles, settings and query history are stored in your config directory.",
            )
            .small()
            .color(palette.muted),
        );
    });
    ui.label(
        RichText::new("F1 shortcuts · F2 dashboard/workspace · F5 run query · Ctrl+, settings")
            .small()
            .color(palette.muted),
    );
}

#[derive(Debug, Clone, Copy)]
enum DashboardAction {
    Select(usize),
    Connect(usize),
    Test(Uuid),
    Edit(usize),
    Delete(usize),
    NewProfile,
    NewSqliteProfile,
    CancelConnect,
    OpenWorkspace,
    Disconnect,
    DismissError,
}

fn apply(app: &mut App, action: DashboardAction) {
    match action {
        DashboardAction::Select(index) => {
            app.dashboard.selected = index;
        }
        DashboardAction::Connect(index) => {
            app.dashboard.selected = index;
            app.request_connect_or_confirm(index);
        }
        DashboardAction::Test(id) => app.test_profile(id),
        DashboardAction::Edit(index) => {
            if let Some(profile) = app.profiles.get(index).cloned() {
                app.dashboard.selected = index;
                app.open_profile_form(Some(&profile));
            }
        }
        DashboardAction::Delete(index) => {
            app.dashboard.selected = index;
            app.confirm_delete_selected_profile();
        }
        DashboardAction::NewProfile => app.open_profile_form(None),
        DashboardAction::NewSqliteProfile => {
            let profile = ConnectionProfile::new(Driver::Sqlite).with_defaults();
            app.open_profile_form(Some(&profile));
        }
        DashboardAction::CancelConnect => app.cancel_connect(),
        DashboardAction::OpenWorkspace => {
            app.focus_pane(Focus::Editor);
            app.toast(LogLevel::Info, "Workspace opened");
        }
        DashboardAction::Disconnect => app.request_disconnect(),
        DashboardAction::DismissError => {
            app.dashboard.connect_error = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sqlite_quick_profile_has_usable_defaults() {
        let profile = ConnectionProfile::new(Driver::Sqlite).with_defaults();
        assert_eq!(profile.driver, Driver::Sqlite);
        assert!(!profile.database.is_empty());
        assert!(!profile.name.is_empty());
        assert_eq!(profile.endpoint(), profile.database);
    }
}
