//! Connection establishment, metadata discovery and test connections.

use std::str::FromStr;
use std::time::{Duration, Instant};

use sqlx::mysql::{MySqlConnectOptions, MySqlPoolOptions, MySqlSslMode};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions, PgSslMode};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::Executor;

use super::error::DbError;
use super::{DbPool, Session, CONNECT_TIMEOUT, QUERY_ACQUIRE_TIMEOUT};
use crate::model::{ConnectionProfile, Driver, SessionInfo, SslMode, TestOutcome};

impl From<SslMode> for PgSslMode {
    fn from(mode: SslMode) -> Self {
        match mode {
            SslMode::Disable => PgSslMode::Disable,
            SslMode::Prefer => PgSslMode::Prefer,
            SslMode::Require => PgSslMode::Require,
            SslMode::VerifyCa => PgSslMode::VerifyCa,
            SslMode::VerifyFull => PgSslMode::VerifyFull,
        }
    }
}

impl From<SslMode> for MySqlSslMode {
    fn from(mode: SslMode) -> Self {
        match mode {
            SslMode::Disable => MySqlSslMode::Disabled,
            SslMode::Prefer => MySqlSslMode::Preferred,
            SslMode::Require => MySqlSslMode::Required,
            SslMode::VerifyCa => MySqlSslMode::VerifyCa,
            // MySQL's "Identity" maps to hostname verification.
            SslMode::VerifyFull => MySqlSslMode::VerifyIdentity,
        }
    }
}

/// Establish a session, including a small metadata round-trip.
pub async fn connect(profile: &ConnectionProfile, session_id: u64) -> Result<Session, DbError> {
    profile.validate().map_err(DbError::new)?;
    let started = Instant::now();

    match profile.driver {
        Driver::Postgres => connect_postgres(profile, session_id, started).await,
        Driver::MySql => connect_mysql(profile, session_id, started).await,
        Driver::Sqlite => connect_sqlite(profile, session_id, started).await,
    }
}

async fn connect_postgres(
    profile: &ConnectionProfile,
    session_id: u64,
    started: Instant,
) -> Result<Session, DbError> {
    let options = PgConnectOptions::new()
        .host(profile.host.trim())
        .port(profile.port)
        .username(profile.user.trim())
        .password(profile.password.as_str())
        .database(profile.database.trim())
        .ssl_mode(profile.ssl_mode.into())
        .application_name("rustquery");

    let pool = PgPoolOptions::new()
        .max_connections(super::pool_size())
        .min_connections(0)
        .acquire_timeout(CONNECT_TIMEOUT)
        .idle_timeout(Duration::from_secs(600))
        .max_lifetime(Duration::from_secs(1800))
        .test_before_acquire(true)
        .connect_with(options.clone())
        .await
        .map_err(|e| DbError::from_sqlx(e, ""))?;

    // A single-connection pool for transactions.
    let tx_pool = PgPoolOptions::new()
        .max_connections(1)
        .min_connections(0)
        .acquire_timeout(QUERY_ACQUIRE_TIMEOUT)
        .idle_timeout(None)
        .max_lifetime(None)
        .connect_with(options)
        .await
        .map_err(|e| DbError::from_sqlx(e, ""))?;

    let mut conn = pool
        .acquire()
        .await
        .map_err(|e| DbError::from_sqlx(e, ""))?;
    let version = query_scalar_pg(&mut conn, "SELECT version()").await;
    let user = query_scalar_pg(&mut conn, "SELECT current_user").await;
    let database = query_scalar_pg(&mut conn, "SELECT current_database()").await;
    drop(conn);

    let ping_start = Instant::now();
    pool.execute("SELECT 1")
        .await
        .map_err(|e| DbError::from_sqlx(e, ""))?;
    let ping_ms = ping_start.elapsed().as_millis() as u64;

    let info = SessionInfo {
        profile_id: profile.id,
        profile_name: profile.name.clone(),
        driver: Driver::Postgres,
        server_version: clean_version(&version),
        current_user: user,
        current_database: database,
        latency_ms: ping_ms.max(started.elapsed().as_millis() as u64 / 10),
    };

    Ok(Session {
        id: session_id,
        profile: profile.clone(),
        info,
        pool: DbPool::Postgres(pool),
        tx_pool: DbPool::Postgres(tx_pool),
        tx_active: false,
    })
}

