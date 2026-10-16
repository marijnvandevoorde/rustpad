//! Tests to ensure that documents are persisted with SQLite (and MySQL, when
//! `TEST_MYSQL_URL` points at a disposable MySQL database).

use std::time::Duration;

use anyhow::Result;
use common::*;
use operational_transform::OperationSeq;
use rustpad_server::{
    database::{Database, PersistedDocument},
    server, server_with_flusher, ServerConfig,
};
use serde_json::json;
use tempfile::NamedTempFile;
use tokio::time;

pub mod common;

fn temp_sqlite_uri() -> Result<String> {
    Ok(format!(
        "sqlite://{}",
        NamedTempFile::new()?
            .into_temp_path()
            .as_os_str()
            .to_str()
            .expect("failed to get name of tempfile as &str")
    ))
}

#[tokio::test]
async fn test_database() -> Result<()> {
    pretty_env_logger::try_init().ok();
    check_database(&temp_sqlite_uri()?, "").await
}

#[tokio::test]
async fn test_database_mysql() -> Result<()> {
    pretty_env_logger::try_init().ok();
    let Ok(uri) = std::env::var("TEST_MYSQL_URL") else {
        eprintln!("TEST_MYSQL_URL not set, skipping");
        return Ok(());
    };
    // Unique ids, so reruns against the same database do not collide.
    let prefix = format!("test-{}-", rand::random::<u32>());
    check_database(&uri, &prefix).await?;

    let database = Database::new(&uri).await?;
    let unicode = PersistedDocument {
        text: "h\u{e9}llo \u{1f600} \u{4e16}\u{754c}".into(),
        language: Some("markdown".into()),
    };
    let id = format!("{prefix}unicode");
    database.store(&id, &unicode).await?;
    // Storing an identical document again must not fail.
    database.store(&id, &unicode).await?;
    assert_eq!(database.load(&id).await?, unicode);
    // Document ids are case-sensitive.
    let upper = PersistedDocument {
        text: "upper".into(),
        language: None,
    };
    database.store(&format!("{prefix}CASE"), &upper).await?;
    assert!(database.load(&format!("{prefix}case")).await.is_err());
    assert_eq!(database.load(&format!("{prefix}CASE")).await?, upper);
    assert!(database.count().await? >= 4);
    Ok(())
}

async fn check_database(uri: &str, prefix: &str) -> Result<()> {
    let database = Database::new(uri).await?;
    let hello = format!("{prefix}hello");
    let world = format!("{prefix}world");

    assert!(database.load(&hello).await.is_err());
    assert!(database.load(&world).await.is_err());

    let doc1 = PersistedDocument {
        text: "Hello Text".into(),
        language: None,
    };

    assert!(database.store(&hello, &doc1).await.is_ok());
    assert_eq!(database.load(&hello).await?, doc1);
    assert!(database.load(&world).await.is_err());

    let doc2 = PersistedDocument {
        text: "print('World Text :)')".into(),
        language: Some("python".into()),
    };

    assert!(database.store(&world, &doc2).await.is_ok());
    assert_eq!(database.load(&hello).await?, doc1);
    assert_eq!(database.load(&world).await?, doc2);

    assert!(database.store(&hello, &doc2).await.is_ok());
    assert_eq!(database.load(&hello).await?, doc2);

    Ok(())
}

#[tokio::test]
async fn test_persist() -> Result<()> {
    pretty_env_logger::try_init().ok();

    let filter = server(ServerConfig {
        expiry_days: 2,
        database: Some(Database::new(&temp_sqlite_uri()?).await?),
    });

    expect_text(&filter, "persist", "").await;

    let mut client = connect(&filter, "persist").await?;
    let msg = client.recv().await?;
    assert_eq!(msg, json!({ "Identity": 0 }));

    let mut operation = OperationSeq::default();
    operation.insert("hello");
    let msg = json!({
        "Edit": {
            "revision": 0,
            "operation": operation
        }
    });
    client.send(&msg).await;

    let msg = client.recv().await?;
    msg.get("History")
        .expect("should receive history operation");
    expect_text(&filter, "persist", "hello").await;

    let hour = Duration::from_secs(3600);
    time::pause();
    time::advance(47 * hour).await;
    expect_text(&filter, "persist", "hello").await;

    // Give SQLite some time to actually update the database.
    time::resume();
    time::sleep(Duration::from_millis(150)).await;
    time::pause();

    time::advance(3 * hour).await;
    expect_text(&filter, "persist", "hello").await;

    Ok(())
}

#[tokio::test]
async fn test_persist_language() -> Result<()> {
    pretty_env_logger::try_init().ok();

    let database = Database::new(&temp_sqlite_uri()?).await?;
    let document = PersistedDocument {
        text: "fn main() {}".into(),
        language: None,
    };
    database.store("lang", &document).await?;

    let filter = server(ServerConfig {
        expiry_days: 2,
        database: Some(database.clone()),
    });

    let mut client = connect(&filter, "lang").await?;
    let msg = client.recv().await?;
    assert_eq!(msg, json!({ "Identity": 0 }));

    // Only change the language, without editing the text.
    client.send(&json!({ "SetLanguage": "rust" })).await;
    client.recv().await?; // History (the loaded text)
    let msg = client.recv().await?;
    assert_eq!(msg, json!({ "Language": "rust" }));

    // Wait for the persister (every 3-4 seconds).
    time::sleep(Duration::from_millis(4500)).await;
    let stored = database.load("lang").await?;
    assert_eq!(stored.text, "fn main() {}");
    assert_eq!(stored.language.as_deref(), Some("rust"));

    Ok(())
}

#[tokio::test]
async fn test_flush() -> Result<()> {
    pretty_env_logger::try_init().ok();

    let database = Database::new(&temp_sqlite_uri()?).await?;
    let (filter, flusher) = server_with_flusher(ServerConfig {
        expiry_days: 2,
        database: Some(database.clone()),
    });

    // An untouched document is not stored.
    let mut empty = connect(&filter, "empty").await?;
    assert_eq!(empty.recv().await?, json!({ "Identity": 0 }));

    let mut client = connect(&filter, "flush").await?;
    assert_eq!(client.recv().await?, json!({ "Identity": 0 }));
    let mut operation = OperationSeq::default();
    operation.insert("unsaved");
    client
        .send(&json!({ "Edit": { "revision": 0, "operation": operation } }))
        .await;
    client.recv().await?.get("History").expect("history");

    // Flush right away, before the periodic persister has run.
    assert!(database.load("flush").await.is_err());
    assert_eq!(flusher.flush().await, 1);
    assert_eq!(database.load("flush").await?.text, "unsaved");
    assert!(database.load("empty").await.is_err());

    // Connections to flushed documents are closed.
    client.recv_closed().await?;
    Ok(())
}
