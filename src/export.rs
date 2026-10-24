//! Export the current result set as CSV, JSON or SQL INSERT statements.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::model::{Driver, QueryResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    Csv,
    Json,
    Sql,
}

impl ExportFormat {
    pub const ALL: [ExportFormat; 3] = [ExportFormat::Csv, ExportFormat::Json, ExportFormat::Sql];

    pub fn label(self) -> &'static str {
        match self {
            ExportFormat::Csv => "CSV",
            ExportFormat::Json => "JSON",
            ExportFormat::Sql => "SQL INSERT",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            ExportFormat::Csv => "csv",
            ExportFormat::Json => "json",
            ExportFormat::Sql => "sql",
        }
    }

    pub fn next(self) -> Self {
        let idx = Self::ALL.iter().position(|f| *f == self).unwrap_or(0);
        Self::ALL[(idx + 1) % Self::ALL.len()]
    }

    pub fn previous(self) -> Self {
        let idx = Self::ALL.iter().position(|f| *f == self).unwrap_or(0);
        Self::ALL[(idx + Self::ALL.len() - 1) % Self::ALL.len()]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportScope {
    /// Every row in the result set.
    All,
    /// The current filtered/sorted view.
    Visible,
}

impl ExportScope {
    pub fn label(self) -> &'static str {
        match self {
            ExportScope::All => "All rows",
            ExportScope::Visible => "Visible view",
        }
    }

    pub fn toggle(self) -> Self {
        match self {
            ExportScope::All => ExportScope::Visible,
            ExportScope::Visible => ExportScope::All,
        }
    }
}

/// Render the selected rows. `rows` contains indexes into `result.rows`.
pub fn render(
    result: &QueryResult,
    rows: &[usize],
    format: ExportFormat,
    include_headers: bool,
    table: &str,
    driver: Driver,
) -> String {
    match format {
        ExportFormat::Csv => render_csv(result, rows, include_headers),
        ExportFormat::Json => render_json(result, rows),
        ExportFormat::Sql => render_sql(result, rows, table, driver),
    }
}

pub fn render_csv(result: &QueryResult, rows: &[usize], include_headers: bool) -> String {
    let mut out = String::new();
    if include_headers {
        out.push_str(
            &result
                .columns
                .iter()
                .map(|c| csv_field(Some(c.name.as_str())))
                .collect::<Vec<_>>()
                .join(","),
        );
        out.push('\n');
    }
    for &row_idx in rows {
        let Some(row) = result.rows.get(row_idx) else {
            continue;
        };
        for (idx, cell) in row.iter().enumerate() {
            if idx > 0 {
                out.push(',');
            }
            out.push_str(&csv_field(cell.as_deref()));
        }
        out.push('\n');
    }
    out
}

