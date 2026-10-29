//! Native desktop shell for RustQuery.
//!
//! [`DesktopApp`] owns the UI-independent [`App`] state, a Tokio runtime and
//! the worker channel. Database tasks are spawned from the UI thread (the
//! runtime is entered for every frame) and their completions are drained at
//! the start of each `eframe` update.

pub mod dashboard;
pub mod editor;
pub mod modals;
pub mod results;
pub mod theme;
pub mod workspace;

use std::path::PathBuf;
use std::time::Duration;

use eframe::egui::{
    self, Align2, Color32, Frame, Key, KeyboardShortcut, Margin, Modifiers, RichText,
};
use tokio::runtime::Runtime;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

use crate::app::{App, Focus, Modal, TxAction, View, WorkerMessage};
use crate::config::{LoadReport, Store, ThemeKind};
use crate::model::LogLevel;

use self::theme::Palette;

/// The eframe application wrapper.
pub struct DesktopApp {
    pub app: App,
    runtime: Runtime,
    worker_rx: UnboundedReceiver<WorkerMessage>,
    applied_theme: Option<ThemeKind>,
    applied_ui_scale: Option<u16>,
    title: String,
}

impl DesktopApp {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        store: Store,
        report: LoadReport,
        runtime: Runtime,
        worker_tx: UnboundedSender<WorkerMessage>,
        worker_rx: UnboundedReceiver<WorkerMessage>,
    ) -> Self {
        let app = App::new(store, report, worker_tx);
        let theme = app.theme_kind();
        let scale = app.settings.ui_scale_percent;
        theme::apply(&cc.egui_ctx, theme, scale);
        Self {
            title: window_title(&app),
            app,
            runtime,
            worker_rx,
            applied_theme: Some(theme),
            applied_ui_scale: Some(scale),
        }
    }

    fn drain_workers(&mut self) {
        while let Ok(message) = self.worker_rx.try_recv() {
            self.app.handle_worker(message);
        }
    }

    fn handle_close_request(&mut self, ctx: &egui::Context) {
        if ctx.input(|input| input.viewport().close_requested()) && !self.app.should_quit {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.app.request_quit();
        }
        if self.app.should_quit {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        if self.app.modal.is_some() {
            return;
        }
        let workspace = self.app.view == View::Workspace;
        let connected = self.app.session.is_some();

        let mut action: Option<Shortcut> = None;
        ctx.input_mut(|input| {
            use Shortcut::*;
            let cmd = Modifiers::COMMAND;
            let cmd_shift = Modifiers::COMMAND | Modifiers::SHIFT;

            if input.consume_shortcut(&KeyboardShortcut::new(cmd, Key::Q)) {
                action = Some(Quit);
            } else if input.consume_shortcut(&KeyboardShortcut::new(cmd_shift, Key::O)) {
                action = Some(SwitchConnection);
            } else if input.consume_shortcut(&KeyboardShortcut::new(cmd_shift, Key::N)) {
                action = Some(NewConnection);
            } else if input.consume_shortcut(&KeyboardShortcut::new(cmd, Key::T)) {
                action = Some(NewTab);
            } else if input.consume_shortcut(&KeyboardShortcut::new(cmd, Key::W)) {
                action = Some(CloseTab);
            } else if input.consume_shortcut(&KeyboardShortcut::new(cmd, Key::O)) {
                action = Some(OpenFile);
            } else if input.consume_shortcut(&KeyboardShortcut::new(cmd, Key::S)) {
                action = Some(SaveFile);
            } else if input.consume_shortcut(&KeyboardShortcut::new(cmd, Key::E)) {
                action = Some(Export);
            } else if input.consume_shortcut(&KeyboardShortcut::new(cmd, Key::F)) {
                action = Some(Filter);
            } else if input.consume_shortcut(&KeyboardShortcut::new(cmd, Key::H)) {
                action = Some(History);
            } else if input.consume_shortcut(&KeyboardShortcut::new(cmd, Key::D)) {
                action = Some(Disconnect);
            } else if input.consume_shortcut(&KeyboardShortcut::new(cmd, Key::Comma)) {
                action = Some(Settings);
            } else if input.consume_shortcut(&KeyboardShortcut::new(cmd, Key::Space)) {
                action = Some(Complete);
            } else if input.consume_shortcut(&KeyboardShortcut::new(cmd, Key::Enter)) {
                action = Some(Execute);
            } else if input.consume_shortcut(&KeyboardShortcut::new(cmd, Key::Tab)) {
                action = Some(NextTab);
            } else if input.consume_shortcut(&KeyboardShortcut::new(cmd_shift, Key::Tab)) {
                action = Some(PreviousTab);
            } else if input.consume_shortcut(&KeyboardShortcut::new(cmd, Key::C)) {
                if self.app.running_count() > 0 || self.app.current_running() {
                    action = Some(CancelQuery);
                }
            } else if input.key_pressed(Key::F1) {
                action = Some(Help);
            } else if input.key_pressed(Key::F2) {
                action = Some(Dashboard);
            } else if input.key_pressed(Key::F3) {
                action = Some(FocusSidebar);
            } else if input.key_pressed(Key::F4) {
                action = Some(FocusEditor);
            } else if input.key_pressed(Key::F5) {
                action = Some(Execute);
            } else if input.key_pressed(Key::F6) {
                action = Some(FocusResults);
            } else if input.key_pressed(Key::F7) {
                action = Some(FocusConsole);
            } else if input.key_pressed(Key::F8) {
                action = Some(Transaction(TxAction::Begin));
            } else if input.key_pressed(Key::F9) {
                action = Some(Transaction(TxAction::Commit));
            } else if input.key_pressed(Key::F10) {
                action = Some(Transaction(TxAction::Rollback));
            } else if input.key_pressed(Key::F11) {
                action = Some(Settings);
            } else if input.key_pressed(Key::F12) {
                action = Some(CycleTheme);
            }
        });

        let Some(action) = action else {
            return;
        };
        match action {
            Shortcut::Quit => self.app.request_quit(),
            Shortcut::NewConnection => self.app.open_profile_form(None),
            Shortcut::SwitchConnection => self.app.open_connection_picker(),
            Shortcut::NewTab => self.app.new_query_tab(),
            Shortcut::CloseTab => self.app.request_close_active_tab(),
            Shortcut::OpenFile => open_sql_dialog(&mut self.app),
            Shortcut::SaveFile => self.app.open_save_query_dialog(),
            Shortcut::Export => self.app.open_export_dialog(),
            Shortcut::Filter => self.app.open_filter_dialog(),
            Shortcut::History => self.app.open_history(),
            Shortcut::Disconnect if connected => self.app.request_disconnect(),
            Shortcut::Settings => self.app.open_settings(),
            Shortcut::Complete if workspace => {
                self.app.focus = Focus::Editor;
                self.app.open_completion();
            }
            Shortcut::Execute => {
                if workspace {
                    self.app.execute_current();
                } else if connected {
                    self.app.focus_pane(Focus::Editor);
                } else {
                    self.app.activate_dashboard();
                }
            }
            Shortcut::NextTab => self.app.next_tab(),
            Shortcut::PreviousTab => self.app.previous_tab(),
            Shortcut::CancelQuery => self.app.cancel_current_query(),
            Shortcut::Help => self.app.open_help(),
            Shortcut::Dashboard => {
                if self.app.view == View::Workspace {
                    self.app.open_dashboard();
                } else if connected {
                    self.app.focus_pane(Focus::Editor);
                } else {
                    self.app.open_dashboard();
                }
            }
            Shortcut::FocusSidebar => self.app.focus_pane(Focus::Sidebar),
            Shortcut::FocusEditor => self.app.focus_pane(Focus::Editor),
            Shortcut::FocusResults => self.app.focus_pane(Focus::Results),
            Shortcut::FocusConsole => self.app.focus_pane(Focus::Logs),
            Shortcut::Transaction(action) => self.app.run_transaction(action),
            Shortcut::CycleTheme => self.app.cycle_theme(),
            _ => {}
        }
        ctx.request_repaint();
    }

    fn show_menu(&mut self, ctx: &egui::Context) {
        let palette = Palette::for_theme(self.app.theme_kind());
        egui::TopBottomPanel::top("rustquery_menu")
            .frame(
                Frame::new()
                    .fill(palette.panel)
                    .inner_margin(Margin::symmetric(10, 6)),
            )
            .show(ctx, |ui| {
                egui::menu::bar(ui, |ui| {
                    ui.menu_button("File", |ui| {
                        if ui.button("New query tab\tCtrl+T").clicked() {
                            self.app.new_query_tab();
                            ui.close_menu();
                        }
                        if ui.button("Open SQL file…\tCtrl+O").clicked() {
                            open_sql_dialog(&mut self.app);
                            ui.close_menu();
                        }
                        let can_save = self
                            .app
                            .active_tab()
                            .map(|tab| tab.editor.has_content())
                            .unwrap_or(false);
                        if ui
                            .add_enabled(can_save, egui::Button::new("Save SQL file…\tCtrl+S"))
                            .clicked()
                        {
                            self.app.open_save_query_dialog();
                            ui.close_menu();
                        }
                        if ui
                            .add_enabled(
                                self.app.results_visible(),
                                egui::Button::new("Export results…\tCtrl+E"),
                            )
                            .clicked()
                        {
                            self.app.open_export_dialog();
                            ui.close_menu();
                        }
                        ui.separator();
                        if ui.button("Quit\tCtrl+Q").clicked() {
                            self.app.request_quit();
                            ui.close_menu();
                        }
                    });

                    ui.menu_button("Session", |ui| {
                        if ui.button("Connection dashboard\tF2").clicked() {
                            self.app.open_dashboard();
                            ui.close_menu();
                        }
                        if ui.button("New connection…\tCtrl+Shift+N").clicked() {
                            self.app.open_profile_form(None);
                            ui.close_menu();
                        }
                        if ui.button("New folder…").clicked() {
                            self.app.prompt_new_folder();
                            ui.close_menu();
                        }
                        let connected = self.app.session.is_some();
                        let can_connect =
                            !self.app.profiles.is_empty() && self.app.connect.is_none();
                        if ui
                            .add_enabled(can_connect && !connected, egui::Button::new("Connect…"))
                            .clicked()
                        {
                            self.app.connect_to_profile_at(self.app.dashboard.selected);
                            ui.close_menu();
                        }
                        if ui
                            .add_enabled(
                                connected && self.app.connect.is_none(),
                                egui::Button::new("Switch connection…\tCtrl+Shift+O"),
                            )
                            .clicked()
                        {
                            self.app.open_connection_picker();
                            ui.close_menu();
                        }
                        if ui
                            .add_enabled(connected, egui::Button::new("Disconnect\tCtrl+D"))
                            .clicked()
                        {
                            self.app.request_disconnect();
                            ui.close_menu();
                        }
                        if ui
                            .add_enabled(connected, egui::Button::new("Refresh schema"))
                            .clicked()
                        {
                            self.app.refresh_schema();
                            ui.close_menu();
                        }
                        ui.separator();
                        let tx = self
                            .app
                            .session
                            .as_ref()
                            .map(|session| session.tx_active)
                            .unwrap_or(false);
                        if ui
                            .add_enabled(connected && !tx, egui::Button::new("BEGIN\tF8"))
                            .clicked()
                        {
                            self.app.run_transaction(TxAction::Begin);
                            ui.close_menu();
                        }
                        if ui
                            .add_enabled(connected && tx, egui::Button::new("COMMIT\tF9"))
                            .clicked()
                        {
                            self.app.run_transaction(TxAction::Commit);
                            ui.close_menu();
                        }
                        if ui
                            .add_enabled(connected && tx, egui::Button::new("ROLLBACK\tF10"))
                            .clicked()
                        {
                            self.app.run_transaction(TxAction::Rollback);
                            ui.close_menu();
                        }
                    });

                    ui.menu_button("View", |ui| {
                        if ui
                            .add_enabled(
                                self.app.session.is_some(),
                                egui::Button::new("Workspace\tF2"),
                            )
                            .clicked()
                        {
                            self.app.focus_pane(Focus::Editor);
                            ui.close_menu();
                        }
                        let mut logs = self.app.settings.show_logs;
                        if ui.checkbox(&mut logs, "Show console").changed() {
                            self.app.settings.show_logs = logs;
                            self.app.save_settings_now();
                        }
                        if ui.button("Cycle theme\tF12").clicked() {
                            self.app.cycle_theme();
                            ui.close_menu();
                        }
                        ui.separator();
                        if ui.button("History…\tCtrl+H").clicked() {
                            self.app.open_history();
                            ui.close_menu();
                        }
                        if ui.button("Settings…\tCtrl+,").clicked() {
                            self.app.open_settings();
                            ui.close_menu();
                        }
                    });

                    ui.menu_button("Help", |ui| {
                        if ui.button("Keyboard shortcuts\tF1").clicked() {
                            self.app.open_help();
                            ui.close_menu();
                        }
                        if ui.button("About RustQuery").clicked() {
                            self.app.modal = Some(Modal::Info(crate::app::InfoDialog {
                                title: "About RustQuery".to_string(),
                                lines: about_lines(),
                                scroll: 0,
                            }));
                            ui.close_menu();
                        }
                    });
                });

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .button(RichText::new("Help").color(palette.subtext))
                        .clicked()
                    {
                        self.app.open_help();
                    }
                    if ui
                        .button(RichText::new("Settings").color(palette.subtext))
                        .clicked()
                    {
                        self.app.open_settings();
                    }
                    let theme = self.app.theme_kind();
                    if ui
                        .button(RichText::new(theme.label()).color(palette.accent))
                        .on_hover_text("Cycle colour theme (F12)")
                        .clicked()
                    {
                        self.app.cycle_theme();
                    }
                });
            });
    }

    fn show_status(&mut self, ctx: &egui::Context) {
        let palette = Palette::for_theme(self.app.theme_kind());
        let bar_h = theme::metrics(self.app.settings.ui_scale_percent).status_bar_height;
        egui::TopBottomPanel::bottom("rustquery_status")
            .exact_height(bar_h)
            .frame(
                Frame::new()
                    .fill(palette.panel)
                    .inner_margin(Margin::symmetric(12, 5)),
            )
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    let (dot, color) = if self.app.session.is_some() {
                        ("●", palette.green)
                    } else {
                        ("○", palette.muted)
                    };
                    ui.label(RichText::new(dot).color(color));
                    ui.label(RichText::new(self.app.status_summary()).color(palette.subtext));
                    if let Some(session) = &self.app.session {
                        if session.tx_active {
                            ui.separator();
                            ui.label(
                                RichText::new("TRANSACTION OPEN")
                                    .color(palette.yellow)
                                    .small()
                                    .strong(),
                            );
                        }
                    }
                    if self.app.running_count() > 0 {
                        ui.separator();
                        ui.spinner();
                        ui.label(
                            RichText::new(format!("{} running", self.app.running_count()))
                                .color(palette.yellow)
                                .small(),
                        );
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new(self.app.theme_kind().label())
                                .color(palette.muted)
                                .small(),
                        );
                        ui.separator();
                        if let Some(tab) = self.app.active_tab() {
                            let (line, col) = tab.editor.cursor_line_col();
                            ui.label(
                                RichText::new(format!("Ln {line}, Col {col}"))
                                    .color(palette.subtext)
                                    .small(),
                            );
                            if tab.editor.has_selection() {
                                ui.separator();
                                ui.label(RichText::new("SEL").color(palette.accent).small());
                            }
                        }
                        ui.separator();
                        let focus = self.app.focus.label();
                        ui.label(
                            RichText::new(format!("Focus · {focus}"))
                                .color(palette.muted)
                                .small(),
                        );
                    });
                });
            });
    }

    fn show_toast(&self, ctx: &egui::Context) {
        let Some(toast) = &self.app.toast else {
            return;
        };
        let palette = Palette::for_theme(self.app.theme_kind());
        let color = palette.log_color(toast.level);
        egui::Area::new(egui::Id::new("rustquery_toast"))
            .order(egui::Order::Foreground)
            .anchor(Align2::RIGHT_BOTTOM, egui::vec2(-18.0, -38.0))
            .show(ctx, |ui| {
                Frame::new()
                    .fill(palette.panel)
                    .stroke(egui::Stroke::new(1.5_f32, color))
                    .corner_radius(8)
                    .inner_margin(Margin::symmetric(12, 8))
                    .shadow(egui::epaint::Shadow {
                        offset: [0, 2],
                        blur: 10,
                        spread: 0,
                        color: Color32::from_black_alpha(if palette.light { 28 } else { 72 }),
                    })
                    .show(ui, |ui| {
                        ui.set_min_width(200.0);
                        ui.set_max_width(420.0);
                        ui.label(RichText::new(&toast.text).color(palette.text).strong());
                        ui.label(RichText::new(toast.level.label()).small().color(color));
                    });
            });
    }
}

