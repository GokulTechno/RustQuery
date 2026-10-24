//! Metadata queries used by the schema explorer.
//!
//! Introspection is lazy: connecting loads schemas and table/view names;
//! columns, indexes and foreign keys are fetched only when a table is expanded.

use std::time::Duration;

use sqlx::mysql::MySqlRow;
use sqlx::postgres::PgRow;
use sqlx::sqlite::SqliteRow;
use sqlx::Row;

use super::error::DbError;
use super::DbPool;
use crate::model::{
    ColumnInfo, ForeignKeyInfo, IndexInfo, ObjectKind, SchemaEntry, TableDetails, TableEntry,
};

const INTROSPECT_TIMEOUT: Duration = Duration::from_secs(20);

pub async fn list_schemas(pool: &DbPool) -> Result<Vec<SchemaEntry>, DbError> {
    let work = async {
        match pool {
            DbPool::Postgres(p) => list_pg(p).await,
            DbPool::MySql(p) => list_mysql(p).await,
            DbPool::Sqlite(p) => list_sqlite(p).await,
        }
    };
    match tokio::time::timeout(INTROSPECT_TIMEOUT, work).await {
        Ok(res) => res,
        Err(_) => Err(DbError::new(
            "Schema discovery timed out after 20 seconds. Use Refresh in the explorer to retry.",
        )),
    }
}

async fn list_pg(pool: &sqlx::PgPool) -> Result<Vec<SchemaEntry>, DbError> {
    // pg_catalog is more reliable than information_schema for partitioned and
    // foreign tables, and retains empty user schemas thanks to the LEFT JOIN.
    let rows = sqlx::query(
        r#"
        SELECT s.nspname AS schema_name,
               c.relname AS table_name,
               CASE WHEN c.relkind IN ('v', 'm') THEN 'VIEW' ELSE 'TABLE' END AS object_type
        FROM pg_catalog.pg_namespace s
        LEFT JOIN pg_catalog.pg_class c
               ON c.relnamespace = s.oid
              AND c.relkind IN ('r', 'p', 'v', 'm', 'f')
        WHERE s.nspname NOT IN ('pg_catalog', 'information_schema')
          AND s.nspname NOT LIKE 'pg_toast%'
          AND s.nspname NOT LIKE 'pg_temp%'
        ORDER BY s.nspname, object_type, c.relname
        "#,
    )
    .fetch_all(pool)
    .await
    .map_err(|e| DbError::from_sqlx(e, ""))?;

    let mut schemas: Vec<SchemaEntry> = Vec::new();
    for row in rows {
        let schema_name: String = row.try_get("schema_name").unwrap_or_default();
        let table_name: Option<String> = row.try_get("table_name").unwrap_or_default();
        let object_type: Option<String> = row.try_get("object_type").unwrap_or_default();

        if schemas
            .last()
            .map(|s| s.name != schema_name)
            .unwrap_or(true)
        {
            schemas.push(SchemaEntry {
                name: schema_name.clone(),
                objects: Vec::new(),
            });
        }
        if let Some(name) = table_name.filter(|n| !n.is_empty()) {
            if let Some(schema) = schemas.last_mut() {
                schema.objects.push(TableEntry {
                    schema: schema_name,
                    name,
                    kind: if object_type.as_deref() == Some("VIEW") {
                        ObjectKind::View
                    } else {
                        ObjectKind::Table
                    },
                });
            }
        }
    }
    Ok(schemas)
}

