//! Query execution for arbitrary SQL text.
//!
//! `raw_sql` uses the simple query protocol, which allows multi-statement
//! scripts (BEGIN; UPDATE …; COMMIT;). Rows are streamed and collection stops
//! after the configured row limit, so a careless `SELECT *` cannot exhaust
//! memory or freeze the UI.

use std::time::{Duration, Instant};

use futures_util::TryStreamExt;
use sqlx::{Column, Either, Executor, Row, TypeInfo};

use super::error::DbError;
use super::value;
use super::DbPool;
use crate::model::{ColumnMeta, Driver, QueryResult};

/// Execute SQL and materialise up to `row_limit` rows of the *first*
/// result-producing statement. Later statements are still executed, but their
/// rows are ignored (their affected-row counts are included).
pub async fn execute(
    pool: &DbPool,
    sql: &str,
    row_limit: usize,
    timeout: Duration,
) -> Result<QueryResult, DbError> {
    let sql = sql.trim();
    if sql.is_empty() {
        return Err(DbError::new("Nothing to execute — the editor is empty."));
    }

    let started = Instant::now();
    let row_limit = row_limit.max(1);

    let work = async {
        match pool {
            DbPool::Postgres(p) => execute_pg(p, sql, row_limit).await,
            DbPool::MySql(p) => execute_mysql(p, sql, row_limit).await,
            DbPool::Sqlite(p) => execute_sqlite(p, sql, row_limit).await,
        }
    };

    let mut result = match tokio::time::timeout(timeout, work).await {
        Ok(res) => res?,
        Err(_) => {
            return Err(DbError::new(format!(
                "Query cancelled after exceeding the {}-second timeout. Press F5 to retry or raise the timeout in settings.",
                timeout.as_secs()
            )))
        }
    };
    result.duration_ms = started.elapsed().as_millis() as u64;
    result.sql = sql.to_string();
    Ok(result)
}

async fn execute_pg(
    pool: &sqlx::PgPool,
    sql: &str,
    row_limit: usize,
) -> Result<QueryResult, DbError> {
    let mut stream = sqlx::raw_sql(sql).fetch_many(pool);
    let mut rows = Vec::new();
    let mut columns: Option<Vec<ColumnMeta>> = None;
    let mut affected = 0u64;
    let mut truncated = false;
    let mut in_rows = false;
    let mut first_set_finished = false;
    let mut result_sets = 0usize;

    while let Some(item) = stream
        .try_next()
        .await
        .map_err(|e| DbError::from_sqlx(e, sql))?
    {
        match item {
            Either::Left(done) => {
                affected = affected.saturating_add(done.rows_affected());
                if in_rows {
                    in_rows = false;
                    if columns.is_some() {
                        first_set_finished = true;
                    }
                }
            }
            Either::Right(row) => {
                if first_set_finished {
                    // We already captured the first result set. Keep draining
                    // so remaining statements in the script execute.
                    continue;
                }
                if !in_rows {
                    in_rows = true;
                    result_sets += 1;
                }
                if columns.is_none() {
                    columns = Some(
                        row.columns()
                            .iter()
                            .map(|c| ColumnMeta {
                                name: c.name().to_string(),
                                type_name: c.type_info().name().to_string(),
                            })
                            .collect(),
                    );
                }
                if rows.len() >= row_limit {
                    // Stop collecting but keep draining the stream: with the
                    // simple query protocol the remaining statements in the
                    // script (including COMMIT) still need to execute.
                    truncated = true;
                    first_set_finished = true;
                    continue;
                }
                rows.push(value::postgres_row(&row));
            }
        }
    }

    // A SELECT returning zero rows never produces a Row, so fetch column
    // metadata with a describe round-trip. This also covers UPDATE … RETURNING.
    if columns.is_none() {
        columns = describe_columns_pg(pool, sql).await;
        if columns.is_some() {
            result_sets = result_sets.max(1);
        }
    }

    Ok(QueryResult {
        columns: columns.unwrap_or_default(),
        rows,
        affected,
        duration_ms: 0,
        truncated,
        sql: sql.to_string(),
        result_sets,
    })
}

