//! Shared domain types.
//!
//! Everything in this module is deliberately free of database-driver details so
//! it can be used by the UI, persistence layer and worker tasks alike.

use std::fmt;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Supported database engines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Driver {
    #[default]
    Postgres,
    MySql,
    Sqlite,
}

impl Driver {
    pub const ALL: [Driver; 3] = [Driver::Postgres, Driver::MySql, Driver::Sqlite];

    pub fn label(self) -> &'static str {
        match self {
            Driver::Postgres => "PostgreSQL",
            Driver::MySql => "MySQL",
            Driver::Sqlite => "SQLite",
        }
    }

    pub fn short(self) -> &'static str {
        match self {
            Driver::Postgres => "PG",
            Driver::MySql => "MY",
            Driver::Sqlite => "SQ",
        }
    }

    /// TCP port for client/server engines. SQLite has no network port.
    pub fn default_port(self) -> u16 {
        match self {
            Driver::Postgres => 5432,
            Driver::MySql => 3306,
            Driver::Sqlite => 0,
        }
    }

    pub fn default_user(self) -> &'static str {
        match self {
            Driver::Postgres => "postgres",
            Driver::MySql => "root",
            Driver::Sqlite => "",
        }
    }

    pub fn default_database(self) -> &'static str {
        match self {
            Driver::Postgres => "postgres",
            Driver::MySql => "mysql",
            Driver::Sqlite => "rustquery.db",
        }
    }

    /// True for engines that connect over TCP and use authentication/TLS.
    pub fn is_networked(self) -> bool {
        !matches!(self, Driver::Sqlite)
    }

    pub fn next(self) -> Self {
        let idx = Self::ALL.iter().position(|d| *d == self).unwrap_or(0);
        Self::ALL[(idx + 1) % Self::ALL.len()]
    }

    pub fn previous(self) -> Self {
        let idx = Self::ALL.iter().position(|d| *d == self).unwrap_or(0);
        Self::ALL[(idx + Self::ALL.len() - 1) % Self::ALL.len()]
    }
}

/// TLS behaviour used when establishing a connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum SslMode {
    /// Never use TLS.
    Disable,
    /// Try TLS and silently fall back to plaintext.
    #[default]
    Prefer,
    /// Require TLS, but do not verify the server certificate.
    Require,
    /// Require TLS and verify the certificate authority.
    VerifyCa,
    /// Require TLS, verify the CA and the hostname.
    VerifyFull,
}

impl SslMode {
    pub const ALL: [SslMode; 5] = [
        SslMode::Disable,
        SslMode::Prefer,
        SslMode::Require,
        SslMode::VerifyCa,
        SslMode::VerifyFull,
    ];

    pub fn label(self) -> &'static str {
        match self {
            SslMode::Disable => "disable",
            SslMode::Prefer => "prefer",
            SslMode::Require => "require",
            SslMode::VerifyCa => "verify-ca",
            SslMode::VerifyFull => "verify-full",
        }
    }

    pub fn next(self) -> Self {
        let idx = Self::ALL.iter().position(|m| *m == self).unwrap_or(0);
        Self::ALL[(idx + 1) % Self::ALL.len()]
    }

    pub fn previous(self) -> Self {
        let idx = Self::ALL.iter().position(|m| *m == self).unwrap_or(0);
        Self::ALL[(idx + Self::ALL.len() - 1) % Self::ALL.len()]
    }
}

impl fmt::Display for SslMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// A persisted connection profile.
///
/// Passwords are stored locally in the configuration directory. Protect the
/// directory with filesystem permissions if the machine is shared — RustQuery
/// creates the file with mode `0600` on Unix.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionProfile {
    #[serde(default = "Uuid::new_v4")]
    pub id: Uuid,
    pub name: String,
    pub driver: Driver,
    pub host: String,
    pub port: u16,
    pub database: String,
    pub user: String,
    #[serde(default)]
    pub password: String,
    #[serde(default)]
    pub ssl_mode: SslMode,
}