async fn list_mysql(pool: &sqlx::MySqlPool) -> Result<Vec<SchemaEntry>, DbError> {
    let rows = sqlx::query(
        r#"
        SELECT s.schema_name AS schema_name,
               t.table_name AS table_name,
               t.table_type AS object_type
        FROM information_schema.schemata s
        LEFT JOIN information_schema.tables t
               ON t.table_schema = s.schema_name
        WHERE s.schema_name NOT IN ('information_schema', 'mysql', 'performance_schema', 'sys')
        ORDER BY s.schema_name, t.table_name
        "#,
    )
    .fetch_all(pool)
    .await
    .map_err(|e| DbError::from_sqlx(e, ""))?;

    let mut schemas: Vec<SchemaEntry> = Vec::new();
    for row in rows {
        let schema_name: String = row.try_get("schema_name").unwrap_or_default();
        let table_name: Option<String> = row.try_get("table_name").unwrap_or_default();
        let object_type: Option<String> = row.try_get("object_type").unwrap_or_default();

        if schemas
            .last()
            .map(|s| s.name != schema_name)
            .unwrap_or(true)
        {
            schemas.push(SchemaEntry {
                name: schema_name.clone(),
                objects: Vec::new(),
            });
        }
        if let Some(name) = table_name.filter(|n| !n.is_empty()) {
            if let Some(schema) = schemas.last_mut() {
                schema.objects.push(TableEntry {
                    schema: schema_name,
                    name,
                    kind: if object_type
                        .as_deref()
                        .map(|t| t.to_ascii_uppercase().contains("VIEW"))
                        .unwrap_or(false)
                    {
                        ObjectKind::View
                    } else {
                        ObjectKind::Table
                    },
                });
            }
        }
    }
    Ok(schemas)
}

/// SQLite exposes every attached database (normally just `main`) through
/// `PRAGMA database_list`. Each one is presented as a schema node.
async fn list_sqlite(pool: &sqlx::SqlitePool) -> Result<Vec<SchemaEntry>, DbError> {
    let databases = sqlx::query("PRAGMA database_list")
        .fetch_all(pool)
        .await
        .map_err(|e| DbError::from_sqlx(e, ""))?;

    let mut schemas = Vec::new();
    for database in &databases {
        let name: String = database.try_get("name").unwrap_or_default();
        if name.is_empty() || name == "temp" {
            continue;
        }
        // The schema name is an identifier, not a bindable value. Quote it so
        // attached databases with unusual names remain safe.
        let sql = format!(
            "SELECT name, type FROM {}.sqlite_schema \
             WHERE type IN ('table', 'view') AND name NOT LIKE 'sqlite_%' \
             ORDER BY CASE type WHEN 'table' THEN 0 ELSE 1 END, name",
            sqlite_ident(&name)
        );
        let objects = sqlx::query(&sql)
            .fetch_all(pool)
            .await
            .map_err(|e| DbError::from_sqlx(e, &sql))?;

        let mut schema = SchemaEntry {
            name: name.clone(),
            objects: Vec::new(),
        };
        for object in &objects {
            let table: String = object.try_get("name").unwrap_or_default();
            let object_type: String = object.try_get("type").unwrap_or_default();
            if table.is_empty() {
                continue;
            }
            schema.objects.push(TableEntry {
                schema: name.clone(),
                name: table,
                kind: if object_type.eq_ignore_ascii_case("view") {
                    ObjectKind::View
                } else {
                    ObjectKind::Table
                },
            });
        }
        schemas.push(schema);
    }

    // An empty in-memory database still has a useful "main" node.
    if schemas.is_empty() {
        schemas.push(SchemaEntry {
            name: "main".to_string(),
            objects: Vec::new(),
        });
    }
    Ok(schemas)
}

pub async fn table_details(
    pool: &DbPool,
    schema: &str,
    table: &str,
) -> Result<TableDetails, DbError> {
    let work = async {
        match pool {
            DbPool::Postgres(p) => pg_table_details(p, schema, table).await,
            DbPool::MySql(p) => mysql_table_details(p, schema, table).await,
            DbPool::Sqlite(p) => sqlite_table_details(p, schema, table).await,
        }
    };
    match tokio::time::timeout(INTROSPECT_TIMEOUT, work).await {
        Ok(res) => res,
        Err(_) => Err(DbError::new("Table metadata query timed out.")),
    }
}

