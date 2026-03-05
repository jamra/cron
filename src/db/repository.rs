use chrono::{DateTime, Utc};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::models::{CreateJobRequest, Job, Run, RunStatus, TriggerType, UpdateJobRequest};

#[derive(Clone)]
pub struct Repository {
    pool: SqlitePool,
}

impl Repository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    // Job operations

    pub async fn create_job(&self, req: CreateJobRequest) -> Result<Job, sqlx::Error> {
        let id = Uuid::new_v4();
        let now = Utc::now();
        let schedule_json = req.schedule.as_ref().map(|s| serde_json::to_string(s).unwrap());
        let files_json = serde_json::to_string(&req.files).unwrap();
        let enabled = req.enabled.unwrap_or(true);
        let max_retries = req.max_retries.unwrap_or(0);
        let retry_delay_secs = req.retry_delay_secs.unwrap_or(60);

        sqlx::query(
            r#"
            INSERT INTO jobs (id, name, description, dockerfile, files_json, schedule_json, enabled, max_retries, retry_delay_secs, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(id.to_string())
        .bind(&req.name)
        .bind(&req.description)
        .bind(&req.dockerfile)
        .bind(&files_json)
        .bind(&schedule_json)
        .bind(enabled)
        .bind(max_retries)
        .bind(retry_delay_secs)
        .bind(now.to_rfc3339())
        .bind(now.to_rfc3339())
        .execute(&self.pool)
        .await?;

        Ok(Job {
            id,
            name: req.name,
            description: req.description,
            dockerfile: req.dockerfile,
            files: req.files,
            schedule: req.schedule,
            enabled,
            max_retries,
            retry_delay_secs,
            created_at: now,
            updated_at: now,
        })
    }

    pub async fn get_job(&self, id: Uuid) -> Result<Option<Job>, sqlx::Error> {
        let row: Option<JobRow> = sqlx::query_as(
            r#"
            SELECT id, name, description, dockerfile, files_json, schedule_json, enabled, max_retries, retry_delay_secs, created_at, updated_at
            FROM jobs WHERE id = ?
            "#,
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|r| r.into_job()))
    }

    pub async fn list_jobs(&self) -> Result<Vec<Job>, sqlx::Error> {
        let rows: Vec<JobRow> = sqlx::query_as(
            r#"
            SELECT id, name, description, dockerfile, files_json, schedule_json, enabled, max_retries, retry_delay_secs, created_at, updated_at
            FROM jobs ORDER BY created_at DESC
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(|r| r.into_job()).collect())
    }

    pub async fn list_enabled_jobs_with_schedule(&self) -> Result<Vec<Job>, sqlx::Error> {
        let rows: Vec<JobRow> = sqlx::query_as(
            r#"
            SELECT id, name, description, dockerfile, files_json, schedule_json, enabled, max_retries, retry_delay_secs, created_at, updated_at
            FROM jobs WHERE enabled = 1 AND schedule_json IS NOT NULL
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(|r| r.into_job()).collect())
    }

    pub async fn update_job(&self, id: Uuid, req: UpdateJobRequest) -> Result<Option<Job>, sqlx::Error> {
        let existing = self.get_job(id).await?;
        let Some(mut job) = existing else {
            return Ok(None);
        };

        if let Some(name) = req.name {
            job.name = name;
        }
        if let Some(description) = req.description {
            job.description = Some(description);
        }
        if let Some(dockerfile) = req.dockerfile {
            job.dockerfile = dockerfile;
        }
        if let Some(files) = req.files {
            job.files = files;
        }
        if let Some(schedule) = req.schedule {
            job.schedule = Some(schedule);
        }
        if let Some(enabled) = req.enabled {
            job.enabled = enabled;
        }
        if let Some(max_retries) = req.max_retries {
            job.max_retries = max_retries;
        }
        if let Some(retry_delay_secs) = req.retry_delay_secs {
            job.retry_delay_secs = retry_delay_secs;
        }

        job.updated_at = Utc::now();
        let schedule_json = job.schedule.as_ref().map(|s| serde_json::to_string(s).unwrap());
        let files_json = serde_json::to_string(&job.files).unwrap();

        sqlx::query(
            r#"
            UPDATE jobs SET name = ?, description = ?, dockerfile = ?, files_json = ?, schedule_json = ?, enabled = ?, max_retries = ?, retry_delay_secs = ?, updated_at = ?
            WHERE id = ?
            "#,
        )
        .bind(&job.name)
        .bind(&job.description)
        .bind(&job.dockerfile)
        .bind(&files_json)
        .bind(&schedule_json)
        .bind(job.enabled)
        .bind(job.max_retries)
        .bind(job.retry_delay_secs)
        .bind(job.updated_at.to_rfc3339())
        .bind(id.to_string())
        .execute(&self.pool)
        .await?;

        Ok(Some(job))
    }

    pub async fn delete_job(&self, id: Uuid) -> Result<bool, sqlx::Error> {
        let result = sqlx::query("DELETE FROM jobs WHERE id = ?")
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected() > 0)
    }

    // Run operations

    pub async fn create_run(&self, job_id: Uuid, triggered_by: TriggerType) -> Result<Run, sqlx::Error> {
        self.create_run_with_attempt(job_id, triggered_by, 1).await
    }

    pub async fn create_run_with_attempt(
        &self,
        job_id: Uuid,
        triggered_by: TriggerType,
        attempt: u32,
    ) -> Result<Run, sqlx::Error> {
        let id = Uuid::new_v4();
        let now = Utc::now();

        sqlx::query(
            r#"
            INSERT INTO runs (id, job_id, status, triggered_by, attempt, created_at)
            VALUES (?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(id.to_string())
        .bind(job_id.to_string())
        .bind(RunStatus::Pending.as_str())
        .bind(triggered_by.as_str())
        .bind(attempt)
        .bind(now.to_rfc3339())
        .execute(&self.pool)
        .await?;

        Ok(Run {
            id,
            job_id,
            status: RunStatus::Pending,
            started_at: None,
            finished_at: None,
            exit_code: None,
            triggered_by,
            attempt,
            created_at: now,
        })
    }

    pub async fn get_run(&self, id: Uuid) -> Result<Option<Run>, sqlx::Error> {
        let row: Option<RunRow> = sqlx::query_as(
            r#"
            SELECT id, job_id, status, started_at, finished_at, exit_code, triggered_by, attempt, created_at
            FROM runs WHERE id = ?
            "#,
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|r| r.into_run()))
    }

    pub async fn list_runs_for_job(&self, job_id: Uuid) -> Result<Vec<Run>, sqlx::Error> {
        let rows: Vec<RunRow> = sqlx::query_as(
            r#"
            SELECT id, job_id, status, started_at, finished_at, exit_code, triggered_by, attempt, created_at
            FROM runs WHERE job_id = ? ORDER BY created_at DESC
            "#,
        )
        .bind(job_id.to_string())
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(|r| r.into_run()).collect())
    }

    pub async fn get_last_run_for_job(&self, job_id: Uuid) -> Result<Option<Run>, sqlx::Error> {
        let row: Option<RunRow> = sqlx::query_as(
            r#"
            SELECT id, job_id, status, started_at, finished_at, exit_code, triggered_by, attempt, created_at
            FROM runs WHERE job_id = ? ORDER BY created_at DESC LIMIT 1
            "#,
        )
        .bind(job_id.to_string())
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|r| r.into_run()))
    }

    #[allow(dead_code)]
    pub async fn get_pending_runs(&self) -> Result<Vec<Run>, sqlx::Error> {
        let rows: Vec<RunRow> = sqlx::query_as(
            r#"
            SELECT id, job_id, status, started_at, finished_at, exit_code, triggered_by, attempt, created_at
            FROM runs WHERE status = 'pending' ORDER BY created_at ASC
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(|r| r.into_run()).collect())
    }

    pub async fn update_run_status(
        &self,
        id: Uuid,
        status: RunStatus,
        exit_code: Option<i32>,
    ) -> Result<(), sqlx::Error> {
        let now = Utc::now();

        match status {
            RunStatus::Running => {
                sqlx::query(
                    r#"
                    UPDATE runs SET status = ?, started_at = ? WHERE id = ?
                    "#,
                )
                .bind(status.as_str())
                .bind(now.to_rfc3339())
                .bind(id.to_string())
                .execute(&self.pool)
                .await?;
            }
            RunStatus::Succeeded | RunStatus::Failed | RunStatus::Cancelled => {
                sqlx::query(
                    r#"
                    UPDATE runs SET status = ?, finished_at = ?, exit_code = ? WHERE id = ?
                    "#,
                )
                .bind(status.as_str())
                .bind(now.to_rfc3339())
                .bind(exit_code)
                .bind(id.to_string())
                .execute(&self.pool)
                .await?;
            }
            _ => {
                sqlx::query(
                    r#"
                    UPDATE runs SET status = ? WHERE id = ?
                    "#,
                )
                .bind(status.as_str())
                .bind(id.to_string())
                .execute(&self.pool)
                .await?;
            }
        }

        Ok(())
    }
}