async fn execute_mysql(
    pool: &sqlx::MySqlPool,
    sql: &str,
    row_limit: usize,
) -> Result<QueryResult, DbError> {
    let mut stream = sqlx::raw_sql(sql).fetch_many(pool);
    let mut rows = Vec::new();
    let mut columns: Option<Vec<ColumnMeta>> = None;
    let mut affected = 0u64;
    let mut truncated = false;
    let mut in_rows = false;
    let mut first_set_finished = false;
    let mut result_sets = 0usize;

    while let Some(item) = stream
        .try_next()
        .await
        .map_err(|e| DbError::from_sqlx(e, sql))?
    {
        match item {
            Either::Left(done) => {
                affected = affected.saturating_add(done.rows_affected());
                if in_rows {
                    in_rows = false;
                    if columns.is_some() {
                        first_set_finished = true;
                    }
                }
            }
            Either::Right(row) => {
                if first_set_finished {
                    continue;
                }
                if !in_rows {
                    in_rows = true;
                    result_sets += 1;
                }
                if columns.is_none() {
                    columns = Some(
                        row.columns()
                            .iter()
                            .map(|c| ColumnMeta {
                                name: c.name().to_string(),
                                type_name: c.type_info().name().to_string(),
                            })
                            .collect(),
                    );
                }
                if rows.len() >= row_limit {
                    // See execute_pg: drain later statements so multi-statement
                    // scripts still commit after the display limit is reached.
                    truncated = true;
                    first_set_finished = true;
                    continue;
                }
                rows.push(value::mysql_row(&row));
            }
        }
    }

    if columns.is_none() {
        columns = describe_columns_mysql(pool, sql).await;
        if columns.is_some() {
            result_sets = result_sets.max(1);
        }
    }

    Ok(QueryResult {
        columns: columns.unwrap_or_default(),
        rows,
        affected,
        duration_ms: 0,
        truncated,
        sql: sql.to_string(),
        result_sets,
    })
}

async fn execute_sqlite(
    pool: &sqlx::SqlitePool,
    sql: &str,
    row_limit: usize,
) -> Result<QueryResult, DbError> {
    let mut stream = sqlx::raw_sql(sql).fetch_many(pool);
    let mut rows = Vec::new();
    let mut columns: Option<Vec<ColumnMeta>> = None;
    let mut affected = 0u64;
    let mut truncated = false;
    let mut in_rows = false;
    let mut first_set_finished = false;
    let mut result_sets = 0usize;

    while let Some(item) = stream
        .try_next()
        .await
        .map_err(|e| DbError::from_sqlx(e, sql))?
    {
        match item {
            Either::Left(done) => {
                affected = affected.saturating_add(done.rows_affected());
                if in_rows {
                    in_rows = false;
                    if columns.is_some() {
                        first_set_finished = true;
                    }
                }
            }
            Either::Right(row) => {
                if first_set_finished {
                    continue;
                }
                if !in_rows {
                    in_rows = true;
                    result_sets += 1;
                }
                if columns.is_none() {
                    columns = Some(
                        row.columns()
                            .iter()
                            .map(|c| ColumnMeta {
                                name: c.name().to_string(),
                                type_name: c.type_info().name().to_string(),
                            })
                            .collect(),
                    );
                }
                if rows.len() >= row_limit {
                    truncated = true;
                    first_set_finished = true;
                    continue;
                }
                rows.push(value::sqlite_row(&row));
            }
        }
    }

    if columns.is_none() {
        columns = describe_columns_sqlite(pool, sql).await;
        if columns.is_some() {
            result_sets = result_sets.max(1);
        }
    }

    Ok(QueryResult {
        columns: columns.unwrap_or_default(),
        rows,
        affected,
        duration_ms: 0,
        truncated,
        sql: sql.to_string(),
        result_sets,
    })
}

async fn describe_columns_sqlite(pool: &sqlx::SqlitePool, sql: &str) -> Option<Vec<ColumnMeta>> {
    let described = tokio::time::timeout(Duration::from_secs(5), pool.describe(sql))
        .await
        .ok()?
        .ok()?;
    if described.columns().is_empty() {
        return None;
    }
    Some(
        described
            .columns()
            .iter()
            .map(|c| ColumnMeta {
                name: c.name().to_string(),
                type_name: c.type_info().name().to_string(),
            })
            .collect(),
    )
}

async fn describe_columns_pg(pool: &sqlx::PgPool, sql: &str) -> Option<Vec<ColumnMeta>> {
    // `describe` prepares the statement on a connection; this is safe for a
    // single statement and intentionally best-effort for scripts.
    let described = tokio::time::timeout(Duration::from_secs(5), pool.describe(sql))
        .await
        .ok()?
        .ok()?;
    if described.columns().is_empty() {
        return None;
    }
    Some(
        described
            .columns()
            .iter()
            .map(|c| ColumnMeta {
                name: c.name().to_string(),
                type_name: c.type_info().name().to_string(),
            })
            .collect(),
    )
}

async fn describe_columns_mysql(pool: &sqlx::MySqlPool, sql: &str) -> Option<Vec<ColumnMeta>> {
    let described = tokio::time::timeout(Duration::from_secs(5), pool.describe(sql))
        .await
        .ok()?
        .ok()?;
    if described.columns().is_empty() {
        return None;
    }
    Some(
        described
            .columns()
            .iter()
            .map(|c| ColumnMeta {
                name: c.name().to_string(),
                type_name: c.type_info().name().to_string(),
            })
            .collect(),
    )
}