async fn connect_mysql(
    profile: &ConnectionProfile,
    session_id: u64,
    started: Instant,
) -> Result<Session, DbError> {
    let options = MySqlConnectOptions::new()
        .host(profile.host.trim())
        .port(profile.port)
        .username(profile.user.trim())
        .password(profile.password.as_str())
        .database(profile.database.trim())
        .ssl_mode(profile.ssl_mode.into())
        .charset("utf8mb4");

    let pool = MySqlPoolOptions::new()
        .max_connections(super::pool_size())
        .min_connections(0)
        .acquire_timeout(CONNECT_TIMEOUT)
        .idle_timeout(Duration::from_secs(600))
        .max_lifetime(Duration::from_secs(1800))
        .test_before_acquire(true)
        .connect_with(options.clone())
        .await
        .map_err(|e| DbError::from_sqlx(e, ""))?;

    let tx_pool = MySqlPoolOptions::new()
        .max_connections(1)
        .min_connections(0)
        .acquire_timeout(QUERY_ACQUIRE_TIMEOUT)
        .idle_timeout(None)
        .max_lifetime(None)
        .connect_with(options)
        .await
        .map_err(|e| DbError::from_sqlx(e, ""))?;

    let mut conn = pool
        .acquire()
        .await
        .map_err(|e| DbError::from_sqlx(e, ""))?;
    let version = query_scalar_mysql(&mut conn, "SELECT VERSION()").await;
    let user = query_scalar_mysql(&mut conn, "SELECT CURRENT_USER()").await;
    let database = query_scalar_mysql(&mut conn, "SELECT COALESCE(DATABASE(), '')").await;
    drop(conn);

    let ping_start = Instant::now();
    pool.execute("SELECT 1")
        .await
        .map_err(|e| DbError::from_sqlx(e, ""))?;
    let ping_ms = ping_start.elapsed().as_millis() as u64;

    let info = SessionInfo {
        profile_id: profile.id,
        profile_name: profile.name.clone(),
        driver: Driver::MySql,
        server_version: clean_version(&version),
        current_user: user,
        current_database: database,
        latency_ms: ping_ms.max(started.elapsed().as_millis() as u64 / 10),
    };

    Ok(Session {
        id: session_id,
        profile: profile.clone(),
        info,
        pool: DbPool::MySql(pool),
        tx_pool: DbPool::MySql(tx_pool),
        tx_active: false,
    })
}

/// Open a SQLite database file (or an in-memory database).
///
/// Unlike the client/server engines, a SQLite session deliberately uses a
/// single-connection pool for both normal work and transactions. SQLite has no
/// server-side locking beyond the file lock, and every `BEGIN`/`COMMIT` must
/// happen on the same physical connection. Sharing one pool also keeps
/// `:memory:` databases consistent across schema discovery, queries and
/// transactions instead of silently creating separate databases per
/// connection.
async fn connect_sqlite(
    profile: &ConnectionProfile,
    session_id: u64,
    started: Instant,
) -> Result<Session, DbError> {
    let options = sqlite_options(profile)?;
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .min_connections(0)
        .acquire_timeout(CONNECT_TIMEOUT)
        // Closing the only connection to an in-memory database would discard
        // it, and file databases benefit from keeping the warm connection.
        .idle_timeout(None)
        .max_lifetime(None)
        .test_before_acquire(true)
        .connect_with(options)
        .await
        .map_err(|e| DbError::from_sqlx(e, ""))?;

    let version = sqlx::query_scalar::<_, String>("SELECT sqlite_version()")
        .fetch_one(&pool)
        .await
        .unwrap_or_else(|_| "unknown".to_string());

    let ping_start = Instant::now();
    sqlx::query("SELECT 1")
        .execute(&pool)
        .await
        .map_err(|e| DbError::from_sqlx(e, ""))?;
    let ping_ms = ping_start.elapsed().as_millis() as u64;

    let info = SessionInfo {
        profile_id: profile.id,
        profile_name: profile.name.clone(),
        driver: Driver::Sqlite,
        server_version: format!("SQLite {version}"),
        current_user: "local".to_string(),
        current_database: profile.endpoint(),
        latency_ms: ping_ms.max(started.elapsed().as_millis() as u64 / 10),
    };

    Ok(Session {
        id: session_id,
        profile: profile.clone(),
        info,
        // Both handles point at the same single-connection pool. See the note
        // above: this is essential for transactions and :memory: databases.
        pool: DbPool::Sqlite(pool.clone()),
        tx_pool: DbPool::Sqlite(pool),
        tx_active: false,
    })
}