impl ConnectionProfile {
    pub fn new(driver: Driver) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: String::new(),
            driver,
            host: if driver.is_networked() {
                "localhost".to_string()
            } else {
                String::new()
            },
            port: driver.default_port(),
            database: String::new(),
            user: driver.default_user().to_string(),
            password: String::new(),
            ssl_mode: if driver == Driver::Sqlite {
                SslMode::Disable
            } else {
                SslMode::default()
            },
        }
    }

    /// Populate friendly defaults for a brand-new profile.
    pub fn with_defaults(mut self) -> Self {
        self.name = format!("Local {}", self.driver.label());
        self.database = self.driver.default_database().to_string();
        self
    }

    pub fn endpoint(&self) -> String {
        if self.driver == Driver::Sqlite {
            if self.database == ":memory:" {
                "in-memory database".to_string()
            } else {
                self.database.clone()
            }
        } else if self.database.is_empty() {
            format!("{}:{}", self.host, self.port)
        } else {
            format!("{}:{}/{}", self.host, self.port, self.database)
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() {
            return Err("Connection name cannot be empty.".into());
        }
        if self.driver == Driver::Sqlite {
            if self.database.trim().is_empty() {
                return Err("SQLite database file cannot be empty.".into());
            }
            if self.database.contains('\0') {
                return Err("SQLite database path cannot contain NUL bytes.".into());
            }
            if self.database != ":memory:"
                && !self.database.starts_with("sqlite:")
                && std::path::Path::new(&self.database).is_dir()
            {
                return Err("SQLite database path points to a directory.".into());
            }
            return Ok(());
        }
        if self.host.trim().is_empty() {
            return Err("Host cannot be empty.".into());
        }
        if self.port == 0 {
            return Err("Port must be between 1 and 65535.".into());
        }
        if self.user.trim().is_empty() {
            return Err("User cannot be empty.".into());
        }
        if self.database.trim().is_empty() {
            return Err("Database cannot be empty.".into());
        }
        Ok(())
    }

    /// A safe one-line description that never includes the password.
    pub fn describe(&self) -> String {
        if self.driver == Driver::Sqlite {
            return format!("{} · {}", self.driver.label(), self.endpoint());
        }
        format!(
            "{} · {} · {}@{}",
            self.driver.label(),
            self.endpoint(),
            self.user,
            self.ssl_mode.label()
        )
    }
}

/// The outcome of a test connection attempt.
#[derive(Debug, Clone)]
pub struct TestOutcome {
    pub latency_ms: u64,
    pub server_version: String,
    pub user: String,
}

/// Runtime metadata about an open database session.
#[derive(Debug, Clone)]
pub struct SessionInfo {
    pub profile_id: Uuid,
    pub profile_name: String,
    pub driver: Driver,
    pub server_version: String,
    pub current_user: String,
    pub current_database: String,
    pub latency_ms: u64,
}

/// Metadata for one column returned by a query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnMeta {
    pub name: String,
    pub type_name: String,
}

impl ColumnMeta {
    pub fn is_numeric(&self) -> bool {
        let t = self.type_name.to_ascii_uppercase();
        [
            "INT", "SERIAL", "FLOAT", "DOUBLE", "REAL", "NUMERIC", "DECIMAL", "MONEY", "OID",
        ]
        .iter()
        .any(|needle| t.contains(needle))
    }

    pub fn is_boolean(&self) -> bool {
        let t = self.type_name.to_ascii_uppercase();
        t == "BOOL" || t == "BOOLEAN"
    }
}

/// A fully materialised result set. Cell values are already formatted for
/// display; `None` means SQL `NULL`.
#[derive(Debug, Clone)]
pub struct QueryResult {
    pub columns: Vec<ColumnMeta>,
    pub rows: Vec<Vec<Option<String>>>,
    /// Rows affected by DML/DDL statements in the executed script.
    pub affected: u64,
    pub duration_ms: u64,
    /// True when the configured row limit was reached (more rows may exist).
    pub truncated: bool,
    /// The exact SQL that produced this result.
    pub sql: String,
    /// Number of result-producing statements observed (useful for scripts).
    pub result_sets: usize,
}

impl QueryResult {
    pub fn is_empty_columns(&self) -> bool {
        self.columns.is_empty()
    }

    /// Format a cell for the clipboard (NULL becomes an empty string).
    pub fn cell_text(&self, row: usize, col: usize) -> String {
        self.rows
            .get(row)
            .and_then(|r| r.get(col))
            .and_then(|v| v.clone())
            .unwrap_or_default()
    }
}

/// Database objects shown in the schema explorer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectKind {
    Table,
    View,
}

impl ObjectKind {
    pub fn is_view(self) -> bool {
        matches!(self, ObjectKind::View)
    }
}

#[derive(Debug, Clone)]
pub struct TableEntry {
    pub schema: String,
    pub name: String,
    pub kind: ObjectKind,
}

#[derive(Debug, Clone)]
pub struct SchemaEntry {
    pub name: String,
    pub objects: Vec<TableEntry>,
}