/// Return true if any top-level statement looks like transaction control.
///
/// Multi-statement scripts are routed to the single-connection transaction pool
/// when *any* statement controls a transaction, not only when the script starts
/// with `BEGIN`. This guarantees that e.g. `SELECT 1; BEGIN; UPDATE …; COMMIT;`
/// runs on one physical connection end to end.
pub fn is_tx_statement(sql: &str) -> bool {
    split_statements(sql).iter().any(|statement| {
        matches!(
            first_keyword(statement).as_str(),
            "BEGIN" | "START" | "COMMIT" | "ROLLBACK" | "SAVEPOINT" | "RELEASE"
        )
    })
}

/// Whether a completed SQL script leaves a transaction open.
///
/// Every top-level statement is considered, so scripts that both begin and end
/// a transaction (`BEGIN; UPDATE …; COMMIT;`) report the final state instead of
/// leaving a stale "TX OPEN" indicator. Comments, quoted strings and
/// dollar-quoted bodies are skipped while splitting, preventing keywords inside
/// literals from influencing the result.
pub fn tx_state_after(sql: &str) -> Option<bool> {
    let mut state = None;
    for statement in split_statements(sql) {
        match first_keyword(statement).as_str() {
            "BEGIN" | "START" => state = Some(true),
            // `ROLLBACK TO [SAVEPOINT] name` keeps the transaction open.
            "ROLLBACK" if second_keyword(statement) == "TO" => {}
            "COMMIT" | "ROLLBACK" => state = Some(false),
            _ => {}
        }
    }
    state
}

/// Split a SQL script on semicolons that are outside strings and comments.
///
/// The scanner only understands lexical structure; it intentionally does not
/// parse SQL. Dollar-quoted bodies (PostgreSQL), quoted identifiers and both
/// comment styles are skipped so a semicolon inside them cannot split a
/// statement. MySQL's default backslash escapes are honoured for strings.
fn split_statements(sql: &str) -> Vec<&str> {
    let bytes = sql.as_bytes();
    let mut statements = Vec::new();
    let mut start = 0usize;
    let mut i = 0usize;

    while i < bytes.len() {
        match bytes[i] {
            b'\'' | b'"' | b'`' => {
                i = skip_quoted(bytes, i, bytes[i]);
            }
            b'-' if bytes.get(i + 1) == Some(&b'-') => {
                i = bytes[i..]
                    .iter()
                    .position(|&b| b == b'\n')
                    .map(|offset| i + offset + 1)
                    .unwrap_or(bytes.len());
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                i = skip_block_comment(bytes, i);
            }
            b'$' => {
                if let Some((body_start, tag)) = dollar_quote_tag(bytes, i) {
                    i = skip_dollar_quoted(bytes, body_start, tag);
                } else {
                    i += 1;
                }
            }
            b';' => {
                statements.push(&sql[start..i]);
                i += 1;
                start = i;
            }
            _ => i += 1,
        }
    }

    statements.push(&sql[start..]);
    statements
}

/// Skip a single- or double-quoted literal starting at `start`.
fn skip_quoted(bytes: &[u8], start: usize, quote: u8) -> usize {
    let mut i = start + 1;
    while i < bytes.len() {
        if bytes[i] == b'\\' && quote == b'\'' {
            // Backslash escapes (MySQL and PostgreSQL E'' strings). Skipping
            // two bytes also avoids treating an escaped quote as the end.
            i = (i + 2).min(bytes.len());
        } else if bytes[i] == quote {
            if bytes.get(i + 1) == Some(&quote) {
                i += 2; // Standard SQL escaped quote: '' or "".
            } else {
                return i + 1;
            }
        } else {
            i += 1;
        }
    }
    bytes.len()
}

/// Skip a (possibly nested) block comment starting at `start`.
fn skip_block_comment(bytes: &[u8], start: usize) -> usize {
    let mut depth = 0usize;
    let mut i = start;
    while i + 1 < bytes.len() {
        if bytes[i] == b'/' && bytes[i + 1] == b'*' {
            depth += 1;
            i += 2;
        } else if bytes[i] == b'*' && bytes[i + 1] == b'/' {
            depth -= 1;
            i += 2;
            if depth == 0 {
                return i;
            }
        } else {
            i += 1;
        }
    }
    bytes.len()
}

/// Detect a PostgreSQL dollar-quote opener at `start`.
///
/// Returns the index of the first body byte and the full delimiter (`$$` or
/// `$tag$`). `$1`-style placeholders and ordinary identifiers are rejected.
fn dollar_quote_tag(bytes: &[u8], start: usize) -> Option<(usize, &[u8])> {
    let mut i = start + 1;
    if bytes.get(i) == Some(&b'$') {
        return Some((i + 1, &bytes[start..=i]));
    }
    if !bytes.get(i)?.is_ascii_alphabetic() && bytes.get(i) != Some(&b'_') {
        return None;
    }
    i += 1;
    while matches!(bytes.get(i), Some(b) if b.is_ascii_alphanumeric() || *b == b'_') {
        i += 1;
    }
    if bytes.get(i) == Some(&b'$') {
        Some((i + 1, &bytes[start..=i]))
    } else {
        None
    }
}

