//! Backend database handlers for persisting documents, in SQLite or MySQL.

use std::str::FromStr;

use anyhow::{Result, bail};
use sqlx::{ConnectOptions, MySqlPool, SqlitePool, sqlite::SqliteConnectOptions};

/// Represents a document persisted in database storage.
#[derive(sqlx::FromRow, PartialEq, Eq, Clone, Debug)]
pub struct PersistedDocument {
    /// Text content of the document.
    pub text: String,
    /// Language of the document for editor syntax highlighting.
    pub language: Option<String>,
}

/// A row of the document list, without the full text.
#[derive(sqlx::FromRow, serde::Serialize, PartialEq, Eq, Clone, Debug)]
pub struct DocumentSummary {
    /// Document id, as used in the URL.
    pub id: String,
    /// Title of the document, if one was set.
    pub title: Option<String>,
    /// Language of the document.
    pub language: Option<String>,
    /// Whether the document is locked (read-only).
    pub locked: bool,
    /// Last save in UTC (`YYYY-MM-DDTHH:MM:SSZ`), unknown for documents saved
    /// before this was recorded.
    pub updated_at: Option<String>,
    /// The start of the text.
    pub preview: String,
}

#[derive(Clone, Debug)]
enum Pool {
    Sqlite(SqlitePool),
    MySql(MySqlPool),
}

/// A driver for database operations wrapping a pool connection.
#[derive(Clone, Debug)]
pub struct Database {
    pool: Pool,
}

const MYSQL_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS document (
    id VARCHAR(255) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL PRIMARY KEY,
    text LONGTEXT NOT NULL,
    language VARCHAR(255)
) DEFAULT CHARSET = utf8mb4"#;

const MYSQL_VERSIONS: &str = r#"
CREATE TABLE IF NOT EXISTS document_version (
    id BIGINT NOT NULL AUTO_INCREMENT PRIMARY KEY,
    doc_id VARCHAR(255) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,
    text LONGTEXT NOT NULL,
    language VARCHAR(255),
    reason VARCHAR(32) NOT NULL,
    created_at DATETIME NOT NULL,
    INDEX document_version_doc (doc_id, created_at)
) DEFAULT CHARSET = utf8mb4"#;

/// A saved version of a document, without its text.
#[derive(sqlx::FromRow, serde::Serialize, PartialEq, Eq, Clone, Debug)]
pub struct VersionSummary {
    /// Version id.
    pub id: i64,
    /// When it was taken, in UTC (`YYYY-MM-DDTHH:MM:SSZ`).
    pub created_at: String,
    /// Why it was taken: "idle", "session start", "lock", "before restore"
    /// or "shutdown".
    pub reason: String,
    /// Language at that time.
    pub language: Option<String>,
    /// Length of the text in characters.
    pub size: i64,
}

/// A saved version of a document.
#[derive(sqlx::FromRow, serde::Serialize, PartialEq, Eq, Clone, Debug)]
pub struct Version {
    /// Version id.
    pub id: i64,
    /// When it was taken, in UTC (`YYYY-MM-DDTHH:MM:SSZ`).
    pub created_at: String,
    /// Why it was taken.
    pub reason: String,
    /// Language at that time.
    pub language: Option<String>,
    /// Text at that time.
    pub text: String,
}

/// Columns added after the first release, created when missing.
const MYSQL_ADDED_COLUMNS: &[(&str, &str)] = &[
    (
        "title",
        "ALTER TABLE document ADD COLUMN title VARCHAR(255) NULL",
    ),
    (
        "updated_at",
        "ALTER TABLE document ADD COLUMN updated_at DATETIME NULL",
    ),
    (
        "locked",
        "ALTER TABLE document ADD COLUMN locked BOOLEAN NOT NULL DEFAULT FALSE",
    ),
];

/// Characters of text returned as a preview in the document list.
const PREVIEW_CHARS: i64 = 100;

