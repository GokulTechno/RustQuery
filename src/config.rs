//! Persistent configuration, saved queries/history and workspace state.
//!
//! Files live in `$RUSTQUERY_CONFIG_DIR`, `$XDG_CONFIG_HOME/rustquery`, or
//! `~/.config/rustquery` (in that order). Writes are atomic (temporary file +
//! rename) and private (`0600` on Unix). Corrupt files are backed up rather
//! than silently discarded.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::model::{ConnectionFolder, ConnectionProfile, HistoryEntry, SavedTab};

/// UI colour scheme choice. The concrete palette lives in `desktop::theme`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ThemeKind {
    #[default]
    Mocha,
    Nord,
    Dracula,
    Latte,
}

impl ThemeKind {
    pub const ALL: [ThemeKind; 4] = [
        ThemeKind::Mocha,
        ThemeKind::Nord,
        ThemeKind::Dracula,
        ThemeKind::Latte,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ThemeKind::Mocha => "Catppuccin Mocha",
            ThemeKind::Nord => "Nord",
            ThemeKind::Dracula => "Dracula",
            ThemeKind::Latte => "Catppuccin Latte",
        }
    }

    pub fn next(self) -> Self {
        let idx = Self::ALL.iter().position(|t| *t == self).unwrap_or(0);
        Self::ALL[(idx + 1) % Self::ALL.len()]
    }
}

/// User-controlled application settings, persisted between runs.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub theme: ThemeKind,
    /// Maximum rows fetched per query.
    pub row_limit: usize,
    /// Wall-clock query timeout.
    pub query_timeout_secs: u64,
    /// Percentage of the vertical workspace used by the editor.
    pub editor_ratio: u16,
    /// Width of the schema explorer in columns.
    pub sidebar_width: u16,
    /// Show the execution log at the bottom of the workspace.
    pub show_logs: bool,
    /// Interface font scale in percent (90–130). Affects editor, grids and chrome.
    #[serde(default = "default_ui_scale_percent")]
    pub ui_scale_percent: u16,
}

fn default_ui_scale_percent() -> u16 {
    115
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: ThemeKind::Mocha,
            row_limit: 1_000,
            query_timeout_secs: 30,
            editor_ratio: 42,
            sidebar_width: 34,
            show_logs: true,
            ui_scale_percent: 115,
        }
    }
}

