use rusqlite::Connection;
use senix_core::{Error, SQLITE_SCHEMA_VERSION, SqliteStateStore};

#[test]
fn writable_store_sets_a_versioned_wal_schema() {
    let directory = tempfile::tempdir().unwrap();
    let db = directory.path().join("state.db");
    let store = SqliteStateStore::open(&db).unwrap();
    assert_eq!(
        store.database_status().unwrap().schema_version,
        SQLITE_SCHEMA_VERSION
    );
    drop(store);

    let connection = Connection::open(&db).unwrap();
    let journal_mode: String = connection
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .unwrap();
    let schema_version: i64 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(journal_mode.to_ascii_lowercase(), "wal");
    assert_eq!(schema_version, SQLITE_SCHEMA_VERSION);
}

#[test]
fn writable_store_refuses_a_newer_schema_without_mutating_it() {
    let directory = tempfile::tempdir().unwrap();
    let db = directory.path().join("future.db");
    let future_version = SQLITE_SCHEMA_VERSION + 1;
    let connection = Connection::open(&db).unwrap();
    connection
        .pragma_update(None, "user_version", future_version)
        .unwrap();
    drop(connection);

    assert!(matches!(
        SqliteStateStore::open(&db),
        Err(Error::InvalidState(_))
    ));
    let connection = Connection::open(&db).unwrap();
    let actual: i64 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(actual, future_version);
}

#[test]
fn read_only_store_never_upgrades_an_unversioned_file() {
    let directory = tempfile::tempdir().unwrap();
    let db = directory.path().join("old.db");
    drop(Connection::open(&db).unwrap());

    assert!(matches!(
        SqliteStateStore::open_read_only(&db),
        Err(Error::InvalidState(_))
    ));
    let connection = Connection::open(&db).unwrap();
    let actual: i64 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(actual, 0);
}