impl Database {
    /// Construct a new database from a SQLite or MySQL connection URI.
    ///
    /// A `mysql://` URI connects to MySQL (TLS is controlled by its `ssl-mode`
    /// parameter); anything else is treated as SQLite.
    pub async fn new(uri: &str) -> Result<Self> {
        if uri.starts_with("mysql://") {
            let pool = MySqlPool::connect(uri).await?;
            sqlx::query(MYSQL_SCHEMA).execute(&pool).await?;
            sqlx::query(MYSQL_VERSIONS).execute(&pool).await?;
            for (column, ddl) in MYSQL_ADDED_COLUMNS {
                let (exists,): (i64,) = sqlx::query_as(
                    "SELECT count(*) FROM information_schema.columns WHERE table_schema = \
                     DATABASE() AND table_name = 'document' AND column_name = ?",
                )
                .bind(column)
                .fetch_one(&pool)
                .await?;
                if exists == 0 {
                    sqlx::query(*ddl).execute(&pool).await?;
                }
            }
            return Ok(Database {
                pool: Pool::MySql(pool),
            });
        }
        {
            // Create database file if missing, and run migrations.
            let mut conn = SqliteConnectOptions::from_str(uri)?
                .create_if_missing(true)
                .connect()
                .await?;
            sqlx::migrate!().run(&mut conn).await?;
        }
        Ok(Database {
            pool: Pool::Sqlite(SqlitePool::connect(uri).await?),
        })
    }

    /// Load the text of a document from the database.
    pub async fn load(&self, document_id: &str) -> Result<PersistedDocument> {
        let query = r#"SELECT text, language FROM document WHERE id = ?"#;
        Ok(match &self.pool {
            Pool::Sqlite(pool) => {
                sqlx::query_as(query)
                    .bind(document_id)
                    .fetch_one(pool)
                    .await?
            }
            Pool::MySql(pool) => {
                sqlx::query_as(query)
                    .bind(document_id)
                    .fetch_one(pool)
                    .await?
            }
        })
    }

    /// Load a document, or `None` if it does not exist. Unlike [`Self::load`],
    /// other errors are not mixed up with a missing document.
    pub async fn try_load(&self, document_id: &str) -> Result<Option<PersistedDocument>> {
        match self.load(document_id).await {
            Ok(document) => Ok(Some(document)),
            Err(e) => match e.downcast_ref::<sqlx::Error>() {
                Some(sqlx::Error::RowNotFound) => Ok(None),
                _ => Err(e),
            },
        }
    }

