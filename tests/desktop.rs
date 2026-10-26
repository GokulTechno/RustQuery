//! Headless tests for the desktop presentation layer.
//!
//! The `eframe` shell itself needs a window, but almost all of its behaviour is
//! driven by plain data structures in [`rustquery::app`], [`rustquery::grid`]
//! and [`rustquery::config`]. These tests pin the desktop-facing helpers so a
//! refactor cannot silently break profile editing, exports or the result grid.

use rustquery::app::{ExportDialog, ProfileForm, TextInput};
use rustquery::config::{Settings, ThemeKind};
use rustquery::desktop::theme::{summary, Palette};
use rustquery::export::{ExportFormat, ExportScope};
use rustquery::grid::{ResultGrid, SortDirection};
use rustquery::model::{ColumnMeta, ConnectionProfile, Driver, QueryResult, SslMode};
use uuid::Uuid;

fn sample_result() -> QueryResult {
    QueryResult {
        columns: vec![
            ColumnMeta {
                name: "id".to_string(),
                type_name: "INTEGER".to_string(),
            },
            ColumnMeta {
                name: "name".to_string(),
                type_name: "TEXT".to_string(),
            },
        ],
        rows: vec![
            vec![Some("3".to_string()), Some("Carol".to_string())],
            vec![Some("1".to_string()), Some("Alice".to_string())],
            vec![Some("2".to_string()), Some("Bob".to_string())],
        ],
        affected: 0,
        duration_ms: 2,
        truncated: false,
        sql: "SELECT id, name FROM people".to_string(),
        result_sets: 1,
    }
}

#[test]
fn profile_form_builds_a_canonical_sqlite_profile() {
    let mut form = ProfileForm::new(None, 1);
    form.select_driver(Driver::Sqlite);
    assert!(form.is_field_visible(ProfileForm::DATABASE));
    assert!(!form.is_field_visible(ProfileForm::HOST));
    assert!(!form.is_field_visible(ProfileForm::PORT));

    form.name.set("Scratch");
    form.database.set(":memory:");
    let profile = form
        .build_profile()
        .expect("SQLite profile should validate");
    assert_eq!(profile.driver, Driver::Sqlite);
    assert_eq!(profile.name, "Scratch");
    assert_eq!(profile.database, ":memory:");
    assert_eq!(profile.host, "");
    assert_eq!(profile.port, 0);
    assert_eq!(profile.user, "");
    assert_eq!(profile.ssl_mode, SslMode::Disable);
}

#[test]
fn profile_form_reports_invalid_network_values() {
    let mut form = ProfileForm::new(None, 2);
    form.select_driver(Driver::Postgres);
    form.name.set("Production");
    form.port.set("not-a-port");
    let error = form.build_profile().expect_err("bad port must fail");
    assert!(error.contains("Port"), "unexpected error: {error}");

    form.port.set("5432");
    form.host.set("");
    let error = form.build_profile().expect_err("empty host must fail");
    assert!(error.contains("Host"), "unexpected error: {error}");
}

#[test]
fn export_dialog_rewrites_only_known_extensions() {
    let mut dialog = ExportDialog {
        tab_id: Uuid::nil(),
        format: ExportFormat::Csv,
        scope: ExportScope::Visible,
        include_headers: true,
        path: TextInput::new("~/result.csv"),
        table: TextInput::new("exported_data"),
        driver: Driver::Sqlite,
        error: None,
    };

    dialog.set_format(ExportFormat::Json);
    assert_eq!(dialog.path.value, "~/result.json");
    dialog.set_format(ExportFormat::Sql);
    assert_eq!(dialog.path.value, "~/result.sql");

    // A path the user typed with another extension is left alone.
    dialog.path.set("/tmp/report.txt");
    dialog.set_format(ExportFormat::Csv);
    assert_eq!(dialog.path.value, "/tmp/report.txt");

    assert_eq!(ExportScope::Visible.toggle(), ExportScope::All);
    assert_eq!(ExportFormat::Csv.next(), ExportFormat::Json);
}

#[test]
fn grid_filters_and_cycles_sort_directions() {
    let result = sample_result();
    let mut grid = ResultGrid::new(&result);
    assert_eq!(grid.visible_count(), 3);

    grid.set_filter(&result, "ali".to_string());
    assert_eq!(grid.visible_count(), 1);
    assert_eq!(grid.row_at(0), Some(1));
    assert!(grid.is_filtered());

    grid.clear_filter(&result);
    assert_eq!(grid.visible_count(), 3);

    grid.sel_col = 0;
    grid.cycle_sort(&result);
    let sort = grid.sort.expect("first cycle sorts ascending");
    assert_eq!(sort.column, 0);
    assert_eq!(sort.direction, SortDirection::Asc);
    assert_eq!(grid.row_at(0), Some(1));

    grid.cycle_sort(&result);
    assert_eq!(
        grid.sort.expect("second cycle sorts descending").direction,
        SortDirection::Desc
    );
    assert_eq!(grid.row_at(0), Some(0));

    grid.cycle_sort(&result);
    assert!(grid.sort.is_none());
}

#[test]
fn saved_profile_without_a_folder_still_loads() {
    let profile: ConnectionProfile = serde_json::from_str(
        r#"{
            "name": "Local",
            "driver": "postgres",
            "host": "localhost",
            "port": 5432,
            "database": "app",
            "user": "postgres"
        }"#,
    )
    .expect("profiles saved before folders should still load");
    assert!(profile.folder_id.is_none());
    assert_eq!(profile.host_label(), "localhost:5432");
    assert_eq!(profile.database_label(), "app");
}

#[test]
fn old_settings_with_mouse_field_still_load() {
    // `mouse` belonged to the terminal build. Serde must ignore it instead of
    // rejecting a user's existing settings file.
    let settings: Settings = serde_json::from_str(
        r#"{
            "theme": "nord",
            "row_limit": 500,
            "query_timeout_secs": 12,
            "editor_ratio": 40,
            "sidebar_width": 30,
            "show_logs": false,
            "mouse": true
        }"#,
    )
    .expect("legacy settings should deserialize");
    assert_eq!(settings.theme, ThemeKind::Nord);
    assert_eq!(settings.row_limit, 500);
    assert_eq!(settings.query_timeout_secs, 12);
    assert!(!settings.show_logs);
}

#[test]
fn every_theme_exposes_a_usable_palette() {
    assert_eq!(ThemeKind::Mocha.next(), ThemeKind::Nord);
    assert_eq!(ThemeKind::Latte.next(), ThemeKind::Mocha);

    for theme in ThemeKind::ALL {
        let palette = Palette::for_theme(theme);
        assert_ne!(palette.bg, palette.panel, "{}", theme.label());
        assert_ne!(palette.text, palette.bg, "{}", theme.label());
        assert!(
            summary(theme).contains(theme.label()),
            "summary should name {}",
            theme.label()
        );
    }
}
