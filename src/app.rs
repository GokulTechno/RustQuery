//! Central application state and async worker orchestration.
//!
//! The desktop UI thread owns `App` and never blocks: every database operation
//! is spawned as a Tokio task whose result arrives through `worker_tx`/`worker_rx`
//! as a [`WorkerMessage`]. Queries carry request ids so stale results (closed
//! tabs, cancelled queries, replaced sessions) are ignored safely.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use chrono::{Local, Utc};
use tokio::sync::mpsc::UnboundedSender;
use tokio::task::AbortHandle;
use uuid::Uuid;

use crate::clipboard::ClipboardService;
use crate::config::{LoadReport, Settings, Store, ThemeKind, WorkspaceFile};
use crate::db::{self, DbError, Session};
use crate::editor::{Cursor, Editor, Motion};
use crate::explorer::{Catalog, CatalogToggle};
use crate::export::{self, ExportFormat, ExportScope};
use crate::grid::ResultGrid;
use crate::model::{
    format_count, ConnectionFolder, ConnectionProfile, Driver, HistoryEntry, LogEntry, LogLevel,
    QueryResult, SavedTab, SslMode, TestOutcome,
};
use crate::schema_tree::{RowKind, SchemaTree, TableLoadRequest, TreeAction};

const MAX_LOGS: usize = 1_000;
const WORKSPACE_AUTOSAVE: Duration = Duration::from_secs(5);
const HISTORY_AUTOSAVE: Duration = Duration::from_secs(2);
const TOAST_DURATION: Duration = Duration::from_millis(4_500);

// ---------------------------------------------------------------------------
// Async messages
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TestKey {
    Profile(Uuid),
    Form(u64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TxAction {
    Begin,
    Commit,
    Rollback,
}

impl TxAction {
    pub fn sql(self) -> &'static str {
        match self {
            TxAction::Begin => "BEGIN",
            TxAction::Commit => "COMMIT",
            TxAction::Rollback => "ROLLBACK",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            TxAction::Begin => "BEGIN",
            TxAction::Commit => "COMMIT",
            TxAction::Rollback => "ROLLBACK",
        }
    }
}

/// Messages produced by background tasks. `Session` never travels back after
/// queries; only results and errors do.
pub enum WorkerMessage {
    Connected {
        attempt: u64,
        result: Result<Session, DbError>,
    },
    SchemasLoaded {
        session_id: u64,
        result: Result<Vec<crate::model::SchemaEntry>, DbError>,
    },
    TableDetailsLoaded {
        session_id: u64,
        node_id: String,
        schema: String,
        table: String,
        result: Result<crate::model::TableDetails, DbError>,
    },
    TestFinished {
        key: TestKey,
        result: Result<TestOutcome, DbError>,
    },
    QueryFinished {
        session_id: u64,
        tab_id: Uuid,
        request_id: u64,
        result: Result<QueryResult, DbError>,
    },
    TransactionFinished {
        session_id: u64,
        action: TxAction,
        result: Result<(), DbError>,
    },
    ExportFinished {
        path: PathBuf,
        rows: usize,
        result: Result<u64, String>,
    },
}

// ---------------------------------------------------------------------------
// UI state types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Dashboard,
    Workspace,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Sidebar,
    Editor,
    Results,
    Logs,
}

impl Focus {
    pub fn label(self) -> &'static str {
        match self {
            Focus::Sidebar => "Explorer",
            Focus::Editor => "Editor",
            Focus::Results => "Results",
            Focus::Logs => "Console",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct DashboardState {
    pub selected: usize,
    pub tests: HashMap<Uuid, TestStatus>,
    pub connect_error: Option<String>,
}

#[derive(Debug, Clone)]
pub enum TestStatus {
    Testing(Instant),
    Ok(TestOutcome),
    Err(String),
}

#[derive(Debug, Clone)]
pub struct ConnectState {
    pub attempt: u64,
    pub profile: ConnectionProfile,
    pub abort: AbortHandle,
    pub started: Instant,
}

#[derive(Debug, Clone)]
pub struct Toast {
    pub text: String,
    pub level: LogLevel,
    pub until: Instant,
}

#[derive(Debug, Clone, Default)]
pub struct ConsoleState {
    pub scroll: u16,
    pub follow: bool,
    pub errors_only: bool,
    /// Populated by the renderer each frame so keyboard paging can be sensible.
    pub total_lines: u16,
    pub page: u16,
}

#[derive(Debug, Clone)]
pub struct CompletionState {
    pub items: Vec<String>,
    pub selected: usize,
    pub start: Cursor,
    pub prefix: String,
}

#[derive(Debug, Clone)]
pub struct RunningQuery {
    pub request_id: u64,
    pub started: Instant,
    pub sql: String,
}

#[derive(Debug, Clone)]
pub struct RunningTask {
    pub tab_id: Uuid,
    pub abort: AbortHandle,
    pub started: Instant,
    pub sql: String,
}

#[derive(Debug, Clone)]
pub struct TxPending {
    pub action: TxAction,
    pub abort: AbortHandle,
    pub started: Instant,
}

#[derive(Debug)]
pub struct QueryTab {
    pub id: Uuid,
    pub title: String,
    pub editor: Editor,
    pub result: Option<QueryResult>,
    pub grid: Option<ResultGrid>,
    pub last_error: Option<DbError>,
    pub running: Option<RunningQuery>,
    pub auto_focus_results: bool,
    pub is_preview: bool,
}

impl QueryTab {
    pub fn new(id: Uuid, title: String, sql: &str) -> Self {
        Self {
            id,
            title,
            editor: Editor::new(sql),
            result: None,
            grid: None,
            last_error: None,
            running: None,
            auto_focus_results: false,
            is_preview: false,
        }
    }

    pub fn display_title(&self) -> String {
        if self.editor.is_modified() {
            format!("{} •", self.title)
        } else {
            self.title.clone()
        }
    }
}

// ---------------------------------------------------------------------------
// Single-line input & modal state
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct TextInput {
    pub value: String,
    pub cursor: usize,
}

impl TextInput {
    pub fn new(value: impl Into<String>) -> Self {
        let cleaned = sanitize_single_line(&value.into());
        let cursor = cleaned.chars().count();
        Self {
            value: cleaned,
            cursor,
        }
    }

    pub fn set(&mut self, value: impl Into<String>) {
        *self = Self::new(value);
    }

    fn clamp(&mut self) {
        let len = self.value.chars().count();
        self.cursor = self.cursor.min(len);
    }

    pub fn insert_str(&mut self, text: &str) {
        let cleaned = sanitize_single_line(text);
        if cleaned.is_empty() {
            return;
        }
        self.clamp();
        let byte = self
            .value
            .char_indices()
            .nth(self.cursor)
            .map(|(idx, _)| idx)
            .unwrap_or(self.value.len());
        self.value.insert_str(byte, &cleaned);
        self.cursor += cleaned.chars().count();
    }

    pub fn insert_char(&mut self, c: char) {
        if c.is_control() {
            return;
        }
        let mut buf = [0u8; 4];
        self.insert_str(c.encode_utf8(&mut buf));
    }

    pub fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        self.remove_range(self.cursor - 1, self.cursor);
        self.cursor -= 1;
    }

    pub fn delete(&mut self) {
        let len = self.value.chars().count();
        if self.cursor >= len {
            return;
        }
        self.remove_range(self.cursor, self.cursor + 1);
    }

    fn remove_range(&mut self, start: usize, end: usize) {
        let chars: Vec<char> = self.value.chars().collect();
        let start = start.min(chars.len());
        let end = end.min(chars.len()).max(start);
        self.value = chars[..start].iter().chain(chars[end..].iter()).collect();
        self.clamp();
    }

    pub fn left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    pub fn right(&mut self) {
        if self.cursor < self.value.chars().count() {
            self.cursor += 1;
        }
    }

    pub fn word_left(&mut self) {
        let chars: Vec<char> = self.value.chars().collect();
        let mut i = self.cursor.min(chars.len());
        while i > 0 && !is_word_char(chars[i - 1]) {
            i -= 1;
        }
        while i > 0 && is_word_char(chars[i - 1]) {
            i -= 1;
        }
        self.cursor = i;
    }

    pub fn word_right(&mut self) {
        let chars: Vec<char> = self.value.chars().collect();
        let len = chars.len();
        let mut i = self.cursor.min(len);
        while i < len && !is_word_char(chars[i]) {
            i += 1;
        }
        while i < len && is_word_char(chars[i]) {
            i += 1;
        }
        self.cursor = i;
    }

    pub fn home(&mut self) {
        self.cursor = 0;
    }

    pub fn end(&mut self) {
        self.cursor = self.value.chars().count();
    }

    pub fn delete_word_left(&mut self) {
        let old = self.cursor;
        self.word_left();
        let start = self.cursor;
        self.remove_range(start, old);
        self.cursor = start;
    }

    pub fn clear(&mut self) {
        self.value.clear();
        self.cursor = 0;
    }

    pub fn before_after(&self) -> (String, String) {
        let chars: Vec<char> = self.value.chars().collect();
        let cursor = self.cursor.min(chars.len());
        (
            chars[..cursor].iter().collect(),
            chars[cursor..].iter().collect(),
        )
    }

    pub fn display_secret(&self) -> String {
        "•".repeat(self.value.chars().count())
    }
}

fn sanitize_single_line(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c == '\n' || c == '\r' || c == '\t' {
                ' '
            } else {
                c
            }
        })
        .filter(|c| !c.is_control())
        .collect()
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

pub struct ProfileForm {
    pub token: u64,
    pub edit_id: Option<Uuid>,
    pub driver: Driver,
    pub name: TextInput,
    pub host: TextInput,
    pub port: TextInput,
    pub database: TextInput,
    pub user: TextInput,
    pub password: TextInput,
    pub ssl_mode: SslMode,
    pub folder_id: Option<Uuid>,
    pub focus: usize,
    pub show_password: bool,
    pub testing: bool,
    pub test_result: Option<(bool, String)>,
    pub error: Option<String>,
}

impl ProfileForm {
    pub const DRIVER: usize = 0;
    pub const NAME: usize = 1;
    pub const HOST: usize = 2;
    pub const PORT: usize = 3;
    pub const DATABASE: usize = 4;
    pub const USER: usize = 5;
    pub const PASSWORD: usize = 6;
    pub const SSL: usize = 7;
    pub const FIELD_COUNT: usize = 8;

    pub fn new(profile: Option<&ConnectionProfile>, token: u64) -> Self {
        let base = profile
            .cloned()
            .unwrap_or_else(|| ConnectionProfile::new(Driver::Postgres).with_defaults());
        let edit_id = profile.map(|p| p.id);
        Self {
            token,
            edit_id,
            driver: base.driver,
            name: TextInput::new(base.name),
            host: TextInput::new(base.host),
            port: TextInput::new(base.port.to_string()),
            database: TextInput::new(base.database),
            user: TextInput::new(base.user),
            password: TextInput::new(base.password),
            ssl_mode: base.ssl_mode,
            folder_id: base.folder_id,
            focus: if profile.is_some() {
                Self::NAME
            } else {
                Self::DRIVER
            },
            show_password: false,
            testing: false,
            test_result: None,
            error: None,
        }
    }

    /// Whether a field applies to the currently selected driver.
    ///
    /// SQLite is file-based, so host/port/user/password/TLS are hidden instead
    /// of asking for values that would be ignored.
    pub fn is_field_visible(&self, field: usize) -> bool {
        if self.driver == Driver::Sqlite {
            matches!(field, Self::DRIVER | Self::NAME | Self::DATABASE)
        } else {
            field < Self::FIELD_COUNT
        }
    }

    pub fn field_mut(&mut self, focus: usize) -> Option<&mut TextInput> {
        if !self.is_field_visible(focus) {
            return None;
        }
        match focus {
            Self::NAME => Some(&mut self.name),
            Self::HOST => Some(&mut self.host),
            Self::PORT => Some(&mut self.port),
            Self::DATABASE => Some(&mut self.database),
            Self::USER => Some(&mut self.user),
            Self::PASSWORD => Some(&mut self.password),
            _ => None,
        }
    }

