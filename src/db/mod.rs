//! Database abstraction layer.
//!
//! RustQuery uses SQLx's native PostgreSQL, MySQL and SQLite drivers. The pool
//! types are wrapped in a small enum so the rest of the application can treat
//! them uniformly while still receiving precise column type metadata.

pub mod connect;
pub mod error;
pub mod execute;
pub mod introspect;
pub mod value;

use std::time::Duration;

use sqlx::mysql::MySqlPool;
use sqlx::postgres::PgPool;
use sqlx::sqlite::SqlitePool;
use sqlx::Pool;

pub use connect::connect;
pub use error::DbError;

use crate::model::{ConnectionProfile, Driver, SessionInfo};

/// A cloneable handle to either supported pool. SQLx pools are internally
/// reference-counted, so cloning is cheap and safe to move into worker tasks.
#[derive(Clone)]
pub enum DbPool {
    Postgres(PgPool),
    MySql(MySqlPool),
    Sqlite(SqlitePool),
}

impl DbPool {
    pub fn driver(&self) -> Driver {
        match self {
            DbPool::Postgres(_) => Driver::Postgres,
            DbPool::MySql(_) => Driver::MySql,
            DbPool::Sqlite(_) => Driver::Sqlite,
        }
    }
}

/// Everything required to run statements and describe the session.
///
/// `pool` is used for normal (auto-commit) queries. `tx_pool` is deliberately
/// capped at one connection: BEGIN/COMMIT/ROLLBACK and all statements while a
/// transaction is open are routed through it, guaranteeing they share the same
/// physical session without borrowing a `Transaction` across async UI tasks.
#[derive(Clone)]
pub struct Session {
    pub id: u64,
    pub profile: ConnectionProfile,
    pub info: SessionInfo,
    pub pool: DbPool,
    pub tx_pool: DbPool,
    pub tx_active: bool,
}

impl Session {
    pub fn driver(&self) -> Driver {
        self.info.driver
    }

    pub fn pool_for_statement(&self, statement_is_transaction_control: bool) -> &DbPool {
        if self.tx_active || statement_is_transaction_control {
            &self.tx_pool
        } else {
            &self.pool
        }
    }
}

/// Pool configuration shared by both drivers.
pub(crate) const CONNECT_TIMEOUT: Duration = Duration::from_secs(12);
pub(crate) const QUERY_ACQUIRE_TIMEOUT: Duration = Duration::from_secs(15);

/// Generic helper for closing a pool from an owned clone without blocking the
/// UI thread on shutdown.
pub async fn close_pool(pool: DbPool) {
    match pool {
        DbPool::Postgres(p) => {
            let _ = tokio::time::timeout(Duration::from_secs(2), p.close()).await;
        }
        DbPool::MySql(p) => {
            let _ = tokio::time::timeout(Duration::from_secs(2), p.close()).await;
        }
        DbPool::Sqlite(p) => {
            let _ = tokio::time::timeout(Duration::from_secs(2), p.close()).await;
        }
    }
}

pub(crate) fn pool_size() -> u32 {
    6
}

// Silence an unused-import warning when compiling with certain feature sets.
#[allow(dead_code)]
fn _assert_pool_is_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Pool<sqlx::Postgres>>();
    assert_send_sync::<Pool<sqlx::MySql>>();
    assert_send_sync::<Pool<sqlx::Sqlite>>();
}
