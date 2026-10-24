//! Dynamic value formatting.
//!
//! SQLx is statically typed, while a SQL workbench must display *arbitrary*
//! result columns. We therefore inspect each column's runtime type metadata and
//! decode the value into the most appropriate Rust type. A raw fallback keeps
//! unknown/custom types readable instead of failing the whole grid.

use bigdecimal::BigDecimal;
use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, SecondsFormat, Utc};
use sqlx::mysql::{MySqlRow, MySqlTypeInfo};
use sqlx::postgres::{PgRow, PgTypeInfo, PgTypeKind};
use sqlx::sqlite::SqliteRow;
use sqlx::{Column, Row, TypeInfo, ValueRef};
use uuid::Uuid;

/// Avoid storing unbounded values (a single BLOB could otherwise exhaust RAM).
const MAX_CELL_CHARS: usize = 16_384;
/// Binary values are rendered as a short hex preview.
const MAX_BLOB_BYTES: usize = 128;

pub fn postgres_row(row: &PgRow) -> Vec<Option<String>> {
    row.columns()
        .iter()
        .enumerate()
        .map(|(idx, col)| {
            let value = extract_pg(row, idx, col.type_info());
            value.map(|v| clamp(v, MAX_CELL_CHARS))
        })
        .collect()
}

pub fn mysql_row(row: &MySqlRow) -> Vec<Option<String>> {
    row.columns()
        .iter()
        .enumerate()
        .map(|(idx, col)| {
            let value = extract_mysql(row, idx, col.type_info());
            value.map(|v| clamp(v, MAX_CELL_CHARS))
        })
        .collect()
}

/// Format a SQLite row.
///
/// SQLite columns are dynamically typed: the same column can contain an
/// INTEGER in one row and TEXT in the next. SQLx's per-value `type_info()`
/// reports the actual storage class, so values are decoded from that rather
/// than from the column's declared affinity.
pub fn sqlite_row(row: &SqliteRow) -> Vec<Option<String>> {
    row.columns()
        .iter()
        .enumerate()
        .map(|(idx, _)| {
            let value = extract_sqlite(row, idx);
            value.map(|v| clamp(v, MAX_CELL_CHARS))
        })
        .collect()
}

fn extract_sqlite(row: &SqliteRow, idx: usize) -> Option<String> {
    let type_name = {
        let raw = row.try_get_raw(idx).ok()?;
        if raw.is_null() {
            return None;
        }
        raw.type_info().name().to_string()
    };

    match type_name.as_str() {
        "INTEGER" | "INT4" => row.try_get::<i64, _>(idx).ok().map(|v| v.to_string()),
        "REAL" => row.try_get::<f64, _>(idx).ok().map(|v| v.to_string()),
        "TEXT" => row.try_get::<String, _>(idx).ok(),
        "BLOB" => row
            .try_get::<Vec<u8>, _>(idx)
            .ok()
            .map(|v| render_bytes(&v)),
        "BOOLEAN" => row
            .try_get::<bool, _>(idx)
            .ok()
            .map(|v| if v { "true" } else { "false" }.to_string()),
        "NULL" => None,
        // Declared DATE/TIME/DATETIME/NUMERIC columns override the runtime
        // storage class in the column metadata, but the stored value itself is
        // usually text. Show it verbatim; dynamic fallbacks cover the rest.
        "DATE" | "TIME" | "DATETIME" | "NUMERIC" => row
            .try_get::<String, _>(idx)
            .ok()
            .or_else(|| row.try_get::<i64, _>(idx).ok().map(|v| v.to_string()))
            .or_else(|| row.try_get::<f64, _>(idx).ok().map(|v| v.to_string())),
        _ => raw_sqlite_fallback(row, idx, &type_name),
    }
}

fn raw_sqlite_fallback(row: &SqliteRow, idx: usize, name: &str) -> Option<String> {
    if let Ok(text) = row.try_get::<String, _>(idx) {
        return Some(text);
    }
    if let Ok(number) = row.try_get::<i64, _>(idx) {
        return Some(number.to_string());
    }
    if let Ok(number) = row.try_get::<f64, _>(idx) {
        return Some(number.to_string());
    }
    if let Ok(bytes) = row.try_get::<Vec<u8>, _>(idx) {
        return Some(render_bytes(&bytes));
    }
    Some(format!("<{name}>"))
}

