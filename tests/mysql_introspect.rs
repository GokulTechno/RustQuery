//! MySQL schema introspection against a local server (see `#[ignore]` note).

use rustquery::db;
use rustquery::model::{ConnectionProfile, Driver};

/// Run manually when MySQL is listening on `127.0.0.1:3307` with root/testpass
/// and database `myapp` containing a `users` table.
#[tokio::test]
#[ignore = "requires MySQL on localhost:3307 (e.g. docker run -p 3307:3306 ...)"]
async fn mysql_lists_user_databases() {
    let mut profile = ConnectionProfile::new(Driver::MySql).with_defaults();
    profile.host = "127.0.0.1".to_string();
    profile.port = 3307;
    profile.user = "root".to_string();
    profile.password = "testpass".to_string();
    profile.database = "myapp".to_string();

    let session = db::connect(&profile, 1).await.expect("connect");
    let schemas = db::introspect::list_schemas(&session.pool)
        .await
        .expect("list schemas");
    assert!(
        schemas.iter().any(|s| s.name == "myapp"),
        "expected myapp schema, got {:?}",
        schemas.iter().map(|s| s.name.clone()).collect::<Vec<_>>()
    );
    let myapp = schemas.iter().find(|s| s.name == "myapp").expect("myapp");
    assert!(
        myapp.objects.iter().any(|o| o.name == "users"),
        "expected users table, got {:?}",
        myapp.objects
    );

    let details = db::introspect::table_details(&session.pool, "myapp", "users")
        .await
        .expect("table details");
    assert!(
        details.columns.iter().any(|c| c.name == "id"),
        "expected id column, got {:?}",
        details.columns
    );

    db::close_pool(session.pool).await;
}