#[derive(Debug, Clone, Copy)]
enum Shortcut {
    Quit,
    NewConnection,
    SwitchConnection,
    NewTab,
    CloseTab,
    OpenFile,
    SaveFile,
    Export,
    Filter,
    History,
    Disconnect,
    Settings,
    Complete,
    Execute,
    NextTab,
    PreviousTab,
    CancelQuery,
    Help,
    Dashboard,
    FocusSidebar,
    FocusEditor,
    FocusResults,
    FocusConsole,
    Transaction(TxAction),
    CycleTheme,
}

impl eframe::App for DesktopApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // `App` spawns database tasks with `tokio::spawn`; entering the runtime
        // around every frame makes those calls valid from the UI thread.
        let _guard = self.runtime.enter();

        self.drain_workers();
        self.app.tick();
        self.handle_shortcuts(ctx);
        self.handle_close_request(ctx);

        let theme = self.app.theme_kind();
        let scale = self.app.settings.ui_scale_percent;
        if self.applied_theme != Some(theme) || self.applied_ui_scale != Some(scale) {
            theme::apply(ctx, theme, scale);
            self.applied_theme = Some(theme);
            self.applied_ui_scale = Some(scale);
        }

        let palette = Palette::for_theme(theme);
        self.show_menu(ctx);
        self.show_status(ctx);

        if self.app.view == View::Dashboard {
            egui::CentralPanel::default()
                .frame(Frame::new().fill(palette.bg))
                .show(ctx, |ui| dashboard::show(&mut self.app, ui));
        } else {
            // The explorer is resizable. Store its width in the existing
            // settings field (in approximate character columns) so the layout
            // survives restarts and remains compatible with older configs.
            let sidebar_width = (self.app.settings.sidebar_width as f32 * 8.0).clamp(200.0, 560.0);
            egui::SidePanel::left("rustquery_explorer")
                .resizable(true)
                .default_width(sidebar_width)
                .width_range(200.0..=560.0)
                .frame(Frame::new().fill(palette.panel).stroke(
                    if self.app.focus == Focus::Sidebar {
                        egui::Stroke::new(1.5_f32, palette.accent)
                    } else {
                        egui::Stroke::new(0.0_f32, Color32::TRANSPARENT)
                    },
                ))
                .show(ctx, |ui| {
                    workspace::show_sidebar(&mut self.app, ui);
                    let width = ui.available_width().max(200.0);
                    let columns = (width / 8.0).clamp(20.0, 80.0) as u16;
                    if columns != self.app.settings.sidebar_width {
                        self.app.settings.sidebar_width = columns;
                    }
                });
            egui::CentralPanel::default()
                .frame(Frame::new().fill(palette.bg))
                .show(ctx, |ui| workspace::show(&mut self.app, ui));
        }

        modals::show(&mut self.app, ctx);
        self.show_toast(ctx);

        let new_title = window_title(&self.app);
        if new_title != self.title {
            self.title = new_title.clone();
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(new_title));
        }

        if self.app.should_quit {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }

        let busy = self.app.running_count() > 0
            || self.app.current_running()
            || self.app.connect.is_some()
            || self.app.tx_pending.is_some();
        if busy || self.app.toast.is_some() {
            ctx.request_repaint_after(Duration::from_millis(100));
        } else if self.app.workspace_dirty {
            ctx.request_repaint_after(Duration::from_secs(1));
        }
    }
}