fn clamp(mut value: String, max_chars: usize) -> String {
    if value.chars().count() > max_chars {
        value = value.chars().take(max_chars).collect();
        value.push('…');
    }
    value
}

fn is_null_pg(row: &PgRow, idx: usize) -> bool {
    row.try_get_raw(idx)
        .map(|raw| raw.is_null())
        .unwrap_or(true)
}

fn is_null_mysql(row: &MySqlRow, idx: usize) -> bool {
    row.try_get_raw(idx)
        .map(|raw| raw.is_null())
        .unwrap_or(true)
}

fn extract_pg(row: &PgRow, idx: usize, ty: &PgTypeInfo) -> Option<String> {
    if is_null_pg(row, idx) {
        return None;
    }

    // Arrays need to be handled before name matching: their SQL name is
    // prefixed with an underscore (e.g. `_int4`) or is a custom array name.
    if let PgTypeKind::Array(elem) = ty.kind() {
        if let Some(v) = extract_pg_array(row, idx, elem.name()) {
            return Some(v);
        }
    }

    let name = ty.name();
    match name {
        "BOOL" => row
            .try_get::<bool, _>(idx)
            .ok()
            .map(|v| if v { "true" } else { "false" }.to_string()),
        "INT2" => get_pg::<i16>(row, idx),
        "INT4" => get_pg::<i32>(row, idx),
        "INT8" => get_pg::<i64>(row, idx),
        // `sqlx` models PostgreSQL's OID as a newtype rather than `u32`.
        "OID" => row
            .try_get::<sqlx::postgres::types::Oid, _>(idx)
            .ok()
            .map(|v| v.0.to_string()),
        "FLOAT4" => get_pg::<f32>(row, idx),
        "FLOAT8" => get_pg::<f64>(row, idx),
        "NUMERIC" => row
            .try_get::<BigDecimal, _>(idx)
            .ok()
            .map(|v| v.to_string()),
        "TEXT" | "VARCHAR" | "BPCHAR" | "NAME" | "CHAR" | "UNKNOWN" => {
            row.try_get::<String, _>(idx).ok()
        }
        "UUID" => row.try_get::<Uuid, _>(idx).ok().map(|v| v.to_string()),
        "JSON" | "JSONB" => row
            .try_get::<serde_json::Value, _>(idx)
            .ok()
            .map(|v| serde_json::to_string(&v).unwrap_or_else(|_| v.to_string())),
        "TIMESTAMP" => row
            .try_get::<NaiveDateTime, _>(idx)
            .ok()
            .map(|v| v.format("%Y-%m-%d %H:%M:%S%.f").to_string()),
        "TIMESTAMPTZ" => row
            .try_get::<DateTime<Utc>, _>(idx)
            .ok()
            .map(|v| v.to_rfc3339_opts(SecondsFormat::AutoSi, true)),
        "DATE" => row
            .try_get::<NaiveDate, _>(idx)
            .ok()
            .map(|v| v.format("%Y-%m-%d").to_string()),
        "TIME" => row
            .try_get::<NaiveTime, _>(idx)
            .ok()
            .map(|v| v.format("%H:%M:%S%.f").to_string()),
        "BYTEA" => row
            .try_get::<Vec<u8>, _>(idx)
            .ok()
            .map(|v| render_bytes(&v)),
        "INTERVAL" => row
            .try_get::<sqlx::postgres::types::PgInterval, _>(idx)
            .ok()
            .map(render_pg_interval),
        "MONEY" => row.try_get::<String, _>(idx).ok(),
        "VOID" => Some("NULL".to_string()),
        _ => raw_pg_fallback(row, idx, name),
    }
}

fn get_pg<T>(row: &PgRow, idx: usize) -> Option<String>
where
    T: for<'r> sqlx::Decode<'r, sqlx::Postgres> + sqlx::Type<sqlx::Postgres> + std::fmt::Display,
{
    row.try_get::<T, _>(idx).ok().map(|v| v.to_string())
}