#[derive(Debug, Clone)]
pub struct ColumnInfo {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
    pub default: Option<String>,
    /// "PRI", "UNI", "MUL" on MySQL; "PK" for PostgreSQL and SQLite keys.
    pub key: String,
    pub ordinal: i64,
}

#[derive(Debug, Clone)]
pub struct IndexInfo {
    pub name: String,
    pub unique: bool,
    pub primary: bool,
    pub columns: Vec<String>,
    pub method: String,
}

#[derive(Debug, Clone)]
pub struct ForeignKeyInfo {
    pub name: String,
    pub columns: Vec<String>,
    pub ref_schema: String,
    pub ref_table: String,
    pub ref_columns: Vec<String>,
    pub on_delete: String,
}

#[derive(Debug, Clone, Default)]
pub struct TableDetails {
    pub columns: Vec<ColumnInfo>,
    pub indexes: Vec<IndexInfo>,
    pub foreign_keys: Vec<ForeignKeyInfo>,
}

/// A persisted query execution record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub id: Uuid,
    pub at: DateTime<Utc>,
    pub connection: String,
    pub sql: String,
    pub duration_ms: u64,
    pub rows: u64,
    pub success: bool,
    pub error: Option<String>,
}

impl HistoryEntry {
    pub fn snippet(&self, max: usize) -> String {
        let compact = self.sql.split_whitespace().collect::<Vec<_>>().join(" ");
        if compact.chars().count() > max {
            let prefix: String = compact.chars().take(max.saturating_sub(1)).collect();
            format!("{prefix}…")
        } else {
            compact
        }
    }
}

/// Persisted editor tab (used to restore the workspace between runs).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedTab {
    pub title: String,
    pub sql: String,
    #[serde(default)]
    pub modified: bool,
}

/// Format an integer with thousands separators for result counts and sizes.
pub fn format_count(n: u64) -> String {
    // Thousands separators make wide result sets much easier to scan.
    let s = n.to_string();
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_validation() {
        let mut p = ConnectionProfile::new(Driver::Postgres).with_defaults();
        assert!(p.validate().is_ok());
        p.name.clear();
        assert!(p.validate().is_err());
    }

    #[test]
    fn thousands_separator() {
        assert_eq!(format_count(0), "0");
        assert_eq!(format_count(999), "999");
        assert_eq!(format_count(1_000), "1,000");
        assert_eq!(format_count(1_250_000), "1,250,000");
    }

    #[test]
    fn ssl_cycles() {
        let mut m = SslMode::Disable;
        for _ in 0..SslMode::ALL.len() {
            m = m.next();
        }
        assert_eq!(m, SslMode::Disable);
    }

    #[test]
    fn driver_cycles_both_ways() {
        let mut d = Driver::Postgres;
        for _ in 0..Driver::ALL.len() {
            d = d.next();
        }
        assert_eq!(d, Driver::Postgres);
        assert_eq!(Driver::Postgres.previous(), Driver::Sqlite);
        assert_eq!(Driver::Sqlite.next(), Driver::Postgres);
        assert!(!Driver::Sqlite.is_networked());
        assert!(Driver::MySql.is_networked());
        assert_eq!(Driver::Sqlite.default_port(), 0);
    }

    #[test]
    fn sqlite_profile_validation() {
        let mut p = ConnectionProfile::new(Driver::Sqlite).with_defaults();
        assert_eq!(p.database, "rustquery.db");
        assert_eq!(p.port, 0);
        assert!(p.host.is_empty());
        assert_eq!(p.endpoint(), "rustquery.db");
        assert!(p.validate().is_ok());
        assert!(!p.describe().to_ascii_lowercase().contains("ssl"));

        p.database = ":memory:".to_string();
        assert_eq!(p.endpoint(), "in-memory database");
        assert!(p.validate().is_ok());

        p.database.clear();
        assert!(p.validate().is_err());
    }
}

/// Severity used by the in-app execution console.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Info,
    Success,
    Warn,
    Error,
}

impl LogLevel {
    pub fn label(self) -> &'static str {
        match self {
            LogLevel::Info => "INFO",
            LogLevel::Success => "OK",
            LogLevel::Warn => "WARN",
            LogLevel::Error => "ERROR",
        }
    }
}

/// One console entry. `detail` may contain multiple lines (database hints,
/// constraint violations, schema-load failures, …).
#[derive(Debug, Clone)]
pub struct LogEntry {
    pub at: DateTime<chrono::Local>,
    pub level: LogLevel,
    pub message: String,
    pub detail: Option<String>,
    pub source: Option<String>,
}