#[derive(sqlx::FromRow)]
struct JobRow {
    id: String,
    name: String,
    description: Option<String>,
    dockerfile: String,
    files_json: String,
    schedule_json: Option<String>,
    enabled: bool,
    max_retries: u32,
    retry_delay_secs: u32,
    created_at: String,
    updated_at: String,
}

impl JobRow {
    fn into_job(self) -> Job {
        Job {
            id: Uuid::parse_str(&self.id).unwrap(),
            name: self.name,
            description: self.description,
            dockerfile: self.dockerfile,
            files: serde_json::from_str(&self.files_json).unwrap_or_default(),
            schedule: self.schedule_json.and_then(|s| serde_json::from_str(&s).ok()),
            enabled: self.enabled,
            max_retries: self.max_retries,
            retry_delay_secs: self.retry_delay_secs,
            created_at: DateTime::parse_from_rfc3339(&self.created_at)
                .unwrap()
                .with_timezone(&Utc),
            updated_at: DateTime::parse_from_rfc3339(&self.updated_at)
                .unwrap()
                .with_timezone(&Utc),
        }
    }
}

#[derive(sqlx::FromRow)]
struct RunRow {
    id: String,
    job_id: String,
    status: String,
    started_at: Option<String>,
    finished_at: Option<String>,
    exit_code: Option<i32>,
    triggered_by: String,
    attempt: u32,
    created_at: String,
}

impl RunRow {
    fn into_run(self) -> Run {
        Run {
            id: Uuid::parse_str(&self.id).unwrap(),
            job_id: Uuid::parse_str(&self.job_id).unwrap(),
            status: RunStatus::from_str(&self.status).unwrap_or(RunStatus::Pending),
            started_at: self
                .started_at
                .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
                .map(|dt| dt.with_timezone(&Utc)),
            finished_at: self
                .finished_at
                .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
                .map(|dt| dt.with_timezone(&Utc)),
            exit_code: self.exit_code,
            triggered_by: TriggerType::from_str(&self.triggered_by).unwrap_or(TriggerType::Manual),
            attempt: self.attempt,
            created_at: DateTime::parse_from_rfc3339(&self.created_at)
                .unwrap()
                .with_timezone(&Utc),
        }
    }
}