fn extract_pg_array(row: &PgRow, idx: usize, elem_name: &str) -> Option<String> {
    match elem_name {
        "TEXT" | "VARCHAR" | "BPCHAR" | "NAME" | "CITEXT" => {
            get_pg_array::<String>(row, idx, |v| v.to_string())
        }
        "INT2" => get_pg_array::<i16>(row, idx, |v| v.to_string()),
        "INT4" => get_pg_array::<i32>(row, idx, |v| v.to_string()),
        "INT8" => get_pg_array::<i64>(row, idx, |v| v.to_string()),
        "FLOAT4" => get_pg_array::<f32>(row, idx, |v| v.to_string()),
        "FLOAT8" => get_pg_array::<f64>(row, idx, |v| v.to_string()),
        "BOOL" => get_pg_array::<bool>(row, idx, |v| v.to_string()),
        "UUID" => get_pg_array::<Uuid>(row, idx, |v| v.to_string()),
        // Unknown element types are common with enums. Show a marker rather
        // than hiding the whole column.
        other => Some(format!("<array of {other}>")),
    }
}

fn get_pg_array<T>(row: &PgRow, idx: usize, fmt: impl Fn(T) -> String) -> Option<String>
where
    T: for<'r> sqlx::Decode<'r, sqlx::Postgres>
        + sqlx::Type<sqlx::Postgres>
        + sqlx::postgres::PgHasArrayType,
{
    let values = row.try_get::<Vec<Option<T>>, _>(idx).ok()?;
    let rendered: Vec<String> = values
        .into_iter()
        .map(|v| v.map(&fmt).unwrap_or_else(|| "NULL".to_string()))
        .collect();
    Some(format!("{{{}}}", rendered.join(",")))
}

/// Last-resort decoding. Works exceptionally well with the text protocol:
/// custom enums, domains and extensions become readable labels.
fn raw_pg_fallback(row: &PgRow, idx: usize, name: &str) -> Option<String> {
    if let Ok(raw) = row.try_get_raw(idx) {
        if let Ok(s) = raw.as_str() {
            return Some(s.to_string());
        }
        if let Ok(bytes) = raw.as_bytes() {
            // Enum and domain labels are encoded as plain UTF-8 even in binary.
            if let Ok(s) = std::str::from_utf8(bytes) {
                if !s.is_empty() && s.chars().all(|c| !c.is_control() || c == '\n' || c == '\t') {
                    return Some(s.to_string());
                }
            }
            return Some(format!("<{} {} bytes>", name, bytes.len()));
        }
    }
    Some(format!("<{name}>"))
}

fn extract_mysql(row: &MySqlRow, idx: usize, ty: &MySqlTypeInfo) -> Option<String> {
    if is_null_mysql(row, idx) {
        return None;
    }
    let name = ty.name();
    match name {
        "BOOLEAN" => row
            .try_get::<bool, _>(idx)
            .ok()
            .map(|v| if v { "true" } else { "false" }.to_string()),
        n if n.contains("INT") || n == "YEAR" => {
            if n.contains("UNSIGNED") {
                row.try_get::<u64, _>(idx).ok().map(|v| v.to_string())
            } else {
                row.try_get::<i64, _>(idx).ok().map(|v| v.to_string())
            }
        }
        "FLOAT" | "DOUBLE" => row.try_get::<f64, _>(idx).ok().map(|v| v.to_string()),
        "DECIMAL" => row
            .try_get::<BigDecimal, _>(idx)
            .ok()
            .map(|v| v.to_string()),
        "VARCHAR" | "CHAR" | "TEXT" | "TINYTEXT" | "MEDIUMTEXT" | "LONGTEXT" | "ENUM" | "SET" => {
            row.try_get::<String, _>(idx).ok()
        }
        "JSON" => row
            .try_get::<serde_json::Value, _>(idx)
            .ok()
            .map(|v| serde_json::to_string(&v).unwrap_or_else(|_| v.to_string())),
        "DATE" => row
            .try_get::<NaiveDate, _>(idx)
            .ok()
            .map(|v| v.format("%Y-%m-%d").to_string()),
        "DATETIME" | "TIMESTAMP" => row
            .try_get::<NaiveDateTime, _>(idx)
            .ok()
            .or_else(|| {
                row.try_get::<DateTime<Utc>, _>(idx)
                    .ok()
                    .map(|dt| dt.naive_utc())
            })
            .map(|v| v.format("%Y-%m-%d %H:%M:%S%.f").to_string()),
        "TIME" => row
            .try_get::<NaiveTime, _>(idx)
            .ok()
            .map(|v| v.format("%H:%M:%S%.f").to_string()),
        "BIT" => row
            .try_get::<Vec<u8>, _>(idx)
            .ok()
            .map(|v| render_bytes(&v)),
        n if n.contains("BLOB") || n.contains("BINARY") => row
            .try_get::<Vec<u8>, _>(idx)
            .ok()
            .map(|v| render_bytes(&v)),
        _ => raw_mysql_fallback(row, idx, name),
    }
}

