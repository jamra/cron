use axum::{
    extract::{Path, State},
    Json,
};
use serde::Serialize;
use uuid::Uuid;

use super::{ApiError, AppState};
use crate::models::{Run, RunStatus};

pub async fn list_runs_for_job(
    State(state): State<AppState>,
    Path(job_id): Path<Uuid>,
) -> Result<Json<Vec<Run>>, ApiError> {
    // Verify job exists
    state
        .repo
        .get_job(job_id)
        .await?
        .ok_or(ApiError::NotFound)?;

    let runs = state.repo.list_runs_for_job(job_id).await?;
    Ok(Json(runs))
}

pub async fn get_run(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Run>, ApiError> {
    let run = state.repo.get_run(id).await?.ok_or(ApiError::NotFound)?;
    Ok(Json(run))
}

#[derive(Serialize)]
pub struct LogsResponse {
    pub stdout: String,
    pub stderr: String,
}

pub async fn get_logs(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<LogsResponse>, ApiError> {
    // Verify run exists
    state.repo.get_run(id).await?.ok_or(ApiError::NotFound)?;

    let stdout = state.executor.read_log(id, "stdout").await.unwrap_or_default();
    let stderr = state.executor.read_log(id, "stderr").await.unwrap_or_default();

    Ok(Json(LogsResponse { stdout, stderr }))
}

pub async fn cancel_run(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Run>, ApiError> {
    let run = state.repo.get_run(id).await?.ok_or(ApiError::NotFound)?;

    if run.status != RunStatus::Pending && run.status != RunStatus::Running {
        return Err(ApiError::BadRequest(format!(
            "Cannot cancel run with status {:?}",
            run.status
        )));
    }

    // Cancel the run
    state.executor.cancel_run(id).await;
    state
        .repo
        .update_run_status(id, RunStatus::Cancelled, None)
        .await?;

    let run = state.repo.get_run(id).await?.ok_or(ApiError::NotFound)?;
    Ok(Json(run))
}
