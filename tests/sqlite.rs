//! End-to-end SQLite tests.
//!
//! SQLite is embedded, so unlike PostgreSQL/MySQL these tests run a real
//! database without any external service. They cover connecting, multi-statement
//! execution, transaction state, dynamic value rendering and introspection.

use std::path::PathBuf;
use std::time::Duration;

use rustquery::db;
use rustquery::model::{ConnectionProfile, Driver, ObjectKind};

fn temp_db() -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    std::env::temp_dir().join(format!(
        "rustquery-sqlite-{}-{nanos}.db",
        std::process::id()
    ))
}

#[tokio::test]
async fn sqlite_end_to_end() {
    let path = temp_db();
    let mut profile = ConnectionProfile::new(Driver::Sqlite).with_defaults();
    profile.name = "Test SQLite".to_string();
    profile.database = path.to_string_lossy().into_owned();
    assert!(profile.validate().is_ok());

    let session = db::connect(&profile, 7)
        .await
        .expect("SQLite should open the temporary database");
    assert_eq!(session.info.driver, Driver::Sqlite);
    assert!(session.info.server_version.starts_with("SQLite"));
    assert_eq!(session.info.current_user, "local");

    let timeout = Duration::from_secs(20);
    let schema = r#"
        CREATE TABLE users (
            id INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            email TEXT UNIQUE,
            active BOOLEAN DEFAULT 1
        );
        CREATE TABLE posts (
            id INTEGER PRIMARY KEY,
            user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
            title TEXT NOT NULL
        );
        CREATE INDEX idx_users_name ON users(name);
        CREATE VIEW active_users AS SELECT id, name FROM users WHERE active = 1;
        INSERT INTO users (name, email, active)
        VALUES ('Alice', 'alice@example.com', 1), ('Bob', NULL, 0);
        INSERT INTO posts (user_id, title) VALUES (1, 'Hello'), (1, 'World');
    "#;
    let created = db::execute::execute(&session.pool, schema, 200, timeout)
        .await
        .expect("schema script");
    assert!(created.columns.is_empty());
    assert_eq!(created.affected, 4);

    // Dynamic values, NULLs and expressions.
    let result = db::execute::execute(
        &session.pool,
        "SELECT id, name, email, active, id * 10 AS ten FROM users ORDER BY id;",
        200,
        timeout,
    )
    .await
    .expect("select users");
    let headers: Vec<&str> = result.columns.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(headers, vec!["id", "name", "email", "active", "ten"]);
    assert_eq!(result.rows.len(), 2);
    assert_eq!(result.rows[0][0].as_deref(), Some("1"));
    assert_eq!(result.rows[0][1].as_deref(), Some("Alice"));
    assert_eq!(result.rows[0][2].as_deref(), Some("alice@example.com"));
    assert_eq!(result.rows[1][2], None);
    assert_eq!(result.rows[0][4].as_deref(), Some("10"));

    // BLOB rendering is bounded and readable.
    db::execute::execute(
        &session.pool,
        "CREATE TABLE blobs (data BLOB); INSERT INTO blobs VALUES (x'DEADBEEF');",
        10,
        timeout,
    )
    .await
    .expect("blob script");
    let blobs = db::execute::execute(&session.pool, "SELECT data FROM blobs;", 10, timeout)
        .await
        .expect("blob select");
    assert_eq!(blobs.rows[0][0].as_deref(), Some("\\xdeadbeef"));

    // Transaction statements run on the shared SQLite connection, and script
    // state tracking notices that the final statement closes the transaction.
    let script = "BEGIN; INSERT INTO users (name, active) VALUES ('Carol', 1); COMMIT;";
    assert!(db::execute::is_tx_statement(script));
    assert_eq!(db::execute::tx_state_after(script), Some(false));
    db::execute::execute(&session.tx_pool, script, 10, timeout)
        .await
        .expect("transaction script");
    let count = db::execute::execute(
        &session.pool,
        "SELECT COUNT(*) AS n FROM users;",
        10,
        timeout,
    )
    .await
    .expect("count users");
    assert_eq!(count.rows[0][0].as_deref(), Some("3"));

    // An open transaction is rolled back explicitly.
    let open = "BEGIN; INSERT INTO users (name, active) VALUES ('Rolled back', 1);";
    assert_eq!(db::execute::tx_state_after(open), Some(true));
    db::execute::execute(&session.tx_pool, open, 10, timeout)
        .await
        .expect("begin script");
    db::execute::execute(&session.tx_pool, "ROLLBACK;", 10, timeout)
        .await
        .expect("rollback");
    let count = db::execute::execute(
        &session.pool,
        "SELECT COUNT(*) AS n FROM users;",
        10,
        timeout,
    )
    .await
    .expect("count after rollback");
    assert_eq!(count.rows[0][0].as_deref(), Some("3"));

    // Schema discovery and lazy table details.
    let schemas = db::introspect::list_schemas(&session.pool)
        .await
        .expect("list schemas");
    let main = schemas
        .iter()
        .find(|schema| schema.name == "main")
        .expect("main schema");
    assert!(main
        .objects
        .iter()
        .any(|object| object.name == "users" && object.kind == ObjectKind::Table));
    assert!(main
        .objects
        .iter()
        .any(|object| object.name == "active_users" && object.kind == ObjectKind::View));
    assert!(!main
        .objects
        .iter()
        .any(|object| object.name == "sqlite_sequence"));

    let users = db::introspect::table_details(&session.pool, "main", "users")
        .await
        .expect("users details");
    let id = users
        .columns
        .iter()
        .find(|column| column.name == "id")
        .expect("id column");
    assert_eq!(id.key, "PK");
    assert!(!id.nullable);
    assert!(users
        .indexes
        .iter()
        .any(|index| index.name == "idx_users_name"
            && !index.unique
            && index.columns == vec!["name".to_string()]));
    assert!(users
        .indexes
        .iter()
        .any(|index| index.unique && index.name.starts_with("sqlite_autoindex")));

    let posts = db::introspect::table_details(&session.pool, "main", "posts")
        .await
        .expect("posts details");
    let fk = posts.foreign_keys.first().expect("foreign key");
    assert_eq!(fk.ref_table, "users");
    assert_eq!(fk.ref_schema, "main");
    assert_eq!(fk.columns, vec!["user_id".to_string()]);
    assert_eq!(fk.ref_columns, vec!["id".to_string()]);
    assert_eq!(fk.on_delete, "CASCADE");

    // Test-connection path.
    let outcome = db::connect::test_connection(&profile)
        .await
        .expect("test connection");
    assert!(outcome.server_version.starts_with("SQLite"));

    // Close the pool before deleting the file.
    let pool = session.pool.clone();
    drop(session);
    db::close_pool(pool).await;
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
}