/// Build SQLite connection options from the profile's database field.
///
/// Accepted forms:
/// * `:memory:` — a private temporary database;
/// * `sqlite::memory:` / `sqlite://…` — a SQLx connection URL;
/// * any filesystem path — opened read/write, created when missing.
fn sqlite_options(profile: &ConnectionProfile) -> Result<SqliteConnectOptions, DbError> {
    let raw = profile.database.trim();
    if raw.is_empty() {
        return Err(DbError::new("SQLite database file cannot be empty."));
    }
    let mut options = if raw == ":memory:" {
        SqliteConnectOptions::new().in_memory(true)
    } else if raw.starts_with("sqlite:") {
        SqliteConnectOptions::from_str(raw)
            .map_err(|e| DbError::new(format!("Invalid SQLite connection string: {e}")))?
    } else {
        SqliteConnectOptions::new()
            .filename(raw)
            .create_if_missing(true)
    };
    // These are the SQLx defaults, but state them explicitly: foreign-key
    // enforcement and waiting for short-lived file locks matter in a GUI.
    options = options
        .foreign_keys(true)
        .busy_timeout(Duration::from_secs(5));
    Ok(options)
}

async fn query_scalar_pg(conn: &mut sqlx::PgConnection, sql: &str) -> String {
    sqlx::query_scalar::<_, String>(sql)
        .fetch_one(&mut *conn)
        .await
        .unwrap_or_else(|_| "unknown".to_string())
}

async fn query_scalar_mysql(conn: &mut sqlx::MySqlConnection, sql: &str) -> String {
    sqlx::query_scalar::<_, String>(sql)
        .fetch_one(&mut *conn)
        .await
        .unwrap_or_else(|_| "unknown".to_string())
}

/// Version strings often start with "PostgreSQL 16.2 on x86_64…". Keep the
/// useful prefix for the status bar.
fn clean_version(version: &str) -> String {
    let first = version.lines().next().unwrap_or(version).trim();
    if first.chars().count() > 96 {
        let prefix: String = first.chars().take(95).collect();
        format!("{prefix}…")
    } else {
        first.to_string()
    }
}

/// Verify credentials and return metadata without keeping a session open.
pub async fn test_connection(profile: &ConnectionProfile) -> Result<TestOutcome, DbError> {
    let session = connect(profile, 0).await?;
    let outcome = TestOutcome {
        latency_ms: session.info.latency_ms,
        server_version: session.info.server_version.clone(),
        user: session.info.current_user.clone(),
    };
    // Dropping the clone closes the pools in the background; the worker does
    // not need to keep the test connection alive.
    super::close_pool(session.pool.clone()).await;
    super::close_pool(session.tx_pool.clone()).await;
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssl_modes_map() {
        let _ = PgSslMode::from(SslMode::Require);
        let _ = MySqlSslMode::from(SslMode::VerifyFull);
    }

    #[test]
    fn version_is_cleaned() {
        assert_eq!(
            clean_version("PostgreSQL 16.2 on x86_64"),
            "PostgreSQL 16.2 on x86_64"
        );
    }
}