async fn pg_table_details(
    pool: &sqlx::PgPool,
    schema: &str,
    table: &str,
) -> Result<TableDetails, DbError> {
    let columns = sqlx::query(
        r#"
        SELECT c.column_name,
               CASE WHEN c.data_type = 'ARRAY' THEN c.udt_name ELSE c.data_type END AS data_type,
               (c.is_nullable = 'YES') AS nullable,
               c.column_default,
               EXISTS (
                   SELECT 1
                   FROM information_schema.table_constraints tc
                   JOIN information_schema.key_column_usage kcu
                     ON kcu.constraint_name = tc.constraint_name
                    AND kcu.constraint_schema = tc.constraint_schema
                   WHERE tc.constraint_type = 'PRIMARY KEY'
                     AND tc.table_schema = c.table_schema
                     AND tc.table_name = c.table_name
                     AND kcu.column_name = c.column_name
               ) AS is_primary,
               EXISTS (
                   SELECT 1
                   FROM information_schema.table_constraints tc
                   JOIN information_schema.key_column_usage kcu
                     ON kcu.constraint_name = tc.constraint_name
                    AND kcu.constraint_schema = tc.constraint_schema
                   WHERE tc.constraint_type = 'UNIQUE'
                     AND tc.table_schema = c.table_schema
                     AND tc.table_name = c.table_name
                     AND kcu.column_name = c.column_name
               ) AS is_unique,
               c.ordinal_position
        FROM information_schema.columns c
        WHERE c.table_schema = $1 AND c.table_name = $2
        ORDER BY c.ordinal_position
        "#,
    )
    .bind(schema)
    .bind(table)
    .fetch_all(pool)
    .await
    .map_err(|e| DbError::from_sqlx(e, ""))?
    .iter()
    .map(|row: &PgRow| {
        let data_type: String = row.try_get("data_type").unwrap_or_default();
        let data_type = if let Some(base) = data_type.strip_prefix('_') {
            format!("{base}[]")
        } else {
            data_type
        };
        let is_pk: bool = row.try_get("is_primary").unwrap_or(false);
        let is_unique: bool = row.try_get("is_unique").unwrap_or(false);
        ColumnInfo {
            name: row.try_get("column_name").unwrap_or_default(),
            data_type,
            nullable: row.try_get("nullable").unwrap_or(true),
            default: row
                .try_get::<Option<String>, _>("column_default")
                .unwrap_or_default(),
            key: if is_pk {
                "PK".to_string()
            } else if is_unique {
                "UQ".to_string()
            } else {
                String::new()
            },
            ordinal: row.try_get("ordinal_position").unwrap_or_default(),
        }
    })
    .collect();

    let indexes = sqlx::query(
        r#"
        SELECT i.relname AS index_name,
               ix.indisunique AS is_unique,
               ix.indisprimary AS is_primary,
               COALESCE(string_agg(a.attname, ', ' ORDER BY k.ord), '') AS columns,
               am.amname AS method
        FROM pg_catalog.pg_index ix
        JOIN pg_catalog.pg_class i ON i.oid = ix.indexrelid
        JOIN pg_catalog.pg_class t ON t.oid = ix.indrelid
        JOIN pg_catalog.pg_namespace n ON n.oid = t.relnamespace
        JOIN pg_catalog.pg_am am ON am.oid = i.relam
        LEFT JOIN LATERAL unnest(ix.indkey) WITH ORDINALITY AS k(attnum, ord) ON true
        LEFT JOIN pg_catalog.pg_attribute a
               ON a.attrelid = t.oid AND a.attnum = k.attnum
        WHERE n.nspname = $1 AND t.relname = $2
        GROUP BY i.relname, ix.indisunique, ix.indisprimary, am.amname
        ORDER BY ix.indisprimary DESC, i.relname
        "#,
    )
    .bind(schema)
    .bind(table)
    .fetch_all(pool)
    .await
    .map_err(|e| DbError::from_sqlx(e, ""))?
    .iter()
    .map(|row: &PgRow| {
        let columns: String = row.try_get("columns").unwrap_or_default();
        IndexInfo {
            name: row.try_get("index_name").unwrap_or_default(),
            unique: row.try_get("is_unique").unwrap_or(false),
            primary: row.try_get("is_primary").unwrap_or(false),
            columns: split_columns(&columns),
            method: row.try_get("method").unwrap_or_default(),
        }
    })
    .collect();

    let fk_rows = sqlx::query(
        r#"
        SELECT con.conname AS constraint_name,
               att.attname AS column_name,
               fn.nspname AS ref_schema,
               fc.relname AS ref_table,
               fatt.attname AS ref_column,
               CASE con.confdeltype
                   WHEN 'c' THEN 'CASCADE'
                   WHEN 'n' THEN 'SET NULL'
                   WHEN 'd' THEN 'SET DEFAULT'
                   WHEN 'r' THEN 'RESTRICT'
                   ELSE 'NO ACTION'
               END AS on_delete
        FROM pg_catalog.pg_constraint con
        JOIN pg_catalog.pg_class t ON t.oid = con.conrelid
        JOIN pg_catalog.pg_namespace n ON n.oid = t.relnamespace
        JOIN pg_catalog.pg_class fc ON fc.oid = con.confrelid
        JOIN pg_catalog.pg_namespace fn ON fn.oid = fc.relnamespace
        CROSS JOIN LATERAL unnest(con.conkey, con.confkey)
             WITH ORDINALITY AS k(attnum, refattnum, ord)
        JOIN pg_catalog.pg_attribute att
             ON att.attrelid = con.conrelid AND att.attnum = k.attnum
        JOIN pg_catalog.pg_attribute fatt
             ON fatt.attrelid = con.confrelid AND fatt.attnum = k.refattnum
        WHERE con.contype = 'f' AND n.nspname = $1 AND t.relname = $2
        ORDER BY con.conname, k.ord
        "#,
    )
    .bind(schema)
    .bind(table)
    .fetch_all(pool)
    .await
    .map_err(|e| DbError::from_sqlx(e, ""))?;

    let mut foreign_keys: Vec<ForeignKeyInfo> = Vec::new();
    for row in &fk_rows {
        let name: String = row.try_get("constraint_name").unwrap_or_default();
        let ref_schema: String = row.try_get("ref_schema").unwrap_or_default();
        let ref_table: String = row.try_get("ref_table").unwrap_or_default();
        let on_delete: String = row.try_get("on_delete").unwrap_or_default();
        let column: String = row.try_get("column_name").unwrap_or_default();
        let ref_column: String = row.try_get("ref_column").unwrap_or_default();

        let entry = match foreign_keys.last_mut() {
            Some(last)
                if last.name == name
                    && last.ref_schema == ref_schema
                    && last.ref_table == ref_table =>
            {
                last
            }
            _ => {
                foreign_keys.push(ForeignKeyInfo {
                    name: name.clone(),
                    columns: Vec::new(),
                    ref_schema: ref_schema.clone(),
                    ref_table: ref_table.clone(),
                    ref_columns: Vec::new(),
                    on_delete: on_delete.clone(),
                });
                foreign_keys.last_mut().expect("just pushed")
            }
        };
        entry.columns.push(column);
        entry.ref_columns.push(ref_column);
    }

    Ok(TableDetails {
        columns,
        indexes,
        foreign_keys,
    })
}