    pub fn next_focus(&mut self, backwards: bool) {
        for _ in 0..Self::FIELD_COUNT {
            self.focus = if backwards {
                (self.focus + Self::FIELD_COUNT - 1) % Self::FIELD_COUNT
            } else {
                (self.focus + 1) % Self::FIELD_COUNT
            };
            if self.is_field_visible(self.focus) {
                break;
            }
        }
    }

    pub fn cycle_driver(&mut self, forwards: bool) {
        let old_driver = self.driver;
        let old_port = old_driver.default_port();
        let old_user = old_driver.default_user();
        let old_database = old_driver.default_database();
        let old_name = format!("Local {}", old_driver.label());

        self.driver = if forwards {
            old_driver.next()
        } else {
            old_driver.previous()
        };

        // Only replace values that still look auto-generated, so switching
        // drivers does not discard user-entered settings.
        if self.name.value.trim().is_empty() || self.name.value.trim() == old_name {
            self.name.set(format!("Local {}", self.driver.label()));
        }
        if self.database.value.trim().is_empty() || self.database.value.trim() == old_database {
            self.database
                .set(self.driver.default_database().to_string());
        }
        if self.driver.is_networked() {
            if self.host.value.trim().is_empty() {
                self.host.set("localhost");
            }
            let port = self.port.value.trim();
            if port.is_empty()
                || port == "0"
                || !old_driver.is_networked()
                || port == old_port.to_string()
            {
                self.port.set(self.driver.default_port().to_string());
            }
            let user = self.user.value.trim();
            if user.is_empty() || !old_driver.is_networked() || user == old_user {
                self.user.set(self.driver.default_user());
            }
        }
    }

    /// Select a driver directly (desktop combo box). Reuses the same
    /// auto-generated-field safeguards as keyboard cycling.
    pub fn select_driver(&mut self, driver: Driver) {
        let mut guard = 0;
        while self.driver != driver && guard < Driver::ALL.len() {
            self.cycle_driver(true);
            guard += 1;
        }
    }

    pub fn cycle_ssl(&mut self, forwards: bool) {
        self.ssl_mode = if forwards {
            self.ssl_mode.next()
        } else {
            self.ssl_mode.previous()
        };
    }

    pub fn build_profile(&self) -> Result<ConnectionProfile, String> {
        let mut profile = if self.driver == Driver::Sqlite {
            ConnectionProfile {
                id: self.edit_id.unwrap_or_else(Uuid::new_v4),
                name: self.name.value.trim().to_string(),
                driver: self.driver,
                // SQLite ignores network settings. Store an empty, canonical
                // profile so the dashboard never shows a bogus host or TLS mode.
                host: String::new(),
                port: 0,
                database: self.database.value.trim().to_string(),
                user: String::new(),
                password: String::new(),
                ssl_mode: SslMode::Disable,
                folder_id: self.folder_id,
            }
        } else {
            let port = self
                .port
                .value
                .trim()
                .parse::<u16>()
                .map_err(|_| "Port must be a number between 1 and 65535.".to_string())?;
            ConnectionProfile {
                id: self.edit_id.unwrap_or_else(Uuid::new_v4),
                name: self.name.value.trim().to_string(),
                driver: self.driver,
                host: self.host.value.trim().to_string(),
                port,
                database: self.database.value.trim().to_string(),
                user: self.user.value.trim().to_string(),
                password: self.password.value.clone(),
                ssl_mode: self.ssl_mode,
                folder_id: self.folder_id,
            }
        };
        if profile.name.is_empty() {
            profile.name = format!("{} connection", profile.driver.label());
        }
        profile.validate()?;
        Ok(profile)
    }
}

pub struct ExportDialog {
    pub tab_id: Uuid,
    pub format: ExportFormat,
    pub scope: ExportScope,
    pub include_headers: bool,
    pub path: TextInput,
    pub table: TextInput,
    pub driver: Driver,
    pub error: Option<String>,
}

impl ExportDialog {
    fn new(tab: &QueryTab, driver: Driver) -> Self {
        let safe_title: String = tab
            .title
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let stamp = Local::now().format("%Y%m%d-%H%M%S");
        let path = format!("~/rustquery-{safe_title}-{stamp}.csv");
        Self {
            tab_id: tab.id,
            format: ExportFormat::Csv,
            scope: ExportScope::Visible,
            include_headers: true,
            path: TextInput::new(path),
            table: TextInput::new("exported_data"),
            driver,
            error: None,
        }
    }

    pub fn set_format(&mut self, format: ExportFormat) {
        let old_ext = self.format.extension();
        self.format = format;
        let value = self.path.value.clone();
        if value.to_ascii_lowercase().ends_with(&format!(".{old_ext}")) {
            let trimmed = &value[..value.len() - old_ext.len() - 1];
            self.path.set(format!("{trimmed}.{}", format.extension()));
        }
    }
}

pub struct FilterDialog {
    pub tab_id: Uuid,
    pub input: TextInput,
    pub original: String,
    pub error: Option<String>,
}

pub struct CellDetailDialog {
    pub title: String,
    pub column: String,
    pub type_name: String,
    pub value: Option<String>,
    pub row_number: usize,
    pub scroll: u16,
}

pub struct InfoDialog {
    pub title: String,
    pub lines: Vec<String>,
    pub scroll: u16,
}

pub struct HistoryBrowser {
    pub selected: usize,
    pub scroll: u16,
}

pub struct SettingsDialog {
    pub selected: usize,
    pub backup: Settings,
}

/// Pick another saved profile while staying in the workspace.
pub struct ConnectionPicker {
    pub selected: usize,
}

pub enum InputPurpose {
    SaveQuery { tab_id: Uuid },
    SidebarFilter,
    CreateFolder,
    RenameFolder { id: Uuid },
}

pub struct InputDialog {
    pub title: String,
    pub label: String,
    pub hint: String,
    pub input: TextInput,
    pub purpose: InputPurpose,
    pub error: Option<String>,
}

pub enum ConfirmAction {
    Quit,
    Disconnect,
    DeleteProfile(Uuid),
    DeleteFolder(Uuid),
    CloseTab(Uuid),
    Connect(Box<ConnectionProfile>),
}

impl ConfirmAction {
    /// Whether confirming discards data or closes a live session. These
    /// actions get a red primary button instead of the normal accent fill.
    pub fn is_destructive(&self) -> bool {
        matches!(
            self,
            ConfirmAction::Quit
                | ConfirmAction::Disconnect
                | ConfirmAction::DeleteProfile(_)
                | ConfirmAction::DeleteFolder(_)
                | ConfirmAction::CloseTab(_)
        )
    }
}

pub struct ConfirmDialog {
    pub title: String,
    pub message: String,
    pub action: ConfirmAction,
    pub yes: bool,
}

pub enum Modal {
    Help { scroll: u16 },
    Profile(ProfileForm),
    Confirm(ConfirmDialog),
    Export(ExportDialog),
    Filter(FilterDialog),
    Cell(CellDetailDialog),
    History(HistoryBrowser),
    Settings(SettingsDialog),
    Input(InputDialog),
    Info(InfoDialog),
    ConnectionPicker(ConnectionPicker),
}

// ---------------------------------------------------------------------------
// App
// ---------------------------------------------------------------------------

/// A connection that stays open while another profile is the editor's target.
struct RetainedSession {
    session: Session,
    tree: SchemaTree,
    schema_loading: bool,
}

pub struct App {
    pub store: Store,
    pub profiles: Vec<ConnectionProfile>,
    pub settings: Settings,
    pub history: Vec<HistoryEntry>,
    pub logs: Vec<LogEntry>,
    pub console: ConsoleState,
    pub tabs: Vec<QueryTab>,
    pub active_tab: usize,
    pub tab_counter: usize,
    pub view: View,
    pub focus: Focus,
    pub tree: SchemaTree,
    pub schema_loading: bool,
    pub session: Option<Session>,
    /// Sessions opened from the explorer that stay connected until quit.
    retained: HashMap<Uuid, RetainedSession>,
    pub catalog: Catalog,
    pub connect: Option<ConnectState>,
    pub dashboard: DashboardState,
    pub modal: Option<Modal>,
    pub completion: Option<CompletionState>,
    pub clipboard: ClipboardService,
    pub worker_tx: UnboundedSender<WorkerMessage>,
    pub running: HashMap<u64, RunningTask>,
    pub next_request: u64,
    pub next_attempt: u64,
    pub next_form_token: u64,
    pub tx_pending: Option<TxPending>,
    pub should_quit: bool,
    pub toast: Option<Toast>,
    pub workspace_dirty: bool,
    pub last_workspace_save: Instant,
    pub workspace_save_failed: bool,
    pub history_dirty: bool,
    pub last_history_save: Instant,
    pub history_save_failed: bool,
    pub started: Instant,
    pub last_directory_hint: Option<String>,
}

impl App {
    pub fn new(
        store: Store,
        report: LoadReport,
        worker_tx: UnboundedSender<WorkerMessage>,
    ) -> Self {
        let mut app = Self {
            store,
            profiles: report.profiles,
            settings: report.settings,
            history: report.history,
            logs: Vec::new(),
            console: ConsoleState {
                scroll: 0,
                follow: true,
                errors_only: false,
                total_lines: 0,
                page: 10,
            },
            tabs: Vec::new(),
            active_tab: 0,
            tab_counter: 0,
            view: View::Workspace,
            focus: Focus::Sidebar,
            tree: SchemaTree::new(),
            schema_loading: false,
            session: None,
            retained: HashMap::new(),
            catalog: Catalog::default(),
            connect: None,
            dashboard: DashboardState::default(),
            modal: None,
            completion: None,
            clipboard: ClipboardService::new(),
            worker_tx,
            running: HashMap::new(),
            next_request: 1,
            next_attempt: 1,
            next_form_token: 1,
            tx_pending: None,
            should_quit: false,
            toast: None,
            workspace_dirty: false,
            last_workspace_save: Instant::now(),
            workspace_save_failed: false,
            history_dirty: false,
            last_history_save: Instant::now(),
            history_save_failed: false,
            started: Instant::now(),
            last_directory_hint: None,
        };

        app.catalog = Catalog::from_saved(report.folders, &app.profiles);
        app.history.sort_by_key(|entry| std::cmp::Reverse(entry.at));
        app.history.truncate(500);

        let mut restored = 0usize;
        for saved in report.workspace.tabs.into_iter().take(32) {
            let title = if saved.title.trim().is_empty() {
                app.next_title()
            } else {
                saved.title
            };
            let id = Uuid::new_v4();
            app.tabs.push(QueryTab::new(id, title, &saved.sql));
            restored += 1;
        }
        if app.tabs.is_empty() {
            app.push_new_tab();
        }
        app.active_tab = report
            .workspace
            .active
            .min(app.tabs.len().saturating_sub(1));
        app.tab_counter = app.tab_counter.max(app.tabs.len());

        app.log(
            LogLevel::Info,
            format!(
                "RustQuery {} ready · {} saved connection{} · {} restored tab{}",
                env!("CARGO_PKG_VERSION"),
                app.profiles.len(),
                if app.profiles.len() == 1 { "" } else { "s" },
                restored,
                if restored == 1 { "" } else { "s" }
            ),
        );
        if app.profiles.is_empty() {
            app.log(
                LogLevel::Info,
                "No connection profiles yet. Use Connections > New connection to create one.",
            );
        }
        for warning in report.warnings {
            app.push_log(LogLevel::Warn, warning, None, Some("config".to_string()));
        }
        app
    }

    // -----------------------------------------------------------------
    // Tabs & titles
    // -----------------------------------------------------------------

    fn next_title(&mut self) -> String {
        self.tab_counter += 1;
        format!("Query {}", self.tab_counter)
    }

    pub fn push_new_tab(&mut self) -> Uuid {
        let title = self.next_title();
        let id = Uuid::new_v4();
        self.tabs.push(QueryTab::new(id, title, ""));
        self.active_tab = self.tabs.len() - 1;
        self.workspace_dirty = true;
        id
    }

    pub fn new_query_tab(&mut self) {
        self.push_new_tab();
        self.view = View::Workspace;
        self.focus = Focus::Editor;
        self.completion = None;
    }

    pub fn active_tab(&self) -> Option<&QueryTab> {
        self.tabs.get(self.active_tab)
    }