fn csv_field(value: Option<&str>) -> String {
    let Some(value) = value else {
        // CSV has no standard NULL marker; an empty unquoted field is the
        // customary representation and round-trips well enough for importers.
        return String::new();
    };
    if value.contains(',') || value.contains('"') || value.contains('\n') || value.contains('\r') {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

pub fn render_json(result: &QueryResult, rows: &[usize]) -> String {
    let keys = unique_keys(result);
    let mut out = String::from("[\n");
    for (n, &row_idx) in rows.iter().enumerate() {
        let Some(row) = result.rows.get(row_idx) else {
            continue;
        };
        out.push_str("  {\n");
        for (col_idx, key) in keys.iter().enumerate() {
            let value = row.get(col_idx).and_then(|v| v.as_deref());
            let rendered = json_value(value, result, col_idx);
            out.push_str("    ");
            out.push_str(&serde_json::to_string(key).unwrap_or_else(|_| "\"\"".into()));
            out.push_str(": ");
            out.push_str(&rendered);
            if col_idx + 1 < keys.len() {
                out.push(',');
            }
            out.push('\n');
        }
        out.push_str("  }");
        if n + 1 < rows.len() {
            out.push(',');
        }
        out.push('\n');
    }
    out.push_str("]\n");
    out
}

fn unique_keys(result: &QueryResult) -> Vec<String> {
    let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    result
        .columns
        .iter()
        .enumerate()
        .map(|(idx, column)| {
            let base = if column.name.trim().is_empty() {
                format!("column_{}", idx + 1)
            } else {
                column.name.clone()
            };
            let count = counts.entry(base.clone()).or_insert(0);
            *count += 1;
            if *count == 1 {
                base
            } else {
                format!("{base}_{}", *count)
            }
        })
        .collect()
}

fn json_value(value: Option<&str>, result: &QueryResult, col_idx: usize) -> String {
    let Some(value) = value else {
        return "null".to_string();
    };
    if let Some(column) = result.columns.get(col_idx) {
        let trimmed = value.trim();
        if column.is_boolean() {
            match trimmed {
                "true" | "t" | "1" | "TRUE" => return "true".to_string(),
                "false" | "f" | "0" | "FALSE" => return "false".to_string(),
                _ => {}
            }
        }
        if column.is_numeric() && !trimmed.is_empty() {
            // Validate with serde_json so values like NaN or 1e999 fall back to
            // strings instead of producing invalid JSON.
            if let Ok(num) = serde_json::from_str::<serde_json::Number>(trimmed) {
                return num.to_string();
            }
        }
    }
    serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_string())
}

pub fn render_sql(result: &QueryResult, rows: &[usize], table: &str, driver: Driver) -> String {
    let table = if table.trim().is_empty() {
        "exported_data"
    } else {
        table.trim()
    };
    let table = quote_qualified_table(table, driver);
    let mut out = String::new();
    out.push_str("-- Generated by RustQuery\n");
    if rows.is_empty() {
        out.push_str(&format!("-- No rows to insert into {table}\n"));
        return out;
    }

    if result.columns.is_empty() {
        out.push_str(&format!(
            "-- Query returned no columns; nothing to insert into {table}\n"
        ));
        return out;
    }

    out.push_str(&format!("INSERT INTO {table} ("));
    out.push_str(
        &result
            .columns
            .iter()
            .map(|c| quote_identifier(&c.name, driver))
            .collect::<Vec<_>>()
            .join(", "),
    );
    out.push_str(") VALUES\n");

    const BATCH: usize = 100;
    for (n, &row_idx) in rows.iter().enumerate() {
        let Some(row) = result.rows.get(row_idx) else {
            continue;
        };
        let values = row
            .iter()
            .enumerate()
            .map(|(col, cell)| {
                sql_literal(
                    cell.as_deref(),
                    result.columns.get(col).map(|c| c.type_name.as_str()),
                    result.columns.get(col).map(|c| c.name.as_str()),
                    driver,
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        out.push_str("  (");
        out.push_str(&values);
        out.push(')');
        let is_last = n + 1 == rows.len();
        if is_last {
            out.push_str(";\n");
        } else if (n + 1) % BATCH == 0 {
            out.push_str(";\n\n");
            out.push_str(&format!("INSERT INTO {table} ("));
            out.push_str(
                &result
                    .columns
                    .iter()
                    .map(|c| quote_identifier(&c.name, driver))
                    .collect::<Vec<_>>()
                    .join(", "),
            );
            out.push_str(") VALUES\n");
        } else {
            out.push_str(",\n");
        }
    }
    out
}

fn sql_literal(
    value: Option<&str>,
    type_name: Option<&str>,
    column_name: Option<&str>,
    driver: Driver,
) -> String {
    let Some(value) = value else {
        return "NULL".to_string();
    };
    let ty = type_name.unwrap_or("").to_ascii_uppercase();
    let trimmed = value.trim();
    let numeric = ty.contains("INT")
        || ty.contains("SERIAL")
        || ty.contains("NUMERIC")
        || ty.contains("DECIMAL")
        || ty.contains("FLOAT")
        || ty.contains("DOUBLE")
        || ty.contains("REAL")
        || ty.contains("MONEY");
    if numeric
        && !trimmed.is_empty()
        && trimmed != "NaN"
        && trimmed != "Infinity"
        && serde_json::from_str::<serde_json::Number>(trimmed).is_ok()
    {
        return trimmed.to_string();
    }
    let boolean = ty == "BOOL" || ty == "BOOLEAN";
    if boolean {
        match trimmed.to_ascii_lowercase().as_str() {
            "true" | "t" | "1" => return "TRUE".to_string(),
            "false" | "f" | "0" => return "FALSE".to_string(),
            _ => {}
        }
    }
    // A column named like a JSON document could be exported as JSONB; without
    // reading its declared type we keep values as escaped text.
    let _ = column_name;
    let escaped = match driver {
        // SQLite follows the SQL standard: single quotes are doubled and
        // backslashes have no special meaning.
        Driver::Postgres | Driver::Sqlite => value.replace('\0', "").replace('\'', "''"),
        Driver::MySql => value
            .replace('\0', "")
            .replace('\\', "\\\\")
            .replace('\'', "''"),
    };
    format!("'{escaped}'")
}

fn quote_identifier(name: &str, driver: Driver) -> String {
    match driver {
        Driver::Postgres | Driver::Sqlite => format!("\"{}\"", name.replace('"', "\"\"")),
        Driver::MySql => format!("`{}`", name.replace('`', "``")),
    }
}

fn quote_qualified_table(table: &str, driver: Driver) -> String {
    table
        .split('.')
        .map(|part| quote_identifier(part, driver))
        .collect::<Vec<_>>()
        .join(".")
}

/// Append the format extension if the user omitted it.
pub fn ensure_extension(path: &Path, format: ExportFormat) -> PathBuf {
    match path.extension().and_then(|e| e.to_str()) {
        Some(ext) if ext.eq_ignore_ascii_case(format.extension()) => path.to_path_buf(),
        _ => path.with_extension(format.extension()),
    }
}

/// Write an export file, creating parent directories as needed.
pub fn write_export(path: &Path, contents: &str) -> Result<()> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating export directory {}", parent.display()))?;
    }
    std::fs::write(path, contents)
        .with_context(|| format!("writing export file {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ColumnMeta, QueryResult};

    fn sample() -> QueryResult {
        QueryResult {
            columns: vec![
                ColumnMeta {
                    name: "id".into(),
                    type_name: "INT4".into(),
                },
                ColumnMeta {
                    name: "name".into(),
                    type_name: "TEXT".into(),
                },
                ColumnMeta {
                    name: "active".into(),
                    type_name: "BOOL".into(),
                },
            ],
            rows: vec![
                vec![
                    Some("1".into()),
                    Some("Alice, \"A\"".into()),
                    Some("true".into()),
                ],
                vec![None, None, Some("false".into())],
            ],
            affected: 0,
            duration_ms: 1,
            truncated: false,
            sql: "select 1".into(),
            result_sets: 1,
        }
    }

    #[test]
    fn csv_escapes_quotes_and_commas() {
        let result = sample();
        let csv = render_csv(&result, &[0, 1], true);
        assert!(csv.starts_with("id,name,active\n"));
        assert!(csv.contains("\"Alice, \"\"A\"\"\""));
        assert!(csv.ends_with("\n"));
    }

    #[test]
    fn json_preserves_types_and_nulls() {
        let result = sample();
        let json = render_json(&result, &[0, 1]);
        assert!(json.contains("\"id\": 1"));
        assert!(json.contains("\"active\": true"));
        assert!(json.contains("\"name\": null"));
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(parsed.is_array());
    }

    #[test]
    fn sql_identifiers_and_literals() {
        let result = sample();
        let sql = render_sql(&result, &[0], "public.my\"table", Driver::Postgres);
        assert!(sql.contains("INSERT INTO \"public\".\"my\"\"table\""));
        assert!(sql.contains("'Alice, \"A\"'"));
        assert!(sql.contains("TRUE"));
        assert!(sql.ends_with(";\n"));
    }

    #[test]
    fn mysql_identifier_quoting() {
        assert_eq!(quote_qualified_table("a.b", Driver::MySql), "`a`.`b`");
    }

    #[test]
    fn sqlite_identifier_and_literal_quoting() {
        assert_eq!(
            quote_qualified_table("main.my\"table", Driver::Sqlite),
            "\"main\".\"my\"\"table\""
        );
        let result = sample();
        let sql = render_sql(&result, &[0], "main.users", Driver::Sqlite);
        assert!(sql.contains("INSERT INTO \"main\".\"users\""));
        assert!(sql.contains("'Alice, \"A\"'"));
    }

    #[test]
    fn extension_is_appended() {
        assert_eq!(
            ensure_extension(Path::new("/tmp/x"), ExportFormat::Csv),
            PathBuf::from("/tmp/x.csv")
        );
        assert_eq!(
            ensure_extension(Path::new("/tmp/x.CSV"), ExportFormat::Csv),
            PathBuf::from("/tmp/x.CSV")
        );
    }

    #[test]
    fn json_duplicate_column_names_are_disambiguated() {
        let result = QueryResult {
            columns: vec![
                ColumnMeta {
                    name: "x".into(),
                    type_name: "TEXT".into(),
                },
                ColumnMeta {
                    name: "x".into(),
                    type_name: "TEXT".into(),
                },
            ],
            rows: vec![vec![Some("a".into()), Some("b".into())]],
            affected: 0,
            duration_ms: 0,
            truncated: false,
            sql: String::new(),
            result_sets: 0,
        };
        let json = render_json(&result, &[0]);
        assert!(json.contains("\"x\":"));
        assert!(json.contains("\"x_2\":"));
    }
}