async fn mysql_table_details(
    pool: &sqlx::MySqlPool,
    schema: &str,
    table: &str,
) -> Result<TableDetails, DbError> {
    let columns = sqlx::query(
        r#"
        SELECT column_name, column_type, is_nullable, column_default,
               column_key, ordinal_position
        FROM information_schema.columns
        WHERE table_schema = ? AND table_name = ?
        ORDER BY ordinal_position
        "#,
    )
    .bind(schema)
    .bind(table)
    .fetch_all(pool)
    .await
    .map_err(|e| DbError::from_sqlx(e, ""))?
    .iter()
    .map(|row: &MySqlRow| ColumnInfo {
        name: row.try_get("column_name").unwrap_or_default(),
        data_type: row.try_get("column_type").unwrap_or_default(),
        nullable: row
            .try_get::<String, _>("is_nullable")
            .map(|s| s.eq_ignore_ascii_case("YES"))
            .unwrap_or(true),
        default: row
            .try_get::<Option<String>, _>("column_default")
            .unwrap_or_default(),
        key: row.try_get("column_key").unwrap_or_default(),
        ordinal: row.try_get("ordinal_position").unwrap_or_default(),
    })
    .collect();

    let indexes = sqlx::query(
        r#"
        SELECT index_name, non_unique, index_type,
               GROUP_CONCAT(COALESCE(column_name, '(expression)')
                            ORDER BY seq_in_index SEPARATOR ', ') AS columns
        FROM information_schema.statistics
        WHERE table_schema = ? AND table_name = ?
        GROUP BY index_name, non_unique, index_type
        ORDER BY (index_name = 'PRIMARY') DESC, index_name
        "#,
    )
    .bind(schema)
    .bind(table)
    .fetch_all(pool)
    .await
    .map_err(|e| DbError::from_sqlx(e, ""))?
    .iter()
    .map(|row: &MySqlRow| {
        let columns: String = row.try_get("columns").unwrap_or_default();
        let name: String = row.try_get("index_name").unwrap_or_default();
        let non_unique: i64 = row.try_get("non_unique").unwrap_or(1);
        IndexInfo {
            primary: name == "PRIMARY",
            name,
            unique: non_unique == 0,
            columns: split_columns(&columns),
            method: row.try_get("index_type").unwrap_or_default(),
        }
    })
    .collect();

    let fk_rows = sqlx::query(
        r#"
        SELECT k.constraint_name, k.column_name,
               k.referenced_table_schema, k.referenced_table_name,
               k.referenced_column_name, r.delete_rule
        FROM information_schema.key_column_usage k
        JOIN information_schema.referential_constraints r
          ON r.constraint_schema = k.constraint_schema
         AND r.constraint_name = k.constraint_name
        WHERE k.table_schema = ? AND k.table_name = ?
          AND k.referenced_table_name IS NOT NULL
        ORDER BY k.constraint_name, k.ordinal_position
        "#,
    )
    .bind(schema)
    .bind(table)
    .fetch_all(pool)
    .await
    .map_err(|e| DbError::from_sqlx(e, ""))?;

    let mut foreign_keys: Vec<ForeignKeyInfo> = Vec::new();
    for row in &fk_rows {
        let name: String = row.try_get("constraint_name").unwrap_or_default();
        let ref_schema: String = row.try_get("referenced_table_schema").unwrap_or_default();
        let ref_table: String = row.try_get("referenced_table_name").unwrap_or_default();
        let on_delete: String = row.try_get("delete_rule").unwrap_or_default();
        let column: String = row.try_get("column_name").unwrap_or_default();
        let ref_column: String = row.try_get("referenced_column_name").unwrap_or_default();

        let entry = match foreign_keys.last_mut() {
            Some(last)
                if last.name == name
                    && last.ref_schema == ref_schema
                    && last.ref_table == ref_table =>
            {
                last
            }
            _ => {
                foreign_keys.push(ForeignKeyInfo {
                    name: name.clone(),
                    columns: Vec::new(),
                    ref_schema: ref_schema.clone(),
                    ref_table: ref_table.clone(),
                    ref_columns: Vec::new(),
                    on_delete: on_delete.clone(),
                });
                foreign_keys.last_mut().expect("just pushed")
            }
        };
        entry.columns.push(column);
        entry.ref_columns.push(ref_column);
    }

    Ok(TableDetails {
        columns,
        indexes,
        foreign_keys,
    })
}

