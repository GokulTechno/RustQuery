//! RustQuery — a fast desktop SQL workbench for PostgreSQL, MySQL and SQLite.
//!
//! The binary is intentionally thin:
//!
//! * parse the few command-line options that matter before a window exists;
//! * load profiles, settings, history and the workspace from disk;
//! * create one multi-threaded Tokio runtime for database work;
//! * hand a [`DesktopApp`] to `eframe`.
//!
//! All application state and every database adapter lives in the `rustquery`
//! library crate so the desktop shell and integration tests share exactly the
//! same code.

use std::path::PathBuf;

use anyhow::{Context, Result};
use eframe::egui;
use rustquery::config::{ConfigPaths, Store};
use rustquery::desktop::DesktopApp;
use tokio::sync::mpsc;

fn main() -> Result<()> {
    let mut config_dir: Option<PathBuf> = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                print_help();
                return Ok(());
            }
            "-V" | "--version" => {
                println!("rustquery {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            "--config-dir" => {
                let Some(value) = args.next() else {
                    anyhow::bail!("--config-dir requires a path argument");
                };
                config_dir = Some(PathBuf::from(value));
            }
            other => {
                anyhow::bail!("unknown argument “{other}” (try --help)");
            }
        }
    }

    let store = match config_dir {
        Some(root) => Store::with_paths(ConfigPaths {
            profiles: root.join("profiles.json"),
            settings: root.join("settings.json"),
            history: root.join("history.json"),
            workspace: root.join("workspace.json"),
            root,
        }),
        None => Store::discover(),
    };
    let report = store.load();

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("rustquery-db")
        .build()
        .context("starting the Tokio runtime")?;
    let (worker_tx, worker_rx) = mpsc::unbounded_channel();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("RustQuery")
            .with_app_id("rustquery")
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([920.0, 600.0])
            .with_icon(window_icon()),
        ..Default::default()
    };

    eframe::run_native(
        "RustQuery",
        options,
        Box::new(move |cc| {
            Ok(Box::new(DesktopApp::new(
                cc, store, report, runtime, worker_tx, worker_rx,
            )))
        }),
    )
    .map_err(|err| anyhow::anyhow!("starting the desktop UI: {err}"))
}

/// Draw a small database-stack icon at runtime so the window and the packaged
/// launcher icon share the same visual identity without shipping a bitmap.
fn window_icon() -> egui::IconData {
    const SIZE: u32 = 64;
    let mut rgba = vec![0u8; (SIZE * SIZE * 4) as usize];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let fx = x as f32 / (SIZE - 1) as f32;
            let fy = y as f32 / (SIZE - 1) as f32;

            // Rounded-square background with a teal-to-indigo gradient.
            let inset = 4.0;
            let radius = 14.0;
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            let dx = (px - SIZE as f32 / 2.0).abs() - (SIZE as f32 / 2.0 - inset - radius);
            let dy = (py - SIZE as f32 / 2.0).abs() - (SIZE as f32 / 2.0 - inset - radius);
            let outside = dx.max(0.0).hypot(dy.max(0.0)) + dx.max(dy).min(0.0);
            if outside > radius {
                continue;
            }

            let mut color = [
                (13.0 + 74.0 * fx) as u8,
                (148.0 - 88.0 * fy) as u8,
                (136.0 + 46.0 * fy) as u8,
                255,
            ];

            // Three rounded bars suggest stacked database records.
            let bars = [(20.0, 26.0), (29.0, 35.0), (38.0, 44.0)];
            for (top, bottom) in bars {
                if (17.0..=47.0).contains(&px) && (top..=bottom).contains(&py) {
                    let edge = (py - top).min(bottom - py);
                    if edge > 1.2 || (px > 18.5 && px < 45.5) {
                        color = [248, 250, 252, 255];
                    }
                }
            }

            let index = ((y * SIZE + x) * 4) as usize;
            rgba[index..index + 4].copy_from_slice(&color);
        }
    }
    egui::IconData {
        rgba,
        width: SIZE,
        height: SIZE,
    }
}

fn print_help() {
    println!(
        "RustQuery {version} — a fast desktop SQL workbench for PostgreSQL, MySQL and SQLite.

Usage:
  rustquery [OPTIONS]

Options:
  --config-dir <PATH>   Use PATH for profiles.json, settings.json,
                        history.json and workspace.json
  -h, --help            Print this help and exit
  -V, --version         Print the version and exit

The desktop application stores its configuration in $XDG_CONFIG_HOME/rustquery
(or ~/.config/rustquery) unless --config-dir is given. The RUSTQUERY_CONFIG_DIR
environment variable is honoured as well.
",
        version = env!("CARGO_PKG_VERSION")
    );
}