fn raw_mysql_fallback(row: &MySqlRow, idx: usize, name: &str) -> Option<String> {
    // SQLx does not expose raw MySQL bytes through its public `ValueRef` API,
    // so try the two lossless representations first. Most unknown types (enums,
    // sets, geometry in text mode) decode as strings; binary types as bytes.
    if let Ok(text) = row.try_get::<String, _>(idx) {
        return Some(text);
    }
    if let Ok(bytes) = row.try_get::<Vec<u8>, _>(idx) {
        if let Ok(text) = std::str::from_utf8(&bytes) {
            if !text.is_empty()
                && text
                    .chars()
                    .all(|c| !c.is_control() || c == '\n' || c == '\t')
            {
                return Some(text.to_string());
            }
        }
        return Some(format!("<{} {} bytes>", name, bytes.len()));
    }
    Some(format!("<{name}>"))
}

fn render_pg_interval(iv: sqlx::postgres::types::PgInterval) -> String {
    let mut parts = Vec::new();
    let years = iv.months / 12;
    let months = iv.months % 12;
    if years != 0 {
        parts.push(format!(
            "{years} year{}",
            if years.abs() == 1 { "" } else { "s" }
        ));
    }
    if months != 0 {
        parts.push(format!(
            "{months} mon{}",
            if months.abs() == 1 { "" } else { "s" }
        ));
    }
    if iv.days != 0 {
        parts.push(format!(
            "{} day{}",
            iv.days,
            if iv.days.abs() == 1 { "" } else { "s" }
        ));
    }
    if iv.microseconds != 0 {
        let micros = iv.microseconds;
        let sign = if micros < 0 { "-" } else { "" };
        let abs = micros.unsigned_abs();
        let secs = abs / 1_000_000;
        let frac = abs % 1_000_000;
        if frac == 0 {
            parts.push(format!("{sign}{secs} sec"));
        } else {
            parts.push(format!("{sign}{secs}.{frac:06} sec"));
        }
    }
    if parts.is_empty() {
        "0".to_string()
    } else {
        parts.join(" ")
    }
}

fn render_bytes(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let shown = &bytes[..bytes.len().min(MAX_BLOB_BYTES)];
    let mut out = String::with_capacity(shown.len() * 2 + 16);
    out.push_str("\\x");
    for b in shown {
        let _ = write!(out, "{b:02x}");
    }
    if bytes.len() > MAX_BLOB_BYTES {
        let _ = write!(out, "… ({} bytes)", bytes.len());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_preview_is_bounded() {
        let data = vec![0xab; 300];
        let rendered = render_bytes(&data);
        assert!(rendered.starts_with("\\xabab"));
        assert!(rendered.contains("300 bytes"));
        assert!(rendered.len() < 400);
    }

    #[test]
    fn interval_renders() {
        let iv = sqlx::postgres::types::PgInterval {
            months: 14,
            days: 3,
            microseconds: 1_500_000,
        };
        assert_eq!(render_pg_interval(iv), "1 year 2 mons 3 days 1.500000 sec");
    }

    #[test]
    fn truncation_respects_chars() {
        assert_eq!(clamp("héllo".into(), 3), "hél…");
    }
}
