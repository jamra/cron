mod repository;

pub use repository::Repository;

use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;
use std::str::FromStr;

pub async fn create_pool(database_url: &str) -> Result<SqlitePool, sqlx::Error> {
    let options = SqliteConnectOptions::from_str(database_url)?
        .create_if_missing(true)
        .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal);

    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(options)
        .await?;

    // Run migrations
    run_migrations(&pool).await?;

    Ok(pool)
}

async fn run_migrations(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS jobs (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            description TEXT,
            git_repo TEXT NOT NULL,
            git_ref TEXT,
            dockerfile_path TEXT NOT NULL DEFAULT 'Dockerfile',
            schedule_json TEXT,
            enabled INTEGER NOT NULL DEFAULT 1,
            max_retries INTEGER NOT NULL DEFAULT 0,
            retry_delay_secs INTEGER NOT NULL DEFAULT 60,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        )
        "#,
    )
    .execute(pool)
    .await?;

    // Migration: add retry columns if they don't exist
    let _ = sqlx::query("ALTER TABLE jobs ADD COLUMN max_retries INTEGER NOT NULL DEFAULT 0")
        .execute(pool)
        .await;
    let _ = sqlx::query("ALTER TABLE jobs ADD COLUMN retry_delay_secs INTEGER NOT NULL DEFAULT 60")
        .execute(pool)
        .await;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS runs (
            id TEXT PRIMARY KEY,
            job_id TEXT NOT NULL,
            status TEXT NOT NULL,
            started_at TEXT,
            finished_at TEXT,
            exit_code INTEGER,
            triggered_by TEXT NOT NULL,
            attempt INTEGER NOT NULL DEFAULT 1,
            created_at TEXT NOT NULL,
            FOREIGN KEY (job_id) REFERENCES jobs(id) ON DELETE CASCADE
        )
        "#,
    )
    .execute(pool)
    .await?;

    // Migration: add attempt column if it doesn't exist
    let _ = sqlx::query("ALTER TABLE runs ADD COLUMN attempt INTEGER NOT NULL DEFAULT 1")
        .execute(pool)
        .await;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_runs_job_id ON runs(job_id)
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_runs_status ON runs(status)
        "#,
    )
    .execute(pool)
    .await?;

    Ok(())
}