impl Settings {
    pub fn sanitize(&mut self) {
        self.row_limit = self.row_limit.clamp(10, 100_000);
        self.query_timeout_secs = self.query_timeout_secs.clamp(1, 3_600);
        self.editor_ratio = self.editor_ratio.clamp(20, 80);
        self.sidebar_width = self.sidebar_width.clamp(20, 80);
        self.ui_scale_percent = self.ui_scale_percent.clamp(90, 130);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsField {
    RowLimit,
    Timeout,
    EditorRatio,
    SidebarWidth,
    UiScale,
    Theme,
    ShowLogs,
}

impl SettingsField {
    pub const ALL: [SettingsField; 7] = [
        SettingsField::RowLimit,
        SettingsField::Timeout,
        SettingsField::EditorRatio,
        SettingsField::SidebarWidth,
        SettingsField::UiScale,
        SettingsField::Theme,
        SettingsField::ShowLogs,
    ];

    pub fn label(self) -> &'static str {
        match self {
            SettingsField::RowLimit => "Row limit",
            SettingsField::Timeout => "Query timeout (s)",
            SettingsField::EditorRatio => "Editor height (%)",
            SettingsField::SidebarWidth => "Explorer width (columns)",
            SettingsField::UiScale => "UI font scale (%)",
            SettingsField::Theme => "Colour theme",
            SettingsField::ShowLogs => "Show console",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct ProfilesFile {
    #[serde(default)]
    profiles: Vec<ConnectionProfile>,
    #[serde(default)]
    folders: Vec<ConnectionFolder>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct HistoryFile {
    #[serde(default)]
    entries: Vec<HistoryEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WorkspaceFile {
    #[serde(default)]
    pub tabs: Vec<SavedTab>,
    #[serde(default)]
    pub active: usize,
}

#[derive(Debug, Clone)]
pub struct ConfigPaths {
    pub root: PathBuf,
    pub profiles: PathBuf,
    pub settings: PathBuf,
    pub history: PathBuf,
    pub workspace: PathBuf,
}

impl ConfigPaths {
    pub fn discover() -> Self {
        let root = std::env::var_os("RUSTQUERY_CONFIG_DIR")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("XDG_CONFIG_HOME")
                    .map(PathBuf::from)
                    .map(|p| p.join("rustquery"))
            })
            .or_else(|| {
                std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .map(|p| p.join(".config").join("rustquery"))
            })
            .unwrap_or_else(|| PathBuf::from(".rustquery"));

        Self {
            profiles: root.join("profiles.json"),
            settings: root.join("settings.json"),
            history: root.join("history.json"),
            workspace: root.join("workspace.json"),
            root,
        }
    }

    fn ensure(&self) -> Result<()> {
        fs::create_dir_all(&self.root)
            .with_context(|| format!("creating config directory {}", self.root.display()))?;
        Ok(())
    }
}

#[derive(Debug, Default)]
pub struct LoadReport {
    pub profiles: Vec<ConnectionProfile>,
    pub folders: Vec<ConnectionFolder>,
    pub settings: Settings,
    pub history: Vec<HistoryEntry>,
    pub workspace: WorkspaceFile,
    pub warnings: Vec<String>,
}

/// File-backed storage for all user state.
#[derive(Debug, Clone)]
pub struct Store {
    pub paths: ConfigPaths,
}

impl Store {
    pub fn discover() -> Self {
        Self {
            paths: ConfigPaths::discover(),
        }
    }

    pub fn with_paths(paths: ConfigPaths) -> Self {
        Self { paths }
    }

    /// Load every state file, collecting non-fatal warnings for the console.
    pub fn load(&self) -> LoadReport {
        let mut report = LoadReport::default();

        let (profiles_file, warn) =
            read_json_or_default::<ProfilesFile>(&self.paths.profiles, "profiles");
        report.profiles = profiles_file.profiles;
        report.folders = profiles_file.folders;
        crate::explorer::normalize_folders(&report.folders, &mut report.profiles);
        report.warnings.extend(warn);

        let (settings, warn) = read_json_or_default::<Settings>(&self.paths.settings, "settings");
        report.settings = settings;
        report.settings.sanitize();
        report.warnings.extend(warn);

        let (history_file, warn) =
            read_json_or_default::<HistoryFile>(&self.paths.history, "history");
        report.history = history_file.entries;
        if report.history.len() > 500 {
            report.history.truncate(500);
        }
        report.warnings.extend(warn);

        let (workspace, warn) =
            read_json_or_default::<WorkspaceFile>(&self.paths.workspace, "workspace");
        report.workspace = workspace;
        report.warnings.extend(warn);

        if self.paths.profiles.exists() {
            secure_permissions(&self.paths.profiles);
        }
        report
    }

    pub fn save_profiles(
        &self,
        profiles: &[ConnectionProfile],
        folders: &[ConnectionFolder],
    ) -> Result<()> {
        let file = ProfilesFile {
            profiles: profiles.to_vec(),
            folders: folders.to_vec(),
        };
        write_json(&self.paths, &self.paths.profiles, &file)
    }

    pub fn save_settings(&self, settings: &Settings) -> Result<()> {
        write_json(&self.paths, &self.paths.settings, settings)
    }

    pub fn save_history(&self, history: &[HistoryEntry]) -> Result<()> {
        let file = HistoryFile {
            entries: history.to_vec(),
        };
        write_json(&self.paths, &self.paths.history, &file)
    }

    pub fn save_workspace(&self, workspace: &WorkspaceFile) -> Result<()> {
        write_json(&self.paths, &self.paths.workspace, workspace)
    }

    /// Save a SQL script to an arbitrary path (Ctrl+S).
    pub fn save_query_file(&self, path: &Path, sql: &str) -> Result<()> {
        let path = expand_tilde(path);
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
        }
        atomic_write(&path, sql.as_bytes())?;
        Ok(())
    }
}

/// Read JSON, falling back to defaults. Corrupt files are renamed with a
/// timestamp so the user can recover them manually.
fn read_json_or_default<T: DeserializeOwned + Default>(
    path: &Path,
    label: &str,
) -> (T, Option<String>) {
    if !path.exists() {
        return (T::default(), None);
    }
    let text = match fs::read_to_string(path) {
        Ok(t) => t,
        Err(err) => {
            return (
                T::default(),
                Some(format!(
                    "Could not read {label} file {}: {err}",
                    path.display()
                )),
            )
        }
    };
    match serde_json::from_str::<T>(&text) {
        Ok(value) => (value, None),
        Err(err) => {
            let backup = path.with_extension(format!(
                "corrupt-{}.json",
                chrono::Utc::now().format("%Y%m%d-%H%M%S")
            ));
            let backup_note = match fs::rename(path, &backup) {
                Ok(()) => format!(" Backed up to {}.", backup.display()),
                Err(_) => String::new(),
            };
            (
                T::default(),
                Some(format!(
                    "Could not parse {label} file {}: {err}.{backup_note}",
                    path.display()
                )),
            )
        }
    }
}

fn write_json<T: Serialize>(paths: &ConfigPaths, target: &Path, value: &T) -> Result<()> {
    paths.ensure()?;
    let data = serde_json::to_vec_pretty(value).context("serialising configuration")?;
    atomic_write(target, &data)
}

fn atomic_write(target: &Path, data: &[u8]) -> Result<()> {
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating directory {}", parent.display()))?;
    }
    let tmp = target.with_extension(format!(
        "{}.tmp-{}",
        target
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("json"),
        std::process::id()
    ));

    {
        let mut file = fs::File::create(&tmp)
            .with_context(|| format!("creating temporary file {}", tmp.display()))?;
        file.write_all(data)
            .with_context(|| format!("writing {}", tmp.display()))?;
        file.sync_all().ok();
        secure_permissions(&tmp);
    }

    if let Err(first_err) = fs::rename(&tmp, target) {
        // Windows will not replace an existing file.
        let _ = fs::remove_file(target);
        fs::rename(&tmp, target).with_context(|| {
            format!(
                "renaming {} to {} (initial error: {first_err})",
                tmp.display(),
                target.display()
            )
        })?;
    }
    secure_permissions(target);
    Ok(())
}

#[cfg(unix)]
fn secure_permissions(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
fn secure_permissions(_path: &Path) {}

/// Expand a leading `~` (and `~/`) using `$HOME`.
pub fn expand_tilde(path: &Path) -> PathBuf {
    let raw = path.to_string_lossy();
    if raw == "~" || raw.starts_with("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            let rest = raw.strip_prefix("~/").unwrap_or("");
            return PathBuf::from(home).join(rest);
        }
    }
    path.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_are_clamped() {
        let mut s = Settings {
            row_limit: 500_000,
            query_timeout_secs: 0,
            editor_ratio: 3,
            sidebar_width: 500,
            ..Settings::default()
        };
        s.sanitize();
        assert_eq!(s.row_limit, 100_000);
        assert_eq!(s.query_timeout_secs, 1);
        assert_eq!(s.editor_ratio, 20);
        assert_eq!(s.sidebar_width, 80);
    }

    #[test]
    fn tilde_expansion() {
        std::env::set_var("HOME", "/tmp/rq-home");
        assert_eq!(
            expand_tilde(Path::new("~/a.sql")),
            PathBuf::from("/tmp/rq-home/a.sql")
        );
        assert_eq!(
            expand_tilde(Path::new("/x/a.sql")),
            PathBuf::from("/x/a.sql")
        );
    }
}
