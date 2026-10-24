//! Structured database errors with enough context for a useful error console.

use std::fmt;

/// A database failure enriched with SQLSTATE, hints and (when available) the
/// line/column inside the submitted SQL text.
#[derive(Debug, Clone, Default)]
pub struct DbError {
    pub summary: String,
    pub code: Option<String>,
    pub detail: Option<String>,
    pub hint: Option<String>,
    pub line: Option<usize>,
    pub column: Option<usize>,
}

impl DbError {
    pub fn new(summary: impl Into<String>) -> Self {
        Self {
            summary: summary.into(),
            ..Self::default()
        }
    }

    pub fn from_sqlx(error: sqlx::Error, sql: &str) -> Self {
        let mut out = match &error {
            sqlx::Error::Database(db) => {
                let mut e = DbError {
                    summary: db.message().to_string(),
                    code: db.code().map(|c| c.into_owned()),
                    ..DbError::default()
                };

                // PostgreSQL exposes rich diagnostics that are worth surfacing.
                if let Some(pg) = db.try_downcast_ref::<sqlx::postgres::PgDatabaseError>() {
                    e.detail = pg.detail().map(str::to_string);
                    e.hint = pg.hint().map(str::to_string);
                    e.code = Some(pg.code().to_string());
                    if let Some(pos) = pg.position() {
                        use sqlx::postgres::PgErrorPosition;
                        let char_pos = match pos {
                            PgErrorPosition::Original(p) => p,
                            PgErrorPosition::Internal { position, .. } => position,
                        };
                        let (line, column) = line_col_for_offset(sql, char_pos);
                        e.line = Some(line);
                        e.column = Some(column);
                    }
                    if e.summary.trim().is_empty() {
                        e.summary = format!("PostgreSQL {:?} error", pg.severity());
                    }
                }

                // MySQL exposes a numeric error code. Keep SQLSTATE too.
                if let Some(my) = db.try_downcast_ref::<sqlx::mysql::MySqlDatabaseError>() {
                    e.code = Some(format!(
                        "{} / {}",
                        my.number(),
                        my.code().unwrap_or("HY000")
                    ));
                    parse_mysql_line(&mut e);
                }
                e
            }
            sqlx::Error::PoolTimedOut => {
                DbError::new("Timed out while waiting for a database connection from the pool.")
            }
            sqlx::Error::PoolClosed => {
                DbError::new("The connection pool has been closed. Reconnect to continue.")
            }
            sqlx::Error::Io(err) => DbError::new(format!(
                "Network error: {err}. Check that the server is reachable."
            )),
            sqlx::Error::Tls(err) => DbError::new(format!(
                "TLS negotiation failed: {err}. Try a different SSL mode."
            )),
            sqlx::Error::ColumnNotFound(name) => {
                DbError::new(format!("Column not found in result metadata: {name}"))
            }
            other => DbError::new(other.to_string()),
        };

        if out.summary.trim().is_empty() {
            out.summary = format!("{error}");
        }
        out.detail = out.detail.filter(|d| !d.trim().is_empty());
        out.hint = out.hint.filter(|h| !h.trim().is_empty());
        out
    }

    /// Build one-line and indented detail lines for the console.
    pub fn console_lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        let code = self
            .code
            .as_ref()
            .map(|c| format!(" [{c}]"))
            .unwrap_or_default();
        let loc = match (self.line, self.column) {
            (Some(line), Some(col)) => format!(" at line {line}, column {col}"),
            (Some(line), None) => format!(" at line {line}"),
            _ => String::new(),
        };
        lines.push(format!("{}{loc}{code}", self.summary));
        if let Some(detail) = &self.detail {
            for l in detail.lines() {
                lines.push(format!("  detail: {l}"));
            }
        }
        if let Some(hint) = &self.hint {
            for l in hint.lines() {
                lines.push(format!("  hint: {l}"));
            }
        }
        lines
    }
}

impl fmt::Display for DbError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.summary)?;
        if let Some(line) = self.line {
            write!(f, " (line {line}")?;
            if let Some(col) = self.column {
                write!(f, ", column {col}")?;
            }
            write!(f, ")")?;
        }
        if let Some(code) = &self.code {
            write!(f, " [{code}]")?;
        }
        Ok(())
    }
}

impl std::error::Error for DbError {}

/// Compute a 1-based line/column from a character offset in the SQL string.
fn line_col_for_offset(sql: &str, pos: usize) -> (usize, usize) {
    let mut line = 1usize;
    let mut col = 1usize;
    for (idx, ch) in sql.chars().enumerate() {
        if idx + 1 >= pos {
            break;
        }
        if ch == '\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
    }
    (line, col)
}

/// MySQL's syntax errors contain "at line N"; extract it when present.
fn parse_mysql_line(err: &mut DbError) {
    let message = err.summary.clone();
    if let Some(idx) = message.rfind("at line ") {
        let rest = &message[idx + "at line ".len()..];
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(line) = digits.parse::<usize>() {
            err.line = Some(line);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_column_lookup() {
        let sql = "SELECT 1;\nSELECT * FRM x;";
        let pos = sql.find("FRM").unwrap() + 1;
        assert_eq!(line_col_for_offset(sql, pos), (2, 10));
    }
}
