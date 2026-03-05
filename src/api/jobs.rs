use axum::{
    extract::{Path, State},
    Json,
};
use uuid::Uuid;

use super::{ApiError, AppState};
use crate::models::{CreateJobRequest, Job, Run, TriggerType, UpdateJobRequest};

pub async fn create_job(
    State(state): State<AppState>,
    Json(req): Json<CreateJobRequest>,
) -> Result<Json<Job>, ApiError> {
    if req.name.is_empty() {
        return Err(ApiError::BadRequest("name is required".to_string()));
    }
    if req.dockerfile.is_empty() {
        return Err(ApiError::BadRequest("dockerfile is required".to_string()));
    }

    let job = state.repo.create_job(req).await?;

    // Add to scheduler if it has a schedule
    state.scheduler.schedule_job(&job).await;

    Ok(Json(job))
}

pub async fn list_jobs(State(state): State<AppState>) -> Result<Json<Vec<Job>>, ApiError> {
    let jobs = state.repo.list_jobs().await?;
    Ok(Json(jobs))
}

pub async fn get_job(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Job>, ApiError> {
    let job = state.repo.get_job(id).await?.ok_or(ApiError::NotFound)?;
    Ok(Json(job))
}

pub async fn update_job(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateJobRequest>,
) -> Result<Json<Job>, ApiError> {
    let job = state
        .repo
        .update_job(id, req)
        .await?
        .ok_or(ApiError::NotFound)?;

    // Update scheduler
    if job.enabled && job.schedule.is_some() {
        state.scheduler.schedule_job(&job).await;
    } else {
        state.scheduler.unschedule_job(job.id).await;
    }

    Ok(Json(job))
}

pub async fn delete_job(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    // Remove from scheduler first
    state.scheduler.unschedule_job(id).await;

    let deleted = state.repo.delete_job(id).await?;
    if !deleted {
        return Err(ApiError::NotFound);
    }
    Ok(Json(serde_json::json!({ "deleted": true })))
}

pub async fn trigger_job(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Run>, ApiError> {
    // Verify job exists
    let job = state.repo.get_job(id).await?.ok_or(ApiError::NotFound)?;

    // Create a new run
    let run = state.repo.create_run(job.id, TriggerType::Manual).await?;

    // Spawn execution in background
    let executor = state.executor.clone();
    let repo = state.repo.clone();
    let run_clone = run.clone();
    tokio::spawn(async move {
        if let Err(e) = executor.execute_run(&repo, &job, &run_clone).await {
            tracing::error!("Failed to execute run {}: {}", run_clone.id, e);
        }
    });

    Ok(Json(run))
}