    pub fn active_tab_mut(&mut self) -> Option<&mut QueryTab> {
        self.tabs.get_mut(self.active_tab)
    }

    pub fn tab(&self, id: Uuid) -> Option<&QueryTab> {
        self.tabs.iter().find(|t| t.id == id)
    }

    pub fn tab_mut(&mut self, id: Uuid) -> Option<&mut QueryTab> {
        self.tabs.iter_mut().find(|t| t.id == id)
    }

    pub fn tab_index(&self, id: Uuid) -> Option<usize> {
        self.tabs.iter().position(|t| t.id == id)
    }

    pub fn set_active_tab(&mut self, index: usize) {
        if index < self.tabs.len() {
            self.active_tab = index;
            self.completion = None;
            self.workspace_dirty = true;
        }
    }

    pub fn next_tab(&mut self) {
        if self.tabs.len() > 1 {
            self.active_tab = (self.active_tab + 1) % self.tabs.len();
            self.completion = None;
            self.workspace_dirty = true;
        }
    }

    pub fn previous_tab(&mut self) {
        if self.tabs.len() > 1 {
            self.active_tab = (self.active_tab + self.tabs.len() - 1) % self.tabs.len();
            self.completion = None;
            self.workspace_dirty = true;
        }
    }

    pub fn close_tab_force(&mut self, tab_id: Uuid) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        // Cancel any query owned by this tab.
        let request_ids: Vec<u64> = self
            .running
            .iter()
            .filter(|(_, task)| task.tab_id == tab_id)
            .map(|(id, _)| *id)
            .collect();
        for id in request_ids {
            if let Some(task) = self.running.remove(&id) {
                task.abort.abort();
            }
        }
        self.tabs.remove(index);
        if self.active_tab >= self.tabs.len() {
            self.active_tab = self.tabs.len().saturating_sub(1);
        }
        if self.tabs.is_empty() {
            self.push_new_tab();
        }
        self.focus = Focus::Editor;
        self.workspace_dirty = true;
        self.completion = None;
    }

    pub fn request_close_active_tab(&mut self) {
        let Some(tab) = self.active_tab() else {
            return;
        };
        let id = tab.id;
        let title = tab.title.clone();
        let needs_confirm = tab.editor.is_modified() && tab.editor.has_content();
        if needs_confirm {
            self.modal = Some(Modal::Confirm(ConfirmDialog {
                title: "Close query tab?".to_string(),
                message: format!(
                    "“{title}” has unsaved editor content.\nClosing the tab discards it. (The workspace autosave will also forget it.)"
                ),
                action: ConfirmAction::CloseTab(id),
                yes: false,
            }));
        } else {
            self.close_tab_force(id);
        }
    }

    pub fn request_close_tab_by_index(&mut self, index: usize) {
        if index < self.tabs.len() {
            self.active_tab = index;
            self.request_close_active_tab();
        }
    }

    // -----------------------------------------------------------------
    // Logging / toasts
    // -----------------------------------------------------------------

    pub fn log(&mut self, level: LogLevel, message: impl Into<String>) {
        self.push_log(level, message.into(), None, None);
    }

    pub fn log_with_detail(
        &mut self,
        level: LogLevel,
        message: impl Into<String>,
        detail: impl Into<String>,
        source: Option<String>,
    ) {
        self.push_log(level, message.into(), Some(detail.into()), source);
    }

    pub fn log_error(&mut self, summary: impl Into<String>, err: &DbError) {
        self.push_log(
            LogLevel::Error,
            summary.into(),
            Some(err.console_lines().join("\n")),
            None,
        );
        self.toast(LogLevel::Error, err.summary.clone());
    }

    pub fn push_log(
        &mut self,
        level: LogLevel,
        message: String,
        detail: Option<String>,
        source: Option<String>,
    ) {
        self.logs.push(LogEntry {
            at: Local::now(),
            level,
            message,
            detail,
            source,
        });
        if self.logs.len() > MAX_LOGS {
            self.logs.drain(..self.logs.len() - MAX_LOGS);
        }
        if self.focus != Focus::Logs {
            self.console.follow = true;
        }
    }

    pub fn toast(&mut self, level: LogLevel, message: impl Into<String>) {
        self.toast = Some(Toast {
            text: message.into(),
            level,
            until: Instant::now() + TOAST_DURATION,
        });
    }

    pub fn running_count(&self) -> usize {
        self.running.len()
    }

    pub fn current_running(&self) -> bool {
        self.active_tab().and_then(|t| t.running.as_ref()).is_some()
    }

    // -----------------------------------------------------------------
    // Persistence
    // -----------------------------------------------------------------

    pub fn save_profiles_now(&mut self) {
        if let Err(err) = self
            .store
            .save_profiles(&self.profiles, &self.catalog.folders)
        {
            self.log(
                LogLevel::Error,
                format!("Could not save connection profiles: {err}"),
            );
        }
    }

    pub fn save_settings_now(&mut self) {
        if let Err(err) = self.store.save_settings(&self.settings) {
            self.log(LogLevel::Error, format!("Could not save settings: {err}"));
        }
    }

    pub fn save_history_now(&mut self) {
        self.last_history_save = Instant::now();
        match self.store.save_history(&self.history) {
            Ok(()) => {
                self.history_dirty = false;
                self.history_save_failed = false;
            }
            Err(err) => {
                // Keep the dirty flag so the next tick retries, but do not
                // flood the console with the same error every few seconds.
                self.history_dirty = true;
                if !self.history_save_failed {
                    self.history_save_failed = true;
                    self.log(
                        LogLevel::Error,
                        format!("Could not save query history: {err}"),
                    );
                }
            }
        }
    }

    pub fn save_workspace_now(&mut self) {
        let tabs: Vec<SavedTab> = self
            .tabs
            .iter()
            .map(|tab| SavedTab {
                title: tab.title.clone(),
                sql: tab.editor.text(),
                modified: tab.editor.is_modified(),
            })
            .collect();
        let workspace = WorkspaceFile {
            tabs,
            active: self.active_tab,
        };
        self.last_workspace_save = Instant::now();
        match self.store.save_workspace(&workspace) {
            Ok(()) => {
                self.workspace_dirty = false;
                self.workspace_save_failed = false;
            }
            Err(err) => {
                // Keep the workspace dirty so a transient failure (read-only
                // disk, full filesystem, permission change) is retried.
                self.workspace_dirty = true;
                if !self.workspace_save_failed {
                    self.workspace_save_failed = true;
                    self.log(
                        LogLevel::Error,
                        format!("Could not autosave workspace: {err}"),
                    );
                }
            }
        }
    }

    pub fn quit_now(&mut self) {
        // Abort in-flight work; dropping pool connections rolls back open
        // transactions on the server.
        let tasks: Vec<_> = self.running.drain().map(|(_, v)| v).collect();
        for task in tasks {
            task.abort.abort();
        }
        if let Some(tx) = self.tx_pending.take() {
            tx.abort.abort();
        }
        if let Some(connect) = self.connect.take() {
            connect.abort.abort();
        }
        self.save_workspace_now();
        self.save_settings_now();
        self.save_history_now();
        self.should_quit = true;
    }

    pub fn request_quit(&mut self) {
        let risky = !self.running.is_empty() || self.any_transaction_open();
        if risky {
            let message = if self.any_transaction_open() {
                "There are running queries and/or an open transaction.\n\nQuitting closes the session; uncommitted changes will be rolled back by the server."
            } else {
                "One or more queries are still running. Quitting will cancel them."
            };
            self.modal = Some(Modal::Confirm(ConfirmDialog {
                title: "Quit RustQuery?".to_string(),
                message: message.to_string(),
                action: ConfirmAction::Quit,
                yes: false,
            }));
        } else {
            self.quit_now();
        }
    }

    /// Execute the action attached to a confirmation dialog. The caller takes
    /// the modal first, so this never leaves a stale confirmation behind.
    pub fn apply_confirm(&mut self, action: ConfirmAction) {
        match action {
            ConfirmAction::Quit => self.quit_now(),
            ConfirmAction::Disconnect => self.disconnect(),
            ConfirmAction::DeleteProfile(id) => self.delete_profile(id),
            ConfirmAction::DeleteFolder(id) => self.delete_folder(id),
            ConfirmAction::CloseTab(id) => self.close_tab_force(id),
            ConfirmAction::Connect(profile) => self.start_connect(*profile),
        }
    }

    pub fn tick(&mut self) {
        if self
            .toast
            .as_ref()
            .map(|t| Instant::now() >= t.until)
            .unwrap_or(false)
        {
            self.toast = None;
        }
        if self.workspace_dirty && self.last_workspace_save.elapsed() >= WORKSPACE_AUTOSAVE {
            self.save_workspace_now();
        }
        if self.history_dirty && self.last_history_save.elapsed() >= HISTORY_AUTOSAVE {
            self.save_history_now();
        }
    }

    pub fn mark_workspace_dirty(&mut self) {
        self.workspace_dirty = true;
    }

    /// Switch to the workspace and mark a pane as the keyboard focus target.
    pub fn focus_pane(&mut self, focus: Focus) {
        self.view = View::Workspace;
        self.focus = focus;
        self.completion = None;
        if focus == Focus::Logs {
            self.settings.show_logs = true;
        }
    }

    pub fn copy_tree_label(&mut self) {
        let Some(node) = self.tree.selected_node() else {
            return;
        };
        let text = node.label.clone();
        let message = self.clipboard.copy(&text);
        self.toast(LogLevel::Info, format!("Node · {message}"));
    }

    fn any_transaction_open(&self) -> bool {
        self.session
            .as_ref()
            .map(|session| session.tx_active)
            .unwrap_or(false)
            || self.retained.values().any(|held| held.session.tx_active)
    }

    fn owns_session(&self, session_id: u64) -> bool {
        self.session.as_ref().map(|session| session.id) == Some(session_id)
            || self
                .retained
                .values()
                .any(|held| held.session.id == session_id)
    }

    fn set_tx_active(&mut self, session_id: u64, active: bool) {
        if self.session.as_ref().map(|session| session.id) == Some(session_id) {
            if let Some(session) = self.session.as_mut() {
                session.tx_active = active;
            }
            return;
        }
        if let Some(held) = self
            .retained
            .values_mut()
            .find(|held| held.session.id == session_id)
        {
            held.session.tx_active = active;
        }
    }

    fn live_profile_ids(&self) -> HashSet<Uuid> {
        let mut live = HashSet::new();
        if let Some(session) = &self.session {
            live.insert(session.info.profile_id);
        }
        live.extend(self.retained.keys().copied());
        live
    }

    pub fn profile_is_live(&self, profile_id: Uuid) -> bool {
        self.live_profile_ids().contains(&profile_id)
    }

    /// Move the editor's session aside without closing its pool.
    fn park_active(&mut self) {
        let Some(session) = self.session.take() else {
            return;
        };
        let profile_id = session.info.profile_id;
        let tree = std::mem::take(&mut self.tree);
        let schema_loading = self.schema_loading;
        self.schema_loading = false;
        self.retained.insert(
            profile_id,
            RetainedSession {
                session,
                tree,
                schema_loading,
            },
        );
    }

    /// Make an already-open profile the editor target. Pools stay open.
    fn activate_open_profile(&mut self, profile_id: Uuid) -> bool {
        if self.session.as_ref().map(|session| session.info.profile_id) == Some(profile_id) {
            return true;
        }
        let Some(held) = self.retained.remove(&profile_id) else {
            return false;
        };
        self.park_active();
        self.session = Some(held.session);
        self.tree = held.tree;
        self.schema_loading = held.schema_loading;
        true
    }

    fn tree_for(&self, profile_id: Uuid) -> Option<&SchemaTree> {
        if self.session.as_ref().map(|session| session.info.profile_id) == Some(profile_id) {
            return Some(&self.tree);
        }
        self.retained.get(&profile_id).map(|held| &held.tree)
    }

    pub fn schema_rows_for(&self, profile_id: Uuid) -> Option<&[crate::schema_tree::TreeRow]> {
        self.tree_for(profile_id).map(|tree| tree.rows.as_slice())
    }

    pub fn tree_selection_for(&self, profile_id: Uuid) -> Option<String> {
        self.tree_for(profile_id)
            .and_then(|tree| tree.selected_id.clone())
    }

    pub fn schema_loading_for(&self, profile_id: Uuid) -> bool {
        if self.session.as_ref().map(|session| session.info.profile_id) == Some(profile_id) {
            return self.schema_loading;
        }
        self.retained
            .get(&profile_id)
            .map(|held| held.schema_loading)
            .unwrap_or(false)
    }

    // -----------------------------------------------------------------
    // Connections
    // -----------------------------------------------------------------

    pub fn open_dashboard(&mut self) {
        self.completion = None;
        self.view = View::Dashboard;
        self.dashboard.connect_error = None;
        self.ensure_dashboard_selection();
    }

    pub fn ensure_dashboard_selection(&mut self) {
        if self.profiles.is_empty() {
            self.dashboard.selected = 0;
        } else {
            self.dashboard.selected = self.dashboard.selected.min(self.profiles.len() - 1);
        }
    }

    pub fn selected_profile(&self) -> Option<&ConnectionProfile> {
        self.profiles.get(self.dashboard.selected)
    }

    pub fn active_profile_index(&self) -> Option<usize> {
        let id = self
            .session
            .as_ref()
            .map(|session| session.info.profile_id)?;
        self.profiles.iter().position(|profile| profile.id == id)
    }

    pub fn open_connection_picker(&mut self) {
        if self.profiles.is_empty() {
            self.toast(
                LogLevel::Info,
                "No saved connections yet — open the dashboard to create one.",
            );
            self.open_dashboard();
            return;
        }
        let selected = self
            .active_profile_index()
            .unwrap_or(self.dashboard.selected)
            .min(self.profiles.len() - 1);
        self.modal = Some(Modal::ConnectionPicker(ConnectionPicker { selected }));
    }

    /// Connect to a saved profile by list index (workspace switcher or dashboard).
    /// An existing session for another profile stays open.
    pub fn connect_to_profile_at(&mut self, index: usize) {
        if index >= self.profiles.len() {
            return;
        }
        self.dashboard.selected = index;
        let profile_id = self.profiles[index].id;
        self.catalog.expanded_connections.insert(profile_id);
        self.open_endpoint(profile_id);
    }

    /// Connect to the selected dashboard profile, or create one when the list
    /// is empty. Shared by the menu, dashboard buttons and double-click.
    pub fn activate_dashboard(&mut self) {
        if self.connect.is_some() {
            self.toast(LogLevel::Info, "A connection attempt is already running.");
            return;
        }
        let index = self.dashboard.selected;
        if index < self.profiles.len() {
            self.request_connect_or_confirm(index);
        } else {
            self.open_profile_form(None);
        }
    }

    pub fn confirm_delete_selected_profile(&mut self) {
        let Some(profile) = self.selected_profile().cloned() else {
            return;
        };
        self.modal = Some(Modal::Confirm(ConfirmDialog {
            title: "Delete connection?".to_string(),
            message: format!(
                "Delete “{}”?\n\nThis removes the saved profile and its password from disk. The active session (if any) stays open.",
                profile.name
            ),
            action: ConfirmAction::DeleteProfile(profile.id),
            yes: false,
        }));
    }

    /// Select and toggle a schema-tree row from the desktop sidebar.
    pub fn toggle_tree_row(&mut self, index: usize) {
        let Some(row) = self.tree.rows.get(index).cloned() else {
            return;
        };
        self.tree.select_row(index);
        if row.expandable {
            if row.expanded {
                self.tree.collapse_selected();
            } else if let Some(request) = self.tree.expand_selected() {
                self.load_table_details(request);
            }
        } else {
            self.handle_sidebar_activate();
        }
    }

    /// Select a schema-tree row and run its Enter action.
    pub fn activate_tree_row(&mut self, index: usize) {
        self.tree.select_row(index);
        self.handle_sidebar_activate();
    }

    /// Add an editor tab containing `text`, optionally marking it clean.
    pub fn open_text_tab(&mut self, title: impl Into<String>, text: &str, clean: bool) -> Uuid {
        let id = Uuid::new_v4();
        let mut tab = QueryTab::new(id, title.into(), text);
        if clean {
            tab.editor.mark_clean();
        }
        self.tabs.push(tab);
        self.active_tab = self.tabs.len() - 1;
        self.view = View::Workspace;
        self.focus = Focus::Editor;
        self.workspace_dirty = true;
        self.completion = None;
        id
    }

    /// Open a UTF-8 SQL file into a new tab.
    pub fn open_sql_file(&mut self, path: &std::path::Path) -> Result<(), String> {
        let text = std::fs::read_to_string(path)
            .map_err(|err| format!("Could not read {}: {err}", path.display()))?;
        let title = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("SQL file")
            .to_string();
        self.open_text_tab(title, &text, true);
        self.log(LogLevel::Success, format!("Opened {}.", path.display()));
        self.last_directory_hint = path.parent().map(|parent| parent.display().to_string());
        Ok(())
    }

    pub fn start_connect(&mut self, profile: ConnectionProfile) {
        if self.connect.is_some() {
            self.toast(
                LogLevel::Warn,
                "A connection attempt is already in progress.",
            );
            return;
        }
        let attempt = self.next_attempt;
        self.next_attempt += 1;
        self.dashboard.connect_error = None;
        self.view = View::Workspace;
        self.completion = None;
        let target = format!("{} · {}", profile.name, profile.endpoint());
        self.log(LogLevel::Info, format!("Connecting to {target}…"));

        let tx = self.worker_tx.clone();
        let connect_profile = profile.clone();
        let handle = tokio::spawn(async move {
            let result = tokio::time::timeout(
                Duration::from_secs(30),
                db::connect(&connect_profile, attempt),
            )
            .await
            .unwrap_or_else(|_| {
                Err(DbError::new(
                    "Connection attempt timed out after 30 seconds. Check host, port and firewall.",
                ))
            });
            let _ = tx.send(WorkerMessage::Connected { attempt, result });
        });
        self.connect = Some(ConnectState {
            attempt,
            profile,
            abort: handle.abort_handle(),
            started: Instant::now(),
        });
    }

    pub fn request_connect_or_confirm(&mut self, index: usize) {
        self.connect_to_profile_at(index);
    }

    /// Expand a host in the explorer and open its session if it is not already live.
    pub fn open_endpoint(&mut self, profile_id: Uuid) {
        self.view = View::Workspace;
        self.catalog.expanded_connections.insert(profile_id);
        self.catalog.expanded_endpoints.insert(profile_id);
        if self.activate_open_profile(profile_id) {
            self.catalog.expanded_databases.insert(profile_id);
            self.catalog.connect_errors.remove(&profile_id);
            self.focus = Focus::Sidebar;
            if self.tree.roots.is_empty() && !self.schema_loading {
                self.request_schemas();
            }
            return;
        }
        if self.connect.as_ref().map(|connect| connect.profile.id) == Some(profile_id) {
            return;
        }
        if self.connect.is_some() {
            self.toast(LogLevel::Info, "A connection attempt is already running.");
            return;
        }
        let Some(profile) = self
            .profiles
            .iter()
            .find(|profile| profile.id == profile_id)
            .cloned()
        else {
            return;
        };
        self.start_connect(profile);
    }

    pub fn toggle_catalog_row(&mut self, id: &str) {
        match self.catalog.toggle(id) {
            CatalogToggle::Endpoint {
                profile_id,
                expanded: true,
            } => self.open_endpoint(profile_id),
            CatalogToggle::Database {
                profile_id,
                expanded: true,
            } => {
                if self.activate_open_profile(profile_id)
                    && self.tree.roots.is_empty()
                    && !self.schema_loading
                {
                    self.request_schemas();
                }
            }
            _ => {}
        }
    }

    pub fn use_schema_row(&mut self, profile_id: Uuid) -> bool {
        if self.activate_open_profile(profile_id) {
            self.view = View::Workspace;
            true
        } else {
            false
        }
    }

    pub fn cancel_connect(&mut self) {
        if let Some(connect) = self.connect.take() {
            connect.abort.abort();
            self.log(
                LogLevel::Warn,
                format!("Connection to “{}” cancelled.", connect.profile.name),
            );
            self.toast(LogLevel::Warn, "Connection cancelled");
        }
    }

    pub fn test_profile(&mut self, id: Uuid) {
        let Some(profile) = self.profiles.iter().find(|p| p.id == id).cloned() else {
            return;
        };
        if let Err(err) = profile.validate() {
            self.dashboard
                .tests
                .insert(id, TestStatus::Err(err.clone()));
            self.toast(LogLevel::Error, err);
            return;
        }
        self.dashboard
            .tests
            .insert(id, TestStatus::Testing(Instant::now()));
        let tx = self.worker_tx.clone();
        tokio::spawn(async move {
            let result = tokio::time::timeout(
                Duration::from_secs(20),
                db::connect::test_connection(&profile),
            )
            .await
            .unwrap_or_else(|_| Err(DbError::new("Connection test timed out.")));
            let _ = tx.send(WorkerMessage::TestFinished {
                key: TestKey::Profile(id),
                result,
            });
        });
    }

    pub fn delete_profile(&mut self, id: Uuid) {
        let before = self.profiles.len();
        self.profiles.retain(|p| p.id != id);
        if self.profiles.len() == before {
            return;
        }
        self.dashboard.tests.remove(&id);
        self.ensure_dashboard_selection();
        self.save_profiles_now();
        if self
            .session
            .as_ref()
            .map(|s| s.info.profile_id == id)
            .unwrap_or(false)
        {
            self.log(
                LogLevel::Warn,
                "Deleted the profile backing the active session. The session stays open until you disconnect.",
            );
        }
        self.log(LogLevel::Info, "Connection profile deleted.");
    }

    pub fn disconnect(&mut self) {
        let tasks: Vec<_> = self.running.drain().map(|(_, v)| v).collect();
        for task in tasks {
            task.abort.abort();
        }
        if let Some(tx) = self.tx_pending.take() {
            tx.abort.abort();
        }
        if let Some(session) = self.session.take() {
            let had_tx = session.tx_active;
            let profile_id = session.info.profile_id;
            self.catalog.expanded_endpoints.remove(&profile_id);
            self.catalog.expanded_databases.remove(&profile_id);
            self.catalog.connect_errors.remove(&profile_id);
            self.log(
                LogLevel::Info,
                format!("Disconnected from “{}”.", session.profile.name),
            );
            if had_tx {
                self.log(
                    LogLevel::Warn,
                    "The open transaction was rolled back by the server when the session closed.",
                );
            }
        }
        for tab in &mut self.tabs {
            tab.running = None;
            tab.result = None;
            tab.grid = None;
            tab.last_error = None;
        }
        self.tree.clear();
        self.schema_loading = false;
        self.completion = None;
        self.view = View::Workspace;
        self.focus = Focus::Sidebar;
        self.ensure_dashboard_selection();
        self.toast(LogLevel::Info, "Disconnected");
    }

    fn session_switch_needs_confirm(&self) -> bool {
        self.session.as_ref().map(|s| s.tx_active).unwrap_or(false) || !self.running.is_empty()
    }

    pub fn request_disconnect(&mut self) {
        if self.session.is_none() {
            self.toast(LogLevel::Warn, "Not connected.");
            return;
        }
        if self.session_switch_needs_confirm() {
            self.modal = Some(Modal::Confirm(ConfirmDialog {
                title: "Disconnect?".to_string(),
                message:
                    "Running queries will be cancelled and an open transaction will be rolled back."
                        .to_string(),
                action: ConfirmAction::Disconnect,
                yes: false,
            }));
        } else {
            self.disconnect();
        }
    }

    // -----------------------------------------------------------------
    // Schema loading
    // -----------------------------------------------------------------

    fn request_schemas(&mut self) {
        let Some(session) = self.session.as_ref() else {
            return;
        };
        if self.schema_loading {
            return;
        }
        let session_id = session.id;
        let pool = session.pool.clone();
        self.schema_loading = true;
        let tx = self.worker_tx.clone();
        tokio::spawn(async move {
            let result = db::introspect::list_schemas(&pool).await;
            let _ = tx.send(WorkerMessage::SchemasLoaded { session_id, result });
        });
    }

    fn load_table_details(&mut self, request: TableLoadRequest) {
        let Some(session) = self.session.as_ref() else {
            return;
        };
        let session_id = session.id;
        let pool = session.pool.clone();
        let tx = self.worker_tx.clone();
        let node_id = request.node_id.clone();
        let schema = request.schema.clone();
        let table = request.table.clone();
        tokio::spawn(async move {
            let result = db::introspect::table_details(&pool, &schema, &table).await;
            let _ = tx.send(WorkerMessage::TableDetailsLoaded {
                session_id,
                node_id,
                schema,
                table,
                result,
            });
        });
    }

    pub fn refresh_schema(&mut self) {
        if self.session.is_none() {
            self.toast(LogLevel::Warn, "Connect to a database first.");
            return;
        }
        if self.schema_loading {
            self.toast(LogLevel::Info, "Schema discovery is already running.");
            return;
        }
        self.tree.clear();
        self.request_schemas();
        self.toast(LogLevel::Info, "Refreshing schema…");
    }

    pub fn handle_tree_action(&mut self, action: TreeAction) {
        match action {
            TreeAction::Preview {
                schema,
                table,
                object: _,
            } => self.open_preview(&schema, &table),
            TreeAction::InsertColumn { name } => {
                self.view = View::Workspace;
                self.focus = Focus::Editor;
                if let Some(tab) = self.active_tab_mut() {
                    tab.editor.insert_text(&name);
                }
                self.workspace_dirty = true;
                self.completion = None;
            }
            TreeAction::ShowIndex { table, info } => {
                let lines = vec![
                    format!("Table: {table}"),
                    format!("Index: {}", info.name),
                    format!("Columns: {}", info.columns.join(", ")),
                    format!("Method: {}", info.method),
                    format!("Unique: {}", if info.unique { "yes" } else { "no" }),
                    format!("Primary: {}", if info.primary { "yes" } else { "no" }),
                ];
                self.modal = Some(Modal::Info(InfoDialog {
                    title: "Index details".to_string(),
                    lines,
                    scroll: 0,
                }));
            }
            TreeAction::ShowForeignKey { table, info } => {
                let lines = vec![
                    format!("Table: {table}"),
                    format!("Constraint: {}", info.name),
                    format!("Columns: {}", info.columns.join(", ")),
                    format!(
                        "References: {}.{}({})",
                        info.ref_schema,
                        info.ref_table,
                        info.ref_columns.join(", ")
                    ),
                    format!("On delete: {}", info.on_delete),
                ];
                self.modal = Some(Modal::Info(InfoDialog {
                    title: "Foreign key details".to_string(),
                    lines,
                    scroll: 0,
                }));
            }
            TreeAction::Toggled | TreeAction::None => {}
        }
    }

    pub fn handle_sidebar_activate(&mut self) {
        let action = self.tree.activate_selected();
        match action {
            TreeAction::None => {}
            other => self.handle_tree_action(other),
        }
    }

    pub fn handle_sidebar_expand(&mut self) {
        if let Some(request) = self.tree.expand_selected() {
            self.load_table_details(request);
        }
    }

    // -----------------------------------------------------------------
    // Query execution
    // -----------------------------------------------------------------

    pub fn execute_current(&mut self) {
        self.execute_active_with_limit(self.settings.row_limit);
    }

    fn execute_active_with_limit(&mut self, row_limit: usize) {
        let Some(session) = self.session.as_ref() else {
            self.toast(
                LogLevel::Warn,
                "Connect to a database before running a query.",
            );
            self.view = View::Workspace;
            self.focus = Focus::Sidebar;
            return;
        };
        if self.tx_pending.is_some() {
            self.toast(LogLevel::Warn, "A transaction command is still running.");
            return;
        }
        let Some(tab) = self.active_tab() else {
            return;
        };
        if tab.running.is_some() {
            self.toast(
                LogLevel::Warn,
                "This tab already has a running query. Press Ctrl+C to cancel it.",
            );
            return;
        }
        let sql = if tab.editor.has_selection() {
            tab.editor.selected_text()
        } else {
            tab.editor.text()
        };
        if sql.trim().is_empty() {
            self.toast(LogLevel::Warn, "Nothing to execute — the editor is empty.");
            return;
        }
        let tab_id = tab.id;
        let tab_title = tab.title.clone();
        let session_id = session.id;
        let tx_active = session.tx_active;
        let statement_is_tx = db::execute::is_tx_statement(&sql);
        let pool = if tx_active || statement_is_tx {
            session.tx_pool.clone()
        } else {
            session.pool.clone()
        };

        self.next_request += 1;
        let request_id = self.next_request;
        let started = Instant::now();
        if let Some(tab) = self.tab_mut(tab_id) {
            tab.last_error = None;
            tab.running = Some(RunningQuery {
                request_id,
                started,
                sql: sql.clone(),
            });
        }
        self.view = View::Workspace;
        self.completion = None;
        let timeout = Duration::from_secs(self.settings.query_timeout_secs);
        let preview_limit = row_limit;
        let tx = self.worker_tx.clone();
        let task_sql = sql.clone();
        let handle = tokio::spawn(async move {
            let result = db::execute::execute(&pool, &task_sql, preview_limit, timeout).await;
            let _ = tx.send(WorkerMessage::QueryFinished {
                session_id,
                tab_id,
                request_id,
                result,
            });
        });
        self.running.insert(
            request_id,
            RunningTask {
                tab_id,
                abort: handle.abort_handle(),
                started,
                sql: sql.clone(),
            },
        );
        self.log(LogLevel::Info, format!("Executing on “{tab_title}”…"));
    }

    pub fn open_preview(&mut self, schema: &str, table: &str) {
        let Some(session) = self.session.as_ref() else {
            self.toast(LogLevel::Warn, "Connect to a database first.");
            return;
        };
        let driver = session.driver();
        let qualified = match driver {
            Driver::Postgres => format!("{}.{}", quote_pg_ident(schema), quote_pg_ident(table)),
            Driver::MySql => format!("{}.{}", quote_my_ident(schema), quote_my_ident(table)),
            Driver::Sqlite => format!(
                "{}.{}",
                quote_sqlite_ident(schema),
                quote_sqlite_ident(table)
            ),
        };
        let sql = format!("SELECT *\nFROM {qualified}\nLIMIT 100;");
        let title = format!("Preview · {table}");
        let id = Uuid::new_v4();
        self.tabs.push(QueryTab {
            is_preview: true,
            ..QueryTab::new(id, title, &sql)
        });
        self.active_tab = self.tabs.len() - 1;
        self.view = View::Workspace;
        self.focus = Focus::Editor;
        self.workspace_dirty = true;
        if let Some(tab) = self.tab_mut(id) {
            tab.auto_focus_results = true;
            tab.editor.mark_clean();
        }
        self.execute_active_with_limit(100);
        self.log(
            LogLevel::Info,
            format!("Previewing {schema}.{table} (limit 100 rows)."),
        );
    }

    pub fn cancel_current_query(&mut self) {
        let Some(tab) = self.active_tab() else {
            return;
        };
        let Some(running) = tab.running.clone() else {
            return;
        };
        let tab_id = tab.id;
        let tab_title = tab.title.clone();
        if let Some(task) = self.running.remove(&running.request_id) {
            task.abort.abort();
        }
        if let Some(tab) = self.tab_mut(tab_id) {
            tab.running = None;
        }
        self.log(
            LogLevel::Warn,
            format!("Query cancelled by user · “{tab_title}”."),
        );
        self.toast(LogLevel::Warn, "Query cancelled");
        // An interrupted transaction command may leave the server-side state
        // uncertain; we keep the UI's tx flag unchanged and let COMMIT/ROLLBACK
        // produce the definitive answer.
    }

    pub fn copy_selection_or_line(&mut self) {
        let Some(tab) = self.active_tab_mut() else {
            return;
        };
        let text = if tab.editor.has_selection() {
            tab.editor.selected_text()
        } else {
            tab.editor.lines[tab.editor.cursor.row].clone()
        };
        if text.is_empty() {
            return;
        }
        let message = self.clipboard.copy(&text);
        self.toast(LogLevel::Info, message);
    }

    pub fn cut_selection(&mut self) {
        let Some(tab) = self.active_tab() else {
            return;
        };
        if !tab.editor.has_selection() {
            self.toast(LogLevel::Info, "Select text to cut.");
            return;
        }
        let text = tab.editor.selected_text();
        let message = self.clipboard.copy(&text);
        if let Some(tab) = self.active_tab_mut() {
            tab.editor.delete_selection();
        }
        self.workspace_dirty = true;
        self.toast(LogLevel::Info, format!("Cut · {message}"));
    }

    pub fn paste_into_editor(&mut self) {
        let Some(text) = self.clipboard.paste() else {
            self.toast(LogLevel::Warn, "Clipboard is empty.");
            return;
        };
        if let Some(tab) = self.active_tab_mut() {
            tab.editor.insert_text(&text);
        }
        self.workspace_dirty = true;
        self.completion = None;
    }

    /// Build the SQL/schema completion list for the token before the cursor.
    pub fn open_completion(&mut self) {
        let Some(tab) = self.active_tab() else {
            return;
        };
        let (start, prefix) = tab
            .editor
            .word_prefix()
            .unwrap_or((tab.editor.cursor, String::new()));
        let needle = prefix.to_lowercase();
        let mut items: Vec<String> = SQL_COMPLETIONS.iter().map(|s| s.to_string()).collect();
        for row in &self.tree.rows {
            if matches!(
                row.kind,
                RowKind::Schema | RowKind::Table | RowKind::View | RowKind::Column
            ) {
                items.push(row.label.clone());
            }
        }
        items.retain(|item| item.to_lowercase().starts_with(&needle));
        items.sort_by_key(|item| item.to_lowercase());
        items.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
        items.truncate(200);
        if items.is_empty() {
            self.completion = None;
            self.toast(LogLevel::Info, "No completion matches.");
            return;
        }
        self.completion = Some(CompletionState {
            items,
            selected: 0,
            start,
            prefix,
        });
    }

    /// Insert the highlighted completion item into the active editor.
    pub fn accept_completion(&mut self) {
        let Some(completion) = self.completion.take() else {
            return;
        };
        let Some(item) = completion.items.get(completion.selected).cloned() else {
            return;
        };
        if let Some(tab) = self.active_tab_mut() {
            tab.editor
                .replace_prefix_before_cursor(completion.start, &item);
        }
        self.workspace_dirty = true;
    }

    /// Move inside the completion popup. Returns `true` when the key was used.
    pub fn completion_step(&mut self, delta: isize, page: bool) -> bool {
        let Some(completion) = &mut self.completion else {
            return false;
        };
        let step = if page { 8 } else { 1 };
        let max = completion.items.len().saturating_sub(1);
        completion.selected =
            (completion.selected as isize + delta * step).clamp(0, max as isize) as usize;
        true
    }

    /// Editor movement helper shared by the desktop editor and shortcut bar.
    pub fn editor_move(&mut self, motion: Motion, extend: bool) {
        if let Some(tab) = self.active_tab_mut() {
            tab.editor.move_cursor(motion, extend);
        }
    }

    pub fn copy_result_cell(&mut self) {
        let Some(tab) = self.active_tab() else {
            return;
        };
        let (Some(result), Some(grid)) = (&tab.result, &tab.grid) else {
            return;
        };
        let Some(raw_row) = grid.selected_raw_row() else {
            return;
        };
        let cell = result
            .rows
            .get(raw_row)
            .and_then(|r| r.get(grid.sel_col))
            .cloned()
            .flatten();
        let Some(value) = cell else {
            self.toast(LogLevel::Info, "NULL cell copied as empty text");
            let msg = self.clipboard.copy("");
            self.toast(LogLevel::Info, msg);
            return;
        };
        let msg = self.clipboard.copy(&value);
        let preview: String = value.chars().take(40).collect();
        self.toast(
            LogLevel::Info,
            format!(
                "{msg} · {}{}",
                preview.replace(['\n', '\r', '\t'], " "),
                if value.chars().count() > 40 {
                    "…"
                } else {
                    ""
                }
            ),
        );
    }

    pub fn copy_result_row(&mut self) {
        let Some(tab) = self.active_tab() else {
            return;
        };
        let (Some(result), Some(grid)) = (&tab.result, &tab.grid) else {
            return;
        };
        if let Some(text) = grid.row_tsv(result, true) {
            let msg = self.clipboard.copy(&text);
            self.toast(LogLevel::Info, format!("Row · {msg}"));
        }
    }

    pub fn copy_all_results(&mut self) {
        let Some(tab) = self.active_tab() else {
            return;
        };
        let (Some(result), Some(grid)) = (&tab.result, &tab.grid) else {
            return;
        };
        let text = grid.all_tsv(result);
        let msg = self.clipboard.copy(&text);
        self.toast(LogLevel::Info, format!("Visible results · {msg}"));
    }

    // -----------------------------------------------------------------
    // Transactions
    // -----------------------------------------------------------------

    pub fn run_transaction(&mut self, action: TxAction) {
        let Some(session) = self.session.as_ref() else {
            self.toast(LogLevel::Warn, "Connect to a database first.");
            return;
        };
        if let Some(tx) = &self.tx_pending {
            self.toast(
                LogLevel::Warn,
                format!("{} is still running.", tx.action.label()),
            );
            return;
        }
        if action == TxAction::Begin && session.tx_active {
            self.toast(LogLevel::Warn, "A transaction is already open.");
            return;
        }
        if action != TxAction::Begin && !session.tx_active {
            self.toast(
                LogLevel::Warn,
                "No active transaction. Press F8 to begin one.",
            );
            return;
        }
        let session_id = session.id;
        let pool = session.tx_pool.clone();
        let sql = action.sql();
        let tx = self.worker_tx.clone();
        let timeout = Duration::from_secs(self.settings.query_timeout_secs.max(10));
        self.log(LogLevel::Info, format!("{} …", action.label()));
        let handle = tokio::spawn(async move {
            let result = db::execute::execute(&pool, sql, 10, timeout)
                .await
                .map(|_| ());
            let _ = tx.send(WorkerMessage::TransactionFinished {
                session_id,
                action,
                result,
            });
        });
        self.tx_pending = Some(TxPending {
            action,
            abort: handle.abort_handle(),
            started: Instant::now(),
        });
    }

    // -----------------------------------------------------------------
    // Export / save query
    // -----------------------------------------------------------------

    pub fn open_export_dialog(&mut self) {
        let Some(tab) = self.active_tab() else {
            return;
        };
        if tab.result.is_none() || tab.grid.is_none() {
            self.toast(
                LogLevel::Warn,
                "Run a query first; there are no results to export.",
            );
            return;
        }
        let driver = self.session.as_ref().map(|s| s.driver()).unwrap_or(
            tab.editor
                .lines
                .first()
                .map(|_| Driver::Postgres)
                .unwrap_or(Driver::Postgres),
        );
        self.modal = Some(Modal::Export(ExportDialog::new(tab, driver)));
    }

    pub fn start_export(&mut self) {
        let Some(Modal::Export(dialog)) = self.modal.take() else {
            return;
        };
        let tab_id = dialog.tab_id;
        let Some(tab) = self.tab(tab_id) else {
            return;
        };
        let (Some(result), Some(grid)) = (tab.result.clone(), tab.grid.clone()) else {
            return;
        };
        let path_raw = dialog.path.value.trim();
        if path_raw.is_empty() {
            self.modal = Some(Modal::Export(ExportDialog {
                error: Some("Enter an output path.".to_string()),
                ..dialog
            }));
            return;
        }
        let mut path = crate::config::expand_tilde(std::path::Path::new(path_raw));
        path = export::ensure_extension(&path, dialog.format);
        let rows: Vec<usize> = match dialog.scope {
            ExportScope::All => (0..result.rows.len()).collect(),
            ExportScope::Visible => grid.order.clone(),
        };
        let row_count = rows.len();
        if row_count == 0 {
            self.modal = Some(Modal::Export(ExportDialog {
                error: Some("The selected view has no rows to export.".to_string()),
                ..dialog
            }));
            return;
        }
        let format = dialog.format;
        let include_headers = dialog.include_headers;
        let table = dialog.table.value.clone();
        let driver = dialog.driver;
        let path_for_log = path.clone();
        let tx = self.worker_tx.clone();
        let result_for_task = result.clone();
        let path_for_task = path.clone();
        self.log(
            LogLevel::Info,
            format!("Exporting {} rows to {}…", row_count, path.display()),
        );
        tokio::spawn(async move {
            let blocking = tokio::task::spawn_blocking(move || {
                let contents = export::render(
                    &result_for_task,
                    &rows,
                    format,
                    include_headers,
                    &table,
                    driver,
                );
                export::write_export(&path_for_task, &contents).map_err(|err| err.to_string())?;
                let bytes = contents.len() as u64;
                Ok::<(u64, usize), String>((bytes, row_count))
            })
            .await;
            let result = match blocking {
                Ok(Ok((bytes, _))) => Ok(bytes),
                Ok(Err(err)) => Err(err),
                Err(err) => Err(format!("Export task failed: {err}")),
            };
            let _ = tx.send(WorkerMessage::ExportFinished {
                path: path_for_log,
                rows: row_count,
                result,
            });
        });
        self.toast(LogLevel::Info, format!("Exporting {row_count} rows…"));
    }

    pub fn open_save_query_dialog(&mut self) {
        let Some(tab) = self.active_tab() else {
            return;
        };
        if !tab.editor.has_content() {
            self.toast(LogLevel::Warn, "Nothing to save.");
            return;
        }
        let safe_title: String = tab
            .title
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let stamp = Local::now().format("%Y%m%d-%H%M%S");
        let path = self
            .last_directory_hint
            .clone()
            .unwrap_or_else(|| format!("~/rustquery-{safe_title}-{stamp}.sql"));
        self.modal = Some(Modal::Input(InputDialog {
            title: "Save SQL file".to_string(),
            label: "Path".to_string(),
            hint: "~ is expanded. Existing files are overwritten.".to_string(),
            input: TextInput::new(path),
            purpose: InputPurpose::SaveQuery { tab_id: tab.id },
            error: None,
        }));
    }

    pub fn commit_save_query(&mut self) {
        let Some(Modal::Input(dialog)) = self.modal.take() else {
            return;
        };
        let InputPurpose::SaveQuery { tab_id } = dialog.purpose else {
            return;
        };
        let Some(tab) = self.tab(tab_id) else {
            return;
        };
        let raw = dialog.input.value.trim();
        if raw.is_empty() {
            self.modal = Some(Modal::Input(InputDialog {
                error: Some("Enter a path.".to_string()),
                ..dialog
            }));
            return;
        }
        let path = crate::config::expand_tilde(std::path::Path::new(raw));
        match self.store.save_query_file(&path, &tab.editor.text()) {
            Ok(()) => {
                if let Some(tab) = self.tab_mut(tab_id) {
                    tab.editor.mark_clean();
                }
                self.last_directory_hint = Some(path.display().to_string());
                self.workspace_dirty = true;
                self.log(
                    LogLevel::Success,
                    format!("SQL saved to {}.", path.display()),
                );
                self.toast(LogLevel::Success, "SQL file saved");
            }
            Err(err) => {
                self.modal = Some(Modal::Input(InputDialog {
                    error: Some(err.to_string()),
                    ..dialog
                }));
            }
        }
    }

    // -----------------------------------------------------------------
    // Filters
    // -----------------------------------------------------------------

    pub fn open_filter_dialog(&mut self) {
        let Some(tab) = self.active_tab() else {
            return;
        };
        let Some(grid) = tab.grid.as_ref() else {
            self.toast(LogLevel::Warn, "No result set to filter.");
            return;
        };
        let filter = grid.filter.clone();
        self.modal = Some(Modal::Filter(FilterDialog {
            tab_id: tab.id,
            input: TextInput::new(filter.clone()),
            original: filter,
            error: None,
        }));
    }

    pub fn apply_filter_dialog(&mut self) {
        let Some(Modal::Filter(dialog)) = &self.modal else {
            return;
        };
        let tab_id = dialog.tab_id;
        let filter = dialog.input.value.clone();
        if let Some(tab) = self.tab_mut(tab_id) {
            if let (Some(result), Some(grid)) = (&tab.result, &mut tab.grid) {
                grid.set_filter(result, filter);
            }
        }
    }

    pub fn cancel_filter_dialog(&mut self) {
        let Some(Modal::Filter(dialog)) = self.modal.take() else {
            return;
        };
        if let Some(tab) = self.tab_mut(dialog.tab_id) {
            if let (Some(result), Some(grid)) = (&tab.result, &mut tab.grid) {
                grid.set_filter(result, dialog.original);
            }
        }
    }

    // -----------------------------------------------------------------
    // Cell details, history, info
    // -----------------------------------------------------------------

    pub fn open_cell_detail(&mut self) {
        let Some(tab) = self.active_tab() else {
            return;
        };
        let (Some(result), Some(grid)) = (&tab.result, &tab.grid) else {
            return;
        };
        let Some(raw_row) = grid.selected_raw_row() else {
            return;
        };
        let Some(row) = result.rows.get(raw_row) else {
            return;
        };
        let col = grid.sel_col;
        let column = result
            .columns
            .get(col)
            .map(|c| c.name.clone())
            .unwrap_or_default();
        let type_name = result
            .columns
            .get(col)
            .map(|c| c.type_name.clone())
            .unwrap_or_default();
        let value = row.get(col).cloned().flatten();
        self.modal = Some(Modal::Cell(CellDetailDialog {
            title: "Cell details".to_string(),
            column,
            type_name,
            value,
            row_number: raw_row + 1,
            scroll: 0,
        }));
    }

    pub fn open_history(&mut self) {
        self.completion = None;
        self.modal = Some(Modal::History(HistoryBrowser {
            selected: 0,
            scroll: 0,
        }));
    }

    pub fn load_history_entry(&mut self, index: usize) {
        let Some(entry) = self.history.get(index) else {
            return;
        };
        let sql = entry.sql.clone();
        let title = format!(
            "History · {}",
            entry.at.with_timezone(&Local).format("%H:%M:%S")
        );
        self.modal = None;
        let id = Uuid::new_v4();
        self.tabs.push(QueryTab::new(id, title, &sql));
        self.active_tab = self.tabs.len() - 1;
        self.view = View::Workspace;
        self.focus = Focus::Editor;
        self.workspace_dirty = true;
        self.completion = None;
    }

    pub fn open_settings(&mut self) {
        self.completion = None;
        self.modal = Some(Modal::Settings(SettingsDialog {
            selected: 0,
            backup: self.settings.clone(),
        }));
    }

    pub fn open_help(&mut self) {
        self.completion = None;
        self.modal = Some(Modal::Help { scroll: 0 });
    }

    pub fn open_profile_form(&mut self, profile: Option<&ConnectionProfile>) {
        self.next_form_token += 1;
        let token = self.next_form_token;
        self.modal = Some(Modal::Profile(ProfileForm::new(profile, token)));
    }

    pub fn open_profile_form_in_folder(&mut self, folder_id: Option<Uuid>) {
        self.open_profile_form(None);
        if let Some(Modal::Profile(form)) = &mut self.modal {
            form.folder_id = folder_id;
        }
    }

    pub fn prompt_new_folder(&mut self) {
        self.completion = None;
        self.modal = Some(Modal::Input(InputDialog {
            title: "New folder".to_string(),
            label: "Group saved connections in the explorer.".to_string(),
            hint: "Folder name".to_string(),
            input: TextInput::new(""),
            purpose: InputPurpose::CreateFolder,
            error: None,
        }));
    }

    pub fn create_folder(&mut self, name: &str) -> Result<(), String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("Folder name cannot be empty.".into());
        }
        let folder = ConnectionFolder::new(name);
        self.catalog.expanded_folders.insert(folder.id);
        self.catalog.folders.push(folder);
        self.catalog
            .folders
            .sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        self.save_profiles_now();
        self.toast(LogLevel::Success, "Folder created");
        Ok(())
    }

    pub fn prompt_rename_folder(&mut self, id: Uuid) {
        let Some(name) = self
            .catalog
            .folders
            .iter()
            .find(|folder| folder.id == id)
            .map(|folder| folder.name.clone())
        else {
            return;
        };
        self.completion = None;
        self.modal = Some(Modal::Input(InputDialog {
            title: "Rename folder".to_string(),
            label: "Folder name".to_string(),
            hint: "Folder name".to_string(),
            input: TextInput::new(name),
            purpose: InputPurpose::RenameFolder { id },
            error: None,
        }));
    }

    pub fn rename_folder(&mut self, id: Uuid, name: &str) -> Result<(), String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("Folder name cannot be empty.".into());
        }
        let Some(folder) = self
            .catalog
            .folders
            .iter_mut()
            .find(|folder| folder.id == id)
        else {
            return Err("That folder is no longer in the explorer.".into());
        };
        folder.name = name.to_string();
        self.catalog
            .folders
            .sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        self.save_profiles_now();
        self.toast(LogLevel::Success, "Folder renamed");
        Ok(())
    }

    pub fn confirm_delete_folder(&mut self, id: Uuid) {
        let Some(folder) = self.catalog.folders.iter().find(|folder| folder.id == id) else {
            return;
        };
        let name = folder.name.clone();
        self.modal = Some(Modal::Confirm(ConfirmDialog {
            title: "Remove folder?".to_string(),
            message: format!(
                "Remove folder “{name}”?\n\nConnections inside it stay saved and move to the top of the explorer. Open sessions stay connected."
            ),
            action: ConfirmAction::DeleteFolder(id),
            yes: false,
        }));
    }

    pub fn delete_folder(&mut self, id: Uuid) {
        let before = self.catalog.folders.len();
        self.catalog.folders.retain(|folder| folder.id != id);
        if self.catalog.folders.len() == before {
            return;
        }
        self.catalog.expanded_folders.remove(&id);
        for profile in &mut self.profiles {
            if profile.folder_id == Some(id) {
                profile.folder_id = None;
            }
        }
        self.save_profiles_now();
        self.toast(
            LogLevel::Info,
            "Folder removed. Its connections are at the top of the explorer.",
        );
    }

    pub fn move_profile_to_folder(&mut self, profile_id: Uuid, folder_id: Option<Uuid>) {
        if let Some(folder_id) = folder_id {
            if !self
                .catalog
                .folders
                .iter()
                .any(|folder| folder.id == folder_id)
            {
                return;
            }
            self.catalog.expanded_folders.insert(folder_id);
        }
        let Some(profile) = self
            .profiles
            .iter_mut()
            .find(|profile| profile.id == profile_id)
        else {
            return;
        };
        profile.folder_id = folder_id;
        self.catalog.expanded_connections.insert(profile_id);
        self.save_profiles_now();
    }

    pub fn disconnect_profile(&mut self, profile_id: Uuid) {
        if self.session.as_ref().map(|session| session.info.profile_id) == Some(profile_id) {
            self.disconnect();
            return;
        }
        if self.retained.remove(&profile_id).is_some() {
            self.catalog.expanded_endpoints.remove(&profile_id);
            self.catalog.expanded_databases.remove(&profile_id);
            self.catalog.connect_errors.remove(&profile_id);
            let name = self
                .profiles
                .iter()
                .find(|profile| profile.id == profile_id)
                .map(|profile| profile.name.clone())
                .unwrap_or_else(|| "connection".to_string());
            self.log(LogLevel::Info, format!("Disconnected from “{name}”."));
            self.toast(LogLevel::Info, "Disconnected");
        }
    }

    fn session_profile_name(&self, session_id: u64) -> String {
        if let Some(session) = self
            .session
            .as_ref()
            .filter(|session| session.id == session_id)
        {
            return session.info.profile_name.clone();
        }
        self.retained
            .values()
            .find(|held| held.session.id == session_id)
            .map(|held| held.session.info.profile_name.clone())
            .unwrap_or_default()
    }

    pub fn save_profile_form(&mut self) {
        let Some(Modal::Profile(form)) = &self.modal else {
            return;
        };
        let profile = match form.build_profile() {
            Ok(p) => p,
            Err(err) => {
                if let Some(Modal::Profile(form)) = &mut self.modal {
                    form.error = Some(err);
                }
                return;
            }
        };
        let edit_id = form.edit_id;
        self.modal = None;
        self.dashboard.tests.remove(&profile.id);
        if let Some(existing) = self.profiles.iter_mut().find(|p| p.id == profile.id) {
            *existing = profile.clone();
            self.log(
                LogLevel::Info,
                format!("Connection “{}” updated.", profile.name),
            );
        } else {
            self.profiles.push(profile.clone());
            self.log(
                LogLevel::Info,
                format!("Connection “{}” saved.", profile.name),
            );
        }
        self.save_profiles_now();
        self.ensure_dashboard_selection();
        self.catalog.expanded_connections.insert(profile.id);
        if let Some(folder_id) = profile.folder_id {
            self.catalog.expanded_folders.insert(folder_id);
        }
        self.view = View::Workspace;
        self.focus = Focus::Sidebar;
        if let Some(id) = edit_id {
            if let Some(index) = self.profiles.iter().position(|p| p.id == id) {
                self.dashboard.selected = index;
            }
        } else if let Some(index) = self.profiles.iter().position(|p| p.id == profile.id) {
            self.dashboard.selected = index;
        }
        self.toast(LogLevel::Success, "Connection saved");
    }

    pub fn test_profile_form(&mut self) {
        let Some(Modal::Profile(form)) = &self.modal else {
            return;
        };
        let profile = match form.build_profile() {
            Ok(p) => p,
            Err(err) => {
                if let Some(Modal::Profile(form)) = &mut self.modal {
                    form.error = Some(err);
                }
                return;
            }
        };
        let token = form.token;
        if let Some(Modal::Profile(form)) = &mut self.modal {
            form.testing = true;
            form.test_result = None;
            form.error = None;
        }
        let tx = self.worker_tx.clone();
        tokio::spawn(async move {
            let result = tokio::time::timeout(
                Duration::from_secs(20),
                db::connect::test_connection(&profile),
            )
            .await
            .unwrap_or_else(|_| Err(DbError::new("Connection test timed out.")));
            let _ = tx.send(WorkerMessage::TestFinished {
                key: TestKey::Form(token),
                result,
            });
        });
    }

    // -----------------------------------------------------------------
    // Worker events
    // -----------------------------------------------------------------

    pub fn handle_worker(&mut self, message: WorkerMessage) {
        match message {
            WorkerMessage::Connected { attempt, result } => {
                if self.connect.as_ref().map(|c| c.attempt) != Some(attempt) {
                    return;
                }
                let failed_profile = self.connect.as_ref().map(|connect| connect.profile.id);
                self.connect = None;
                match result {
                    Ok(session) => {
                        let profile_id = session.info.profile_id;
                        self.retained.remove(&profile_id);
                        if self.session.as_ref().map(|current| current.info.profile_id)
                            != Some(profile_id)
                        {
                            self.park_active();
                        } else {
                            self.session = None;
                            self.tree.clear();
                            self.schema_loading = false;
                        }
                        self.log(
                            LogLevel::Success,
                            format!(
                                "Connected to “{}” · {} · {} ms · {}",
                                session.info.profile_name,
                                session.info.server_version,
                                session.info.latency_ms,
                                session.info.current_user
                            ),
                        );
                        self.session = Some(session);
                        self.tree.clear();
                        self.schema_loading = false;
                        self.catalog.expanded_connections.insert(profile_id);
                        self.catalog.expanded_endpoints.insert(profile_id);
                        self.catalog.expanded_databases.insert(profile_id);
                        self.catalog.connect_errors.remove(&profile_id);
                        self.view = View::Workspace;
                        self.focus = Focus::Sidebar;
                        self.dashboard.connect_error = None;
                        self.request_schemas();
                        self.toast(LogLevel::Success, "Connected");
                    }
                    Err(err) => {
                        let message = err.console_lines().join("\n");
                        self.dashboard.connect_error = Some(err.summary.clone());
                        if let Some(profile_id) = failed_profile {
                            self.catalog
                                .connect_errors
                                .insert(profile_id, err.summary.clone());
                        }
                        self.log_error("Connection failed", &err);
                        self.push_log(
                            LogLevel::Error,
                            "Connection failed".to_string(),
                            Some(message),
                            Some("connect".to_string()),
                        );
                        self.view = View::Workspace;
                        self.toast(LogLevel::Error, err.summary.clone());
                    }
                }
            }
            WorkerMessage::SchemasLoaded { session_id, result } => {
                if !self.owns_session(session_id) {
                    return;
                }
                let active = self.session.as_ref().map(|session| session.id) == Some(session_id);
                match result {
                    Ok(schemas) => {
                        let object_count: usize =
                            schemas.iter().map(|schema| schema.objects.len()).sum();
                        if active {
                            let Some(session) = self.session.as_ref() else {
                                return;
                            };
                            let root_label = schema_root_label(session);
                            let root_detail = session.info.driver.label();
                            self.schema_loading = false;
                            self.tree.load_schemas(&schemas, &root_label, root_detail);
                        } else if let Some((root_label, root_detail)) =
                            retained_schema_caption(&self.retained, session_id)
                        {
                            if let Some(held) = self
                                .retained
                                .values_mut()
                                .find(|held| held.session.id == session_id)
                            {
                                held.schema_loading = false;
                                held.tree.load_schemas(&schemas, &root_label, root_detail);
                            }
                        }
                        self.log(
                            LogLevel::Info,
                            format!(
                                "Schema loaded · {} schema{} · {} object{}.",
                                schemas.len(),
                                if schemas.len() == 1 { "" } else { "s" },
                                object_count,
                                if object_count == 1 { "" } else { "s" }
                            ),
                        );
                    }
                    Err(err) => {
                        if active {
                            self.schema_loading = false;
                            self.tree.clear();
                        } else if let Some(held) = self
                            .retained
                            .values_mut()
                            .find(|held| held.session.id == session_id)
                        {
                            held.schema_loading = false;
                            held.tree.clear();
                        }
                        self.log_error("Schema discovery failed", &err);
                    }
                }
            }
            WorkerMessage::TableDetailsLoaded {
                session_id,
                node_id,
                schema,
                table,
                result,
            } => {
                if !self.owns_session(session_id) {
                    return;
                }
                let active = self.session.as_ref().map(|session| session.id) == Some(session_id);
                match &result {
                    Ok(details) => {
                        if active {
                            self.tree.complete_table_load(&node_id, Ok(details.clone()));
                        } else if let Some(held) = self
                            .retained
                            .values_mut()
                            .find(|held| held.session.id == session_id)
                        {
                            held.tree.complete_table_load(&node_id, Ok(details.clone()));
                        }
                        self.log(
                            LogLevel::Info,
                            format!(
                                "Loaded metadata for {schema}.{table} · {} columns · {} indexes · {} foreign keys.",
                                details.columns.len(),
                                details.indexes.len(),
                                details.foreign_keys.len()
                            ),
                        );
                    }
                    Err(err) => {
                        if active {
                            self.tree
                                .complete_table_load(&node_id, Err(err.summary.clone()));
                        } else if let Some(held) = self
                            .retained
                            .values_mut()
                            .find(|held| held.session.id == session_id)
                        {
                            held.tree
                                .complete_table_load(&node_id, Err(err.summary.clone()));
                        }
                        self.log_error(
                            format!("Could not load metadata for {schema}.{table}"),
                            err,
                        );
                    }
                }
            }
            WorkerMessage::TestFinished { key, result } => match key {
                TestKey::Profile(id) => {
                    let name = self
                        .profiles
                        .iter()
                        .find(|p| p.id == id)
                        .map(|p| p.name.clone())
                        .unwrap_or_else(|| "profile".to_string());
                    match &result {
                        Ok(outcome) => {
                            self.dashboard
                                .tests
                                .insert(id, TestStatus::Ok(outcome.clone()));
                            self.log(
                                LogLevel::Success,
                                format!(
                                    "Connection test passed for “{name}” · {} ms · {}",
                                    outcome.latency_ms, outcome.server_version
                                ),
                            );
                            self.toast(LogLevel::Success, "Connection test passed");
                        }
                        Err(err) => {
                            self.dashboard
                                .tests
                                .insert(id, TestStatus::Err(err.summary.clone()));
                            self.log_error(format!("Connection test failed for “{name}”"), err);
                        }
                    }
                }
                TestKey::Form(token) => {
                    if let Some(Modal::Profile(form)) = &mut self.modal {
                        if form.token == token {
                            form.testing = false;
                            form.test_result = Some(match &result {
                                Ok(outcome) => (
                                    true,
                                    format!(
                                        "Connected in {} ms · {}",
                                        outcome.latency_ms, outcome.server_version
                                    ),
                                ),
                                Err(err) => (false, err.to_string()),
                            });
                        }
                    }
                }
            },
            WorkerMessage::QueryFinished {
                session_id,
                tab_id,
                request_id,
                result,
            } => {
                self.handle_query_finished(session_id, tab_id, request_id, result);
            }
            WorkerMessage::TransactionFinished {
                session_id,
                action,
                result,
            } => {
                if !self.owns_session(session_id) {
                    return;
                }
                if self
                    .tx_pending
                    .as_ref()
                    .map(|tx| tx.action != action)
                    .unwrap_or(true)
                {
                    return;
                }
                self.tx_pending = None;
                match result {
                    Ok(()) => {
                        self.set_tx_active(session_id, action == TxAction::Begin);
                        self.log(LogLevel::Success, format!("{} successful.", action.label()));
                        self.toast(LogLevel::Success, format!("{} complete", action.label()));
                    }
                    Err(err) => {
                        self.log_error(format!("{} failed", action.label()), &err);
                    }
                }
            }
            WorkerMessage::ExportFinished { path, rows, result } => match result {
                Ok(bytes) => {
                    let message = format!(
                        "Exported {} rows to {} ({} KB).",
                        rows,
                        path.display(),
                        bytes / 1024
                    );
                    self.log(LogLevel::Success, message.clone());
                    self.toast(LogLevel::Success, format!("Export complete · {rows} rows"));
                }
                Err(err) => {
                    let message = format!("Export to {} failed: {err}", path.display());
                    self.log(LogLevel::Error, message.clone());
                    self.toast(LogLevel::Error, "Export failed");
                }
            },
        }
    }

    fn handle_query_finished(
        &mut self,
        session_id: u64,
        tab_id: Uuid,
        request_id: u64,
        result: Result<QueryResult, DbError>,
    ) {
        self.running.remove(&request_id);
        if !self.owns_session(session_id) {
            return;
        }

        // Validate the request and release the tab borrow before logging or
        // touching session state. Keeping the borrow alive would prevent
        // calling any &mut self helper (the borrow checker cannot split
        // through `Option`).
        let (tab_title, auto_focus, elapsed_ms, running_sql) = {
            let Some(tab) = self.tab_mut(tab_id) else {
                return;
            };
            if tab.running.as_ref().map(|r| r.request_id) != Some(request_id) {
                return;
            }
            let Some(running) = tab.running.take() else {
                return;
            };
            (
                tab.title.clone(),
                tab.auto_focus_results,
                running.started.elapsed().as_millis() as u64,
                running.sql,
            )
        };
        let should_focus_results = auto_focus && self.active_tab().map(|t| t.id) == Some(tab_id);
        if let Some(tab) = self.tab_mut(tab_id) {
            tab.auto_focus_results = false;
        }

        match result {
            Ok(query) => {
                if let Some(new_state) = db::execute::tx_state_after(&running_sql) {
                    self.set_tx_active(session_id, new_state);
                }
                let rows_returned = query.rows.len();
                let affected = query.affected;
                let truncated = query.truncated;
                let duration = query.duration_ms;
                let has_columns = !query.columns.is_empty();
                let result_sets = query.result_sets;
                let connection = self.session_profile_name(session_id);

                if let Some(tab) = self.tab_mut(tab_id) {
                    tab.last_error = None;
                    tab.grid = if has_columns {
                        Some(ResultGrid::new(&query))
                    } else {
                        None
                    };
                    tab.result = Some(query);
                }

                let summary = if has_columns {
                    format!(
                        "Query executed successfully in {duration} ms · {} row{} returned{} · tab “{tab_title}”",
                        format_count(rows_returned as u64),
                        if rows_returned == 1 { "" } else { "s" },
                        if truncated { " (row limit reached)" } else { "" }
                    )
                } else {
                    format!(
                        "Statement executed successfully in {duration} ms · {} row{} affected · tab “{tab_title}”",
                        format_count(affected),
                        if affected == 1 { "" } else { "s" }
                    )
                };
                let detail = if result_sets > 1 {
                    Some(format!(
                        "The script produced {result_sets} result sets; the grid shows the first."
                    ))
                } else {
                    None
                };
                self.push_log(
                    LogLevel::Success,
                    summary,
                    detail,
                    Some("query".to_string()),
                );
                self.add_history(HistoryEntry {
                    id: Uuid::new_v4(),
                    at: Utc::now(),
                    connection,
                    sql: running_sql,
                    duration_ms: duration,
                    rows: if has_columns {
                        rows_returned as u64
                    } else {
                        affected
                    },
                    success: true,
                    error: None,
                });
                if truncated {
                    self.toast(
                        LogLevel::Warn,
                        format!("Row limit reached ({rows_returned} rows shown)."),
                    );
                }
                if should_focus_results {
                    self.focus = Focus::Results;
                }
            }
            Err(err) => {
                let connection = self.session_profile_name(session_id);
                if let Some(tab) = self.tab_mut(tab_id) {
                    tab.last_error = Some(err.clone());
                }
                let detail = err.console_lines().join("\n");
                self.push_log(
                    LogLevel::Error,
                    format!("Query failed in {elapsed_ms} ms · tab “{tab_title}”"),
                    Some(detail),
                    Some("query".to_string()),
                );
                self.toast(LogLevel::Error, err.summary.clone());
                self.add_history(HistoryEntry {
                    id: Uuid::new_v4(),
                    at: Utc::now(),
                    connection,
                    sql: running_sql,
                    duration_ms: elapsed_ms,
                    rows: 0,
                    success: false,
                    error: Some(err.to_string()),
                });
            }
        }
        self.workspace_dirty = true;
    }

    fn add_history(&mut self, entry: HistoryEntry) {
        self.history.insert(0, entry);
        if self.history.len() > 500 {
            self.history.truncate(500);
        }
        // History is written on the next tick (or on quit) so a query never
        // blocks the UI thread on a synchronous JSON write.
        self.history_dirty = true;
    }

    // -----------------------------------------------------------------
    // Theme / settings
    // -----------------------------------------------------------------

    pub fn cycle_theme(&mut self) {
        self.settings.theme = self.settings.theme.next();
        self.save_settings_now();
        self.toast(
            LogLevel::Info,
            format!("Theme · {}", self.settings.theme.label()),
        );
    }

    pub fn theme_kind(&self) -> ThemeKind {
        self.settings.theme
    }

    pub fn quit_requested(&self) -> bool {
        self.should_quit
    }

    // -----------------------------------------------------------------
    // Completeness helpers used by rendering
    // -----------------------------------------------------------------

    pub fn status_summary(&self) -> String {
        match &self.session {
            Some(session) => format!(
                "{} · {} · {} ms",
                session.info.profile_name, session.info.current_database, session.info.latency_ms
            ),
            None => "Not connected".to_string(),
        }
    }

    pub fn profile_is_connected(&self, id: Uuid) -> bool {
        self.session
            .as_ref()
            .map(|s| s.info.profile_id == id)
            .unwrap_or(false)
    }

    pub fn results_visible(&self) -> bool {
        self.tabs
            .get(self.active_tab)
            .map(|t| t.result.is_some())
            .unwrap_or(false)
    }
}

