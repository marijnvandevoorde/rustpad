//! Backend database handlers for persisting documents, in SQLite or MySQL.

use std::str::FromStr;

use anyhow::{bail, Result};
use sqlx::{sqlite::SqliteConnectOptions, ConnectOptions, MySqlPool, SqlitePool};

/// Represents a document persisted in database storage.
#[derive(sqlx::FromRow, PartialEq, Eq, Clone, Debug)]
pub struct PersistedDocument {
    /// Text content of the document.
    pub text: String,
    /// Language of the document for editor syntax highlighting.
    pub language: Option<String>,
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
    id VARCHAR(255) NOT NULL PRIMARY KEY,
    text LONGTEXT NOT NULL,
    language VARCHAR(255)
) DEFAULT CHARSET = utf8mb4 COLLATE = utf8mb4_bin"#;

impl Database {
    /// Construct a new database from a SQLite or MySQL connection URI.
    ///
    /// A `mysql://` URI connects to MySQL (TLS is controlled by its `ssl-mode`
    /// parameter); anything else is treated as SQLite.
    pub async fn new(uri: &str) -> Result<Self> {
        if uri.starts_with("mysql://") {
            let pool = MySqlPool::connect(uri).await?;
            sqlx::query(MYSQL_SCHEMA).execute(&pool).await?;
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

    /// Store the text of a document in the database.
    pub async fn store(&self, document_id: &str, document: &PersistedDocument) -> Result<()> {
        let rows = match &self.pool {
            Pool::Sqlite(pool) => sqlx::query(
                r#"
INSERT INTO
    document (id, text, language)
VALUES
    (?, ?, ?)
ON CONFLICT(id) DO UPDATE SET
    text = excluded.text,
    language = excluded.language"#,
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
    document (id, text, language)
VALUES
    (?, ?, ?) AS new
ON DUPLICATE KEY UPDATE
    text = new.text,
    language = new.language"#,
                )
                .bind(document_id)
                .bind(&document.text)
                .bind(&document.language)
                .execute(pool)
                .await?
                .rows_affected();
                // MySQL reports 1 for an insert, 2 for an update and 0 when
                // the row was already identical.
                if rows <= 2 {
                    1
                } else {
                    rows
                }
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