    /// Store the text of a document in the database.
    pub async fn store(&self, document_id: &str, document: &PersistedDocument) -> Result<()> {
        let rows = match &self.pool {
            Pool::Sqlite(pool) => sqlx::query(
                r#"
INSERT INTO
    document (id, text, language, updated_at)
VALUES
    (?, ?, ?, strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
ON CONFLICT(id) DO UPDATE SET
    text = excluded.text,
    language = excluded.language,
    updated_at = excluded.updated_at"#,
            )
            .bind(document_id)
            .bind(&document.text)
            .bind(&document.language)
            .execute(pool)
            .await?
            .rows_affected(),
            Pool::MySql(pool) => {
                let rows = sqlx::query(
                    r#"
INSERT INTO
    document (id, text, language, updated_at)
VALUES
    (?, ?, ?, UTC_TIMESTAMP()) AS new
ON DUPLICATE KEY UPDATE
    text = new.text,
    language = new.language,
    updated_at = new.updated_at"#,
                )
                .bind(document_id)
                .bind(&document.text)
                .bind(&document.language)
                .execute(pool)
                .await?
                .rows_affected();
                // MySQL reports 1 for an insert, 2 for an update and 0 when
                // the row was already identical.
                if rows <= 2 { 1 } else { rows }
            }
        };
        if rows != 1 {
            bail!(
                "expected store() to receive 1 row affected, but it affected {} rows instead",
                rows,
            );
        }
        Ok(())
    }

    /// Set or clear the title of a stored document. Returns false if the
    /// document does not exist.
    pub async fn set_title(&self, document_id: &str, title: Option<&str>) -> Result<bool> {
        if !self.exists(document_id).await? {
            return Ok(false);
        }
        let query = "UPDATE document SET title = ? WHERE id = ?";
        match &self.pool {
            Pool::Sqlite(pool) => {
                sqlx::query(query)
                    .bind(title)
                    .bind(document_id)
                    .execute(pool)
                    .await?;
            }
            Pool::MySql(pool) => {
                sqlx::query(query)
                    .bind(title)
                    .bind(document_id)
                    .execute(pool)
                    .await?;
            }
        }
        Ok(true)
    }

    /// Lock or unlock a stored document. Returns false if it does not exist.
    pub async fn set_locked(&self, document_id: &str, locked: bool) -> Result<bool> {
        if !self.exists(document_id).await? {
            return Ok(false);
        }
        let query = "UPDATE document SET locked = ? WHERE id = ?";
        match &self.pool {
            Pool::Sqlite(pool) => {
                sqlx::query(query)
                    .bind(locked)
                    .bind(document_id)
                    .execute(pool)
                    .await?;
            }
            Pool::MySql(pool) => {
                sqlx::query(query)
                    .bind(locked)
                    .bind(document_id)
                    .execute(pool)
                    .await?;
            }
        }
        Ok(true)
    }

    /// Whether a stored document is locked (false if it does not exist).
    pub async fn is_locked(&self, document_id: &str) -> Result<bool> {
        let query = "SELECT locked FROM document WHERE id = ?";
        let row: Option<(bool,)> = match &self.pool {
            Pool::Sqlite(pool) => {
                sqlx::query_as(query)
                    .bind(document_id)
                    .fetch_optional(pool)
                    .await?
            }
            Pool::MySql(pool) => {
                sqlx::query_as(query)
                    .bind(document_id)
                    .fetch_optional(pool)
                    .await?
            }
        };
        Ok(row.is_some_and(|(locked,)| locked))
    }

    /// Saves a version of a document, unless it equals the latest version.
    /// Then thins out versions older than `keep_days` to one per day. Returns
    /// whether a version was added.
    pub async fn snapshot(
        &self,
        document_id: &str,
        document: &PersistedDocument,
        reason: &str,
        keep_days: u32,
    ) -> Result<bool> {
        let latest = "SELECT text, language FROM document_version WHERE doc_id = ? \
                      ORDER BY id DESC LIMIT 1";
        let added = match &self.pool {
            Pool::Sqlite(pool) => {
                let last: Option<(String, Option<String>)> = sqlx::query_as(latest)
                    .bind(document_id)
                    .fetch_optional(pool)
                    .await?;
                if last.as_ref() == Some(&(document.text.clone(), document.language.clone())) {
                    false
                } else {
                    sqlx::query(
                        "INSERT INTO document_version (doc_id, text, language, reason, created_at) \
                         VALUES (?, ?, ?, ?, strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))",
                    )
                    .bind(document_id)
                    .bind(&document.text)
                    .bind(&document.language)
                    .bind(reason)
                    .execute(pool)
                    .await?;
                    true
                }
            }
            Pool::MySql(pool) => {
                let last: Option<(String, Option<String>)> = sqlx::query_as(latest)
                    .bind(document_id)
                    .fetch_optional(pool)
                    .await?;
                if last.as_ref() == Some(&(document.text.clone(), document.language.clone())) {
                    false
                } else {
                    sqlx::query(
                        "INSERT INTO document_version (doc_id, text, language, reason, created_at) \
                         VALUES (?, ?, ?, ?, UTC_TIMESTAMP())",
                    )
                    .bind(document_id)
                    .bind(&document.text)
                    .bind(&document.language)
                    .bind(reason)
                    .execute(pool)
                    .await?;
                    true
                }
            }
        };
        if added {
            self.prune_versions(document_id, keep_days).await?;
        }
        Ok(added)
    }

    /// Keeps every version of the last `keep_days` days, and the newest of
    /// each day before that.
    pub async fn prune_versions(&self, document_id: &str, keep_days: u32) -> Result<u64> {
        Ok(match &self.pool {
            Pool::Sqlite(pool) => sqlx::query(
                "DELETE FROM document_version WHERE id IN (SELECT id FROM ( \
                   SELECT id, ROW_NUMBER() OVER ( \
                     PARTITION BY substr(created_at, 1, 10) ORDER BY id DESC) AS n \
                   FROM document_version WHERE doc_id = ? \
                   AND created_at < strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-' || ? || ' days') \
                 ) WHERE n > 1)",
            )
            .bind(document_id)
            .bind(keep_days)
            .execute(pool)
            .await?
            .rows_affected(),
            Pool::MySql(pool) => sqlx::query(
                "DELETE FROM document_version WHERE id IN (SELECT id FROM ( \
                   SELECT id, ROW_NUMBER() OVER ( \
                     PARTITION BY DATE(created_at) ORDER BY id DESC) AS n \
                   FROM document_version WHERE doc_id = ? \
                   AND created_at < UTC_TIMESTAMP() - INTERVAL ? DAY \
                 ) AS old WHERE n > 1)",
            )
            .bind(document_id)
            .bind(keep_days)
            .execute(pool)
            .await?
            .rows_affected(),
        })
    }

    /// The versions of a document, newest first.
    pub async fn versions(&self, document_id: &str) -> Result<Vec<VersionSummary>> {
        Ok(match &self.pool {
            Pool::Sqlite(pool) => {
                sqlx::query_as(
                    "SELECT id, created_at, reason, language, length(text) AS size \
                     FROM document_version WHERE doc_id = ? ORDER BY id DESC",
                )
                .bind(document_id)
                .fetch_all(pool)
                .await?
            }
            Pool::MySql(pool) => {
                sqlx::query_as(
                    "SELECT id, DATE_FORMAT(created_at, '%Y-%m-%dT%H:%i:%sZ') AS created_at, \
                     reason, language, CAST(CHAR_LENGTH(text) AS SIGNED) AS size \
                     FROM document_version WHERE doc_id = ? ORDER BY id DESC",
                )
                .bind(document_id)
                .fetch_all(pool)
                .await?
            }
        })
    }

    /// One version of a document.
    pub async fn version(&self, document_id: &str, version_id: i64) -> Result<Option<Version>> {
        Ok(match &self.pool {
            Pool::Sqlite(pool) => {
                sqlx::query_as(
                    "SELECT id, created_at, reason, language, text \
                     FROM document_version WHERE doc_id = ? AND id = ?",
                )
                .bind(document_id)
                .bind(version_id)
                .fetch_optional(pool)
                .await?
            }
            Pool::MySql(pool) => {
                sqlx::query_as(
                    "SELECT id, DATE_FORMAT(created_at, '%Y-%m-%dT%H:%i:%sZ') AS created_at, \
                     reason, language, text \
                     FROM document_version WHERE doc_id = ? AND id = ?",
                )
                .bind(document_id)
                .bind(version_id)
                .fetch_optional(pool)
                .await?
            }
        })
    }

    /// Whether a document is stored.
    pub async fn exists(&self, document_id: &str) -> Result<bool> {
        let query = "SELECT count(*) FROM document WHERE id = ?";
        let (count,): (i64,) = match &self.pool {
            Pool::Sqlite(pool) => {
                sqlx::query_as(query)
                    .bind(document_id)
                    .fetch_one(pool)
                    .await?
            }
            Pool::MySql(pool) => {
                sqlx::query_as(query)
                    .bind(document_id)
                    .fetch_one(pool)
                    .await?
            }
        };
        Ok(count > 0)
    }

    /// List all documents, most recently saved first.
    pub async fn list(&self) -> Result<Vec<DocumentSummary>> {
        Ok(match &self.pool {
            Pool::Sqlite(pool) => {
                sqlx::query_as(
                    "SELECT id, title, language, locked, updated_at, substr(text, 1, ?) AS preview \
                     FROM document ORDER BY updated_at IS NULL, updated_at DESC, id",
                )
                .bind(PREVIEW_CHARS)
                .fetch_all(pool)
                .await?
            }
            Pool::MySql(pool) => {
                sqlx::query_as(
                    // The binary collation of id would decode as VARBINARY.
                    "SELECT CAST(id AS CHAR CHARACTER SET utf8mb4) AS id, title, language, locked, \
                     DATE_FORMAT(updated_at, '%Y-%m-%dT%H:%i:%sZ') AS updated_at, \
                     SUBSTRING(text, 1, ?) AS preview \
                     FROM document ORDER BY updated_at IS NULL, updated_at DESC, id",
                )
                .bind(PREVIEW_CHARS)
                .fetch_all(pool)
                .await?
            }
        })
    }

    /// Count the number of documents in the database.
    pub async fn count(&self) -> Result<usize> {
        let query = "SELECT count(*) FROM document";
        let row: (i64,) = match &self.pool {
            Pool::Sqlite(pool) => sqlx::query_as(query).fetch_one(pool).await?,
            Pool::MySql(pool) => sqlx::query_as(query).fetch_one(pool).await?,
        };
        Ok(row.0 as usize)
    }
}