/// Skip the body of a dollar-quoted string up to and including its closing tag.
fn skip_dollar_quoted(bytes: &[u8], start: usize, tag: &[u8]) -> usize {
    let mut i = start;
    while i < bytes.len() {
        if bytes[i] == b'$' && bytes[i..].starts_with(tag) {
            return i + tag.len();
        }
        i += 1;
    }
    bytes.len()
}

/// First alphabetic keyword after leading whitespace and comments.
fn first_keyword(sql: &str) -> String {
    strip_leading_trivia(sql)
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect::<String>()
        .to_ascii_uppercase()
}

/// Strip whitespace and any leading SQL comments from a statement.
fn strip_leading_trivia(sql: &str) -> &str {
    let mut s = sql.trim_start();
    loop {
        if s.starts_with("--") {
            s = s.find('\n').map(|i| &s[i + 1..]).unwrap_or("");
        } else if s.starts_with("/*") {
            s = skip_leading_block_comment(s);
        } else {
            break;
        }
        s = s.trim_start();
    }
    s
}

/// Remove a leading block comment, respecting PostgreSQL's nesting rules.
fn skip_leading_block_comment(s: &str) -> &str {
    let bytes = s.as_bytes();
    let end = skip_block_comment(bytes, 0);
    &s[end.min(s.len())..]
}

/// Keyword immediately after the first one, if any.
fn second_keyword(sql: &str) -> String {
    let s = strip_leading_trivia(sql);
    // `first_keyword` only collects ASCII bytes, so its length is also a valid
    // byte offset into `s`.
    let first_len = first_keyword(s).len();
    first_keyword(s.get(first_len..).unwrap_or(""))
}

/// Human-readable engine label for log lines.
pub fn engine_label(pool: &DbPool) -> Driver {
    pool.driver()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_transaction_statements() {
        assert!(is_tx_statement("BEGIN;"));
        assert!(is_tx_statement("  start transaction"));
        assert!(is_tx_statement("/* hi */ COMMIT"));
        assert!(!is_tx_statement("SELECT 'BEGIN'"));
        assert_eq!(tx_state_after("begin"), Some(true));
        assert_eq!(tx_state_after("rollback;"), Some(false));
        assert_eq!(tx_state_after("select 1"), None);
        assert_eq!(first_keyword("-- c\n  SELECT 1"), "SELECT");
    }

    #[test]
    fn tracks_state_across_whole_scripts() {
        assert_eq!(
            tx_state_after("BEGIN; UPDATE t SET x = 1; COMMIT;"),
            Some(false)
        );
        assert_eq!(tx_state_after("BEGIN; UPDATE t SET x = 1;"), Some(true));
        assert_eq!(tx_state_after("BEGIN; COMMIT; BEGIN;"), Some(true));
        assert_eq!(tx_state_after("BEGIN; ROLLBACK TO keep;"), Some(true));
        assert_eq!(
            tx_state_after("BEGIN; SAVEPOINT keep; ROLLBACK TO keep; COMMIT;"),
            Some(false)
        );
        assert_eq!(tx_state_after("/* BEGIN; */ SELECT 1"), None);
        assert_eq!(
            tx_state_after("BEGIN; SELECT 'COMMIT;'; ROLLBACK;"),
            Some(false)
        );
        assert_eq!(tx_state_after("-- COMMIT;\nBEGIN;"), Some(true));
        assert_eq!(tx_state_after("select 1"), None);
    }

    #[test]
    fn script_scanner_ignores_quoted_semicolons() {
        // The semicolons and COMMIT inside strings/bodies must not split or
        // influence the script's final transaction state.
        assert_eq!(
            tx_state_after("BEGIN; SELECT $body$; COMMIT; $body$; COMMIT;"),
            Some(false)
        );
        assert_eq!(
            tx_state_after("BEGIN; DO $$ BEGIN PERFORM 1; END $$; COMMIT;"),
            Some(false)
        );
        assert_eq!(
            tx_state_after("BEGIN; SELECT 'it''s; COMMIT;'; ROLLBACK;"),
            Some(false)
        );
        assert_eq!(
            tx_state_after("BEGIN; SELECT \"odd;name\"; COMMIT;"),
            Some(false)
        );
        assert!(!is_tx_statement("SELECT 'BEGIN';"));
        assert!(is_tx_statement(
            "SELECT 1; BEGIN; UPDATE t SET x = 1; COMMIT;"
        ));
    }
}