#[tokio::test]
async fn sqlite_in_memory_and_errors() {
    let mut profile = ConnectionProfile::new(Driver::Sqlite).with_defaults();
    profile.database = ":memory:".to_string();
    let session = db::connect(&profile, 1).await.expect("in-memory SQLite");
    assert_eq!(session.info.current_database, "in-memory database");

    db::execute::execute(
        &session.pool,
        "CREATE TABLE t (id INTEGER PRIMARY KEY, value TEXT); INSERT INTO t VALUES (1, 'x');",
        10,
        Duration::from_secs(10),
    )
    .await
    .expect("create in-memory schema");

    // The single shared pool must see the same in-memory database from both
    // handles, including schema introspection.
    let result = db::execute::execute(
        &session.tx_pool,
        "SELECT value FROM t;",
        10,
        Duration::from_secs(10),
    )
    .await
    .expect("read through tx handle");
    assert_eq!(result.rows[0][0].as_deref(), Some("x"));
    let schemas = db::introspect::list_schemas(&session.pool)
        .await
        .expect("in-memory schemas");
    assert!(schemas
        .iter()
        .any(|schema| schema.name == "main" && schema.objects.iter().any(|o| o.name == "t")));

    let error = db::execute::execute(
        &session.pool,
        "SELECT * FROM missing_table;",
        10,
        Duration::from_secs(10),
    )
    .await
    .expect_err("missing table must fail");
    assert!(error.summary.to_ascii_lowercase().contains("no such table"));
}
