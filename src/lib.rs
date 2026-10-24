//! RustQuery's reusable application library.
//!
//! The binary in `main.rs` is intentionally thin: it parses command-line
//! arguments, creates the Tokio runtime and hands control to the `eframe`
//! desktop shell. Every state machine, database adapter and view is exposed
//! here so integration tests can drive [`app::App`] without opening a window.
//!
//! Module overview:
//!
//! * [`app`] — central state, actions and worker orchestration.
//! * [`config`] — atomic JSON persistence for profiles/settings/history/workspace.
//! * [`db`] — pooled PostgreSQL/MySQL/SQLite connections, execution and introspection.
//! * [`desktop`] — `eframe` shell: menu, dashboard, workspace, editor and modals.
//! * [`editor`] — multi-line editor buffer, selections, undo and highlighting.
//! * [`grid`] — virtualised, filterable and sortable result-grid state.
//! * [`schema_tree`] — lazily loaded schema explorer state.
//! * [`export`] — CSV, JSON and SQL INSERT serialisation.
//! * [`clipboard`], [`model`] — shared value types and services.

// `DbError` carries rich diagnostics (SQLSTATE, hint, detail, line/column).
// Keeping it unboxed makes the error path allocation-free; results are produced
// once per operation and this size has no measurable UI impact.
#![allow(clippy::result_large_err)]

pub mod app;
pub mod clipboard;
pub mod config;
pub mod db;
pub mod desktop;
pub mod editor;
pub mod export;
pub mod grid;
pub mod model;
pub mod schema_tree;