impl Drop for DesktopApp {
    fn drop(&mut self) {
        let _guard = self.runtime.enter();
        self.app.save_workspace_now();
        self.app.save_settings_now();
        self.app.save_history_now();
    }
}

fn window_title(app: &App) -> String {
    let mut title = String::from("RustQuery");
    if let Some(session) = &app.session {
        title.push_str(" · ");
        title.push_str(&session.info.profile_name);
        title.push_str(" · ");
        title.push_str(&session.info.current_database);
    }
    if let Some(tab) = app.active_tab() {
        title.push_str(" — ");
        title.push_str(&tab.display_title());
    }
    title
}

fn about_lines() -> Vec<String> {
    let mut lines = vec![
        format!("RustQuery {}", env!("CARGO_PKG_VERSION")),
        "A fast desktop SQL workbench for PostgreSQL, MySQL and SQLite.".to_string(),
        String::new(),
        "Built with Rust, eframe/egui, Tokio and SQLx.".to_string(),
        "Licensed under the MIT License.".to_string(),
    ];
    lines.push(String::new());
    lines.push("SQLite support is embedded.".to_string());
    lines.push("PostgreSQL and MySQL require a reachable database server.".to_string());
    lines
}

pub(crate) fn open_sql_dialog(app: &mut App) {
    let mut dialog = rfd::FileDialog::new()
        .set_title("Open SQL file")
        .add_filter("SQL scripts", &["sql"])
        .add_filter("All files", &["*"]);
    if let Some(dir) = app.last_directory_hint.as_deref() {
        dialog = dialog.set_directory(dir);
    }
    let Some(path) = dialog.pick_file() else {
        return;
    };
    if let Err(err) = app.open_sql_file(&path) {
        app.toast(LogLevel::Error, err);
    }
}

pub(crate) fn pick_export_path(current: &str, extension: &str) -> Option<PathBuf> {
    let current_path = crate::config::expand_tilde(std::path::Path::new(current));
    let mut dialog = rfd::FileDialog::new()
        .set_title("Export results")
        .add_filter(extension.to_ascii_uppercase(), &[extension])
        .add_filter("All files", &["*"]);
    if let Some(parent) = current_path.parent().filter(|p| p.is_dir()) {
        dialog = dialog.set_directory(parent);
    }
    if let Some(name) = current_path.file_name().and_then(|name| name.to_str()) {
        dialog = dialog.set_file_name(name);
    }
    dialog.save_file()
}

pub(crate) fn pick_sqlite_file(current: &str) -> Option<PathBuf> {
    let current_path = crate::config::expand_tilde(std::path::Path::new(current));
    let mut dialog = rfd::FileDialog::new()
        .set_title("Choose SQLite database")
        .add_filter("SQLite database", &["db", "sqlite", "sqlite3"])
        .add_filter("All files", &["*"]);
    if let Some(parent) = current_path.parent().filter(|p| p.is_dir()) {
        dialog = dialog.set_directory(parent);
    }
    dialog.pick_file()
}