const SQL_COMPLETIONS: &[&str] = &[
    "SELECT",
    "FROM",
    "WHERE",
    "JOIN",
    "LEFT JOIN",
    "RIGHT JOIN",
    "INNER JOIN",
    "FULL OUTER JOIN",
    "CROSS JOIN",
    "ON",
    "GROUP BY",
    "ORDER BY",
    "HAVING",
    "LIMIT",
    "OFFSET",
    "INSERT INTO",
    "VALUES",
    "UPDATE",
    "SET",
    "DELETE FROM",
    "RETURNING",
    "WITH",
    "AS",
    "DISTINCT",
    "COUNT",
    "SUM",
    "AVG",
    "MIN",
    "MAX",
    "COALESCE",
    "NULLIF",
    "CAST",
    "CASE",
    "WHEN",
    "THEN",
    "ELSE",
    "END",
    "UNION",
    "UNION ALL",
    "EXCEPT",
    "INTERSECT",
    "BEGIN",
    "COMMIT",
    "ROLLBACK",
    "SAVEPOINT",
    "CREATE TABLE",
    "ALTER TABLE",
    "DROP TABLE",
    "CREATE INDEX",
    "EXPLAIN",
    "EXPLAIN ANALYZE",
    "ANALYZE",
    "VACUUM",
    "TRUNCATE",
    "NOW",
    "CURRENT_TIMESTAMP",
    "CURRENT_DATE",
    "TRUE",
    "FALSE",
    "NULL",
    "AND",
    "OR",
    "NOT",
    "IN",
    "EXISTS",
    "BETWEEN",
    "LIKE",
    "ILIKE",
    "IS NULL",
    "IS NOT NULL",
    "ASC",
    "DESC",
    "PRIMARY KEY",
    "FOREIGN KEY",
    "REFERENCES",
    "DEFAULT",
    "SERIAL",
    "BIGSERIAL",
    "INTEGER",
    "BIGINT",
    "SMALLINT",
    "TEXT",
    "VARCHAR",
    "BOOLEAN",
    "TIMESTAMPTZ",
    "DATE",
    "NUMERIC",
    "JSONB",
    "UUID",
];

