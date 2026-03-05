mod error;
mod jobs;
mod runs;

pub use error::ApiError;

use axum::{
    routing::{delete, get, post, put},
    Router,
};
use std::sync::Arc;

use crate::db::Repository;
use crate::executor::Executor;
use crate::scheduler::Scheduler;

#[derive(Clone)]
pub struct AppState {
    pub repo: Repository,
    pub executor: Arc<Executor>,
    pub scheduler: Arc<Scheduler>,
}

pub fn create_router(state: AppState) -> Router {
    Router::new()
        .route("/api/jobs", post(jobs::create_job))
        .route("/api/jobs", get(jobs::list_jobs))
        .route("/api/jobs/{id}", get(jobs::get_job))
        .route("/api/jobs/{id}", put(jobs::update_job))
        .route("/api/jobs/{id}", delete(jobs::delete_job))
        .route("/api/jobs/{id}/trigger", post(jobs::trigger_job))
        .route("/api/jobs/{id}/runs", get(runs::list_runs_for_job))
        .route("/api/runs/{id}", get(runs::get_run))
        .route("/api/runs/{id}/logs", get(runs::get_logs))
        .route("/api/runs/{id}", delete(runs::cancel_run))
        .with_state(state)
}
