mod docker;
mod git;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::db::Repository;
use crate::models::{Job, Run, RunStatus};

pub struct Executor {
    logs_dir: PathBuf,
    work_dir: PathBuf,
    docker: docker::DockerClient,
    running_containers: Arc<RwLock<HashMap<Uuid, String>>>,
}

impl Executor {
    pub async fn new(logs_dir: PathBuf, work_dir: PathBuf) -> Result<Self, ExecutorError> {
        // Ensure directories exist
        tokio::fs::create_dir_all(&logs_dir).await?;
        tokio::fs::create_dir_all(&work_dir).await?;

        let docker = docker::DockerClient::new().await?;

        Ok(Self {
            logs_dir,
            work_dir,
            docker,
            running_containers: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    /// Execute a run with automatic retries on failure.
    ///
    /// # Retry Behavior
    ///
    /// If `job.max_retries > 0` and the run fails:
    /// 1. Mark current run as "retrying"
    /// 2. Wait for `retry_delay_secs * 2^attempt` seconds (exponential backoff)
    /// 3. Create a new run with incremented attempt number
    /// 4. Execute the new run
    /// 5. Repeat until success or max_retries reached
    ///
    /// # Example
    ///
    /// With `max_retries: 3` and `retry_delay_secs: 60`:
    /// - Attempt 1: runs immediately
    /// - Attempt 2: waits 60s after failure
    /// - Attempt 3: waits 120s after failure
    /// - Attempt 4: waits 240s after failure (final attempt)
    pub async fn execute_run(
        &self,
        repo: &Repository,
        job: &Job,
        run: &Run,
    ) -> Result<(), ExecutorError> {
        let mut current_run = run.clone();

        loop {
            tracing::info!(
                "Starting execution of run {} for job {} (attempt {}/{})",
                current_run.id,
                job.name,
                current_run.attempt,
                job.max_retries + 1
            );

            let result = self.execute_single_run(repo, job, &current_run).await;

            match result {
                Ok(true) => {
                    // Success
                    return Ok(());
                }
                Ok(false) | Err(_) => {
                    // Failed - check if we should retry
                    let can_retry = current_run.attempt <= job.max_retries;

                    if can_retry {
                        // Mark current run as retrying
                        repo.update_run_status(current_run.id, RunStatus::Retrying, None)
                            .await
                            .map_err(|e| ExecutorError::Database(e.to_string()))?;

                        // Calculate backoff delay: base_delay * 2^(attempt-1)
                        let delay_secs = job.retry_delay_secs as u64
                            * (1u64 << (current_run.attempt - 1).min(10));

                        tracing::info!(
                            "Run {} failed, retrying in {} seconds (attempt {}/{})",
                            current_run.id,
                            delay_secs,
                            current_run.attempt + 1,
                            job.max_retries + 1
                        );

                        // Wait before retry
                        tokio::time::sleep(std::time::Duration::from_secs(delay_secs)).await;

                        // Create new run for retry
                        current_run = repo
                            .create_run_with_attempt(
                                job.id,
                                current_run.triggered_by,
                                current_run.attempt + 1,
                            )
                            .await
                            .map_err(|e| ExecutorError::Database(e.to_string()))?;
                    } else {
                        // No more retries
                        tracing::info!(
                            "Run {} failed after {} attempts, no more retries",
                            current_run.id,
                            current_run.attempt
                        );
                        return result.map(|_| ());
                    }
                }
            }
        }
    }

    /// Execute a single run attempt. Returns Ok(true) on success, Ok(false) on failure.
    async fn execute_single_run(
        &self,
        repo: &Repository,
        job: &Job,
        run: &Run,
    ) -> Result<bool, ExecutorError> {
        // Update status to running
        repo.update_run_status(run.id, RunStatus::Running, None)
            .await
            .map_err(|e| ExecutorError::Database(e.to_string()))?;

        // Create log directory for this run
        let run_logs_dir = self.logs_dir.join(run.id.to_string());
        tokio::fs::create_dir_all(&run_logs_dir).await?;

        // Clone/pull the git repository
        let repo_dir = self.work_dir.join(format!("{}_{}", job.id, run.id));
        let git_ref = job.git_ref.as_deref().unwrap_or("main");

        match git::clone_repo(&job.git_repo, git_ref, &repo_dir).await {
            Ok(_) => tracing::info!("Cloned repository to {:?}", repo_dir),
            Err(e) => {
                tracing::error!("Failed to clone repository: {}", e);
                self.write_log(run.id, "stderr", &format!("Failed to clone repository: {}\n", e))
                    .await?;
                repo.update_run_status(run.id, RunStatus::Failed, Some(1))
                    .await
                    .map_err(|e| ExecutorError::Database(e.to_string()))?;
                return Ok(false);
            }
        }

        // Build Docker image
        let image_tag = format!("scheduler-job-{}:{}", job.id, run.id);
        let dockerfile_path = repo_dir.join(&job.dockerfile_path);

        match self
            .docker
            .build_image(&repo_dir, &dockerfile_path, &image_tag, run.id, &self.logs_dir)
            .await
        {
            Ok(_) => tracing::info!("Built Docker image: {}", image_tag),
            Err(e) => {
                tracing::error!("Failed to build Docker image: {}", e);
                self.write_log(run.id, "stderr", &format!("Failed to build Docker image: {}\n", e))
                    .await?;
                repo.update_run_status(run.id, RunStatus::Failed, Some(1))
                    .await
                    .map_err(|e| ExecutorError::Database(e.to_string()))?;
                let _ = tokio::fs::remove_dir_all(&repo_dir).await;
                return Ok(false);
            }
        }

        // Run the container
        let container_id = match self.docker.create_container(&image_tag).await {
            Ok(id) => {
                tracing::info!("Created container: {}", id);
                id
            }
            Err(e) => {
                tracing::error!("Failed to create container: {}", e);
                self.write_log(run.id, "stderr", &format!("Failed to create container: {}\n", e))
                    .await?;
                repo.update_run_status(run.id, RunStatus::Failed, Some(1))
                    .await
                    .map_err(|e| ExecutorError::Database(e.to_string()))?;
                let _ = tokio::fs::remove_dir_all(&repo_dir).await;
                return Ok(false);
            }
        };

        // Track the running container
        {
            let mut containers = self.running_containers.write().await;
            containers.insert(run.id, container_id.clone());
        }

        // Start the container and stream logs
        let exit_code = self
            .docker
            .run_container(&container_id, run.id, &self.logs_dir)
            .await;

        // Remove from tracking
        {
            let mut containers = self.running_containers.write().await;
            containers.remove(&run.id);
        }

        // Clean up container and image
        let _ = self.docker.remove_container(&container_id).await;
        let _ = self.docker.remove_image(&image_tag).await;
        let _ = tokio::fs::remove_dir_all(&repo_dir).await;

        // Update final status
        let (status, code, success) = match exit_code {
            Ok(0) => (RunStatus::Succeeded, Some(0), true),
            Ok(code) => (RunStatus::Failed, Some(code as i32), false),
            Err(e) => {
                tracing::error!("Container execution failed: {}", e);
                (RunStatus::Failed, Some(1), false)
            }
        };

        repo.update_run_status(run.id, status, code)
            .await
            .map_err(|e| ExecutorError::Database(e.to_string()))?;

        tracing::info!("Run {} completed with status {:?}", run.id, status);
        Ok(success)
    }

    pub async fn cancel_run(&self, run_id: Uuid) {
        let container_id = {
            let containers = self.running_containers.read().await;
            containers.get(&run_id).cloned()
        };

        if let Some(container_id) = container_id {
            tracing::info!("Cancelling run {} (container {})", run_id, container_id);
            let _ = self.docker.stop_container(&container_id).await;
        }
    }

    pub async fn read_log(&self, run_id: Uuid, log_type: &str) -> Result<String, ExecutorError> {
        let log_path = self
            .logs_dir
            .join(run_id.to_string())
            .join(format!("{}.log", log_type));

        match tokio::fs::read_to_string(&log_path).await {
            Ok(content) => Ok(content),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
            Err(e) => Err(ExecutorError::Io(e)),
        }
    }

    async fn write_log(&self, run_id: Uuid, log_type: &str, content: &str) -> Result<(), ExecutorError> {
        let run_logs_dir = self.logs_dir.join(run_id.to_string());
        tokio::fs::create_dir_all(&run_logs_dir).await?;

        let log_path = run_logs_dir.join(format!("{}.log", log_type));
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .await?;

        use tokio::io::AsyncWriteExt;
        file.write_all(content.as_bytes()).await?;
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ExecutorError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Docker error: {0}")]
    Docker(#[from] bollard::errors::Error),

    #[error("Git error: {0}")]
    Git(String),

    #[error("Database error: {0}")]
    Database(String),
}