fn schema_root_label(session: &Session) -> String {
    match session.driver() {
        Driver::Postgres => session.info.current_database.clone(),
        Driver::MySql | Driver::Sqlite => "Databases".to_string(),
    }
}

fn retained_schema_caption(
    retained: &HashMap<Uuid, RetainedSession>,
    session_id: u64,
) -> Option<(String, &'static str)> {
    let held = retained
        .values()
        .find(|held| held.session.id == session_id)?;
    Some((
        schema_root_label(&held.session),
        held.session.info.driver.label(),
    ))
}

fn quote_pg_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

fn quote_my_ident(name: &str) -> String {
    format!("`{}`", name.replace('`', "``"))
}

fn quote_sqlite_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn destructive_confirmations_are_classified() {
        assert!(ConfirmAction::Quit.is_destructive());
        assert!(ConfirmAction::Disconnect.is_destructive());
        assert!(ConfirmAction::DeleteProfile(Uuid::nil()).is_destructive());
        assert!(ConfirmAction::DeleteFolder(Uuid::nil()).is_destructive());
        assert!(ConfirmAction::CloseTab(Uuid::nil()).is_destructive());
        assert!(
            !ConfirmAction::Connect(Box::new(ConnectionProfile::new(Driver::Sqlite)))
                .is_destructive()
        );
    }

    #[test]
    fn profile_form_cycles_drivers_and_hides_network_fields() {
        let mut form = ProfileForm::new(None, 1);
        assert_eq!(form.driver, Driver::Postgres);

        form.cycle_driver(true);
        assert_eq!(form.driver, Driver::MySql);
        assert_eq!(form.port.value, "3306");
        assert_eq!(form.user.value, "root");
        assert_eq!(form.database.value, "mysql");

        form.cycle_driver(true);
        assert_eq!(form.driver, Driver::Sqlite);
        assert_eq!(form.database.value, "rustquery.db");
        assert!(form.is_field_visible(ProfileForm::DATABASE));
        assert!(!form.is_field_visible(ProfileForm::HOST));
        assert!(!form.is_field_visible(ProfileForm::SSL));
        assert!(form.field_mut(ProfileForm::HOST).is_none());

        form.focus = ProfileForm::DATABASE;
        form.next_focus(false);
        assert_eq!(form.focus, ProfileForm::DRIVER);

        let profile = form.build_profile().expect("valid SQLite profile");
        assert_eq!(profile.driver, Driver::Sqlite);
        assert_eq!(profile.port, 0);
        assert!(profile.user.is_empty());
        assert!(profile.host.is_empty());

        // Switching back to a network engine restores useful defaults rather
        // than keeping SQLite's hidden stale values.
        form.cycle_driver(true);
        assert_eq!(form.driver, Driver::Postgres);
        assert_eq!(form.port.value, "5432");
        assert_eq!(form.user.value, "postgres");
        assert_eq!(form.database.value, "postgres");
    }
}