async fn sqlite_table_details(
    pool: &sqlx::SqlitePool,
    schema: &str,
    table: &str,
) -> Result<TableDetails, DbError> {
    let schema_id = sqlite_ident(schema);
    let table_id = sqlite_ident(table);

    // table_xinfo includes generated columns (hidden = 2/3) and virtual-table
    // hidden columns, so users can see everything the table exposes.
    let columns_sql = format!("PRAGMA {schema_id}.table_xinfo({table_id})");
    let columns = sqlx::query(&columns_sql)
        .fetch_all(pool)
        .await
        .map_err(|e| DbError::from_sqlx(e, &columns_sql))?
        .iter()
        .map(|row: &SqliteRow| {
            let cid: i64 = row.try_get("cid").unwrap_or_default();
            let pk: i64 = row.try_get("pk").unwrap_or_default();
            let notnull: i64 = row.try_get("notnull").unwrap_or_default();
            let mut data_type: String = row.try_get("type").unwrap_or_default();
            if data_type.trim().is_empty() {
                data_type = "ANY".to_string();
            }
            ColumnInfo {
                name: row.try_get("name").unwrap_or_default(),
                data_type,
                // SQLite does not mark INTEGER PRIMARY KEY columns as
                // NOT NULL in the pragma, but they cannot contain NULL.
                nullable: notnull == 0 && pk == 0,
                default: row
                    .try_get::<Option<String>, _>("dflt_value")
                    .unwrap_or_default(),
                key: if pk > 0 {
                    "PK".to_string()
                } else {
                    String::new()
                },
                ordinal: cid.saturating_add(1),
            }
        })
        .collect();

    let indexes_sql = format!("PRAGMA {schema_id}.index_list({table_id})");
    let index_rows = sqlx::query(&indexes_sql)
        .fetch_all(pool)
        .await
        .map_err(|e| DbError::from_sqlx(e, &indexes_sql))?;
    let mut indexes = Vec::with_capacity(index_rows.len());
    for row in &index_rows {
        let name: String = row.try_get("name").unwrap_or_default();
        if name.is_empty() {
            continue;
        }
        let origin: String = row.try_get("origin").unwrap_or_default();
        let info_sql = format!("PRAGMA {schema_id}.index_info({})", sqlite_ident(&name));
        let info_rows = sqlx::query(&info_sql)
            .fetch_all(pool)
            .await
            .map_err(|e| DbError::from_sqlx(e, &info_sql))?;
        let columns: Vec<String> = info_rows
            .iter()
            .map(|info| {
                info.try_get::<Option<String>, _>("name")
                    .ok()
                    .flatten()
                    .filter(|name| !name.is_empty())
                    .unwrap_or_else(|| "(expression)".to_string())
            })
            .collect();
        indexes.push(IndexInfo {
            name,
            unique: row.try_get::<i64, _>("unique").unwrap_or(0) != 0,
            primary: origin.eq_ignore_ascii_case("pk"),
            columns,
            method: "BTREE".to_string(),
        });
    }
    // The pragma returns indexes in creation order; show the primary key first
    // and keep the remaining names deterministic.
    indexes.sort_by(|a, b| {
        b.primary
            .cmp(&a.primary)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });

    let fk_sql = format!("PRAGMA {schema_id}.foreign_key_list({table_id})");
    let fk_rows = sqlx::query(&fk_sql)
        .fetch_all(pool)
        .await
        .map_err(|e| DbError::from_sqlx(e, &fk_sql))?;
    let mut foreign_keys: Vec<ForeignKeyInfo> = Vec::new();
    for row in &fk_rows {
        let id: i64 = row.try_get("id").unwrap_or_default();
        let name = format!("fk_{id}");
        let ref_table: String = row.try_get("table").unwrap_or_default();
        let column: String = row.try_get("from").unwrap_or_default();
        let ref_column = row
            .try_get::<Option<String>, _>("to")
            .ok()
            .flatten()
            .unwrap_or_default();
        let on_delete: String = row.try_get("on_delete").unwrap_or_default();

        let entry = match foreign_keys.last_mut() {
            Some(last) if last.name == name && last.ref_table == ref_table => last,
            _ => {
                foreign_keys.push(ForeignKeyInfo {
                    name: name.clone(),
                    columns: Vec::new(),
                    ref_schema: schema.to_string(),
                    ref_table: ref_table.clone(),
                    ref_columns: Vec::new(),
                    on_delete: on_delete.clone(),
                });
                foreign_keys.last_mut().expect("just pushed")
            }
        };
        entry.columns.push(column);
        entry.ref_columns.push(ref_column);
    }

    Ok(TableDetails {
        columns,
        indexes,
        foreign_keys,
    })
}

/// Quote an identifier for SQLite (including inside PRAGMA statements).
fn sqlite_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

fn split_columns(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn column_list_parsing() {
        assert_eq!(split_columns("a, b,c"), vec!["a", "b", "c"]);
        assert!(split_columns("").is_empty());
    }
}
