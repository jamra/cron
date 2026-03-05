use bollard::container::{
    Config, CreateContainerOptions, LogsOptions, RemoveContainerOptions, StopContainerOptions,
    WaitContainerOptions,
};
use bollard::image::{BuildImageOptions, RemoveImageOptions};
use bollard::Docker;
use futures_util::StreamExt;
use std::path::Path;
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

use super::ExecutorError;

pub struct DockerClient {
    client: Docker,
}

impl DockerClient {
    pub async fn new() -> Result<Self, ExecutorError> {
        let client = Docker::connect_with_local_defaults()?;

        // Verify connection
        client.ping().await?;

        Ok(Self { client })
    }

    pub async fn build_image(
        &self,
        context_dir: &Path,
        dockerfile_path: &Path,
        tag: &str,
        run_id: Uuid,
        logs_dir: &Path,
    ) -> Result<(), ExecutorError> {
        // Create a tar archive of the build context
        let tar_bytes = create_tar_archive(context_dir).await?;

        // Get relative dockerfile path from context
        let dockerfile_rel = dockerfile_path
            .strip_prefix(context_dir)
            .unwrap_or(dockerfile_path)
            .to_str()
            .unwrap_or("Dockerfile");

        let options = BuildImageOptions {
            dockerfile: dockerfile_rel,
            t: tag,
            rm: true,
            ..Default::default()
        };

        let mut stream = self
            .client
            .build_image(options, None, Some(tar_bytes.into()));

        // Open log file for build output
        let log_path = logs_dir.join(run_id.to_string()).join("stdout.log");
        tokio::fs::create_dir_all(log_path.parent().unwrap()).await?;
        let mut log_file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .await?;

        while let Some(result) = stream.next().await {
            match result {
                Ok(info) => {
                    if let Some(stream) = info.stream {
                        log_file.write_all(stream.as_bytes()).await?;
                    }
                    if let Some(error) = info.error {
                        let err_msg = format!("Build error: {}\n", error);
                        log_file.write_all(err_msg.as_bytes()).await?;
                        return Err(ExecutorError::Build(error));
                    }
                }
                Err(e) => {
                    let err_msg = format!("Build stream error: {}\n", e);
                    log_file.write_all(err_msg.as_bytes()).await?;
                    return Err(e.into());
                }
            }
        }

        Ok(())
    }

    pub async fn create_container(&self, image: &str) -> Result<String, ExecutorError> {
        let config = Config {
            image: Some(image),
            ..Default::default()
        };

        let options = CreateContainerOptions {
            name: format!("scheduler-run-{}", Uuid::new_v4()),
            ..Default::default()
        };

        let response = self.client.create_container(Some(options), config).await?;
        Ok(response.id)
    }

    pub async fn run_container(
        &self,
        container_id: &str,
        run_id: Uuid,
        logs_dir: &Path,
    ) -> Result<i64, ExecutorError> {
        // Start the container
        self.client.start_container::<String>(container_id, None).await?;

        // Open log files
        let logs_path = logs_dir.join(run_id.to_string());
        tokio::fs::create_dir_all(&logs_path).await?;

        let stdout_path = logs_path.join("stdout.log");
        let stderr_path = logs_path.join("stderr.log");

        let mut stdout_file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&stdout_path)
            .await?;

        let mut stderr_file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&stderr_path)
            .await?;

        // Stream logs
        let log_options = LogsOptions::<String> {
            follow: true,
            stdout: true,
            stderr: true,
            ..Default::default()
        };

        let mut log_stream = self.client.logs(container_id, Some(log_options));

        while let Some(result) = log_stream.next().await {
            match result {
                Ok(output) => {
                    use bollard::container::LogOutput;
                    match output {
                        LogOutput::StdOut { message } => {
                            stdout_file.write_all(&message).await?;
                        }
                        LogOutput::StdErr { message } => {
                            stderr_file.write_all(&message).await?;
                        }
                        _ => {}
                    }
                }
                Err(e) => {
                    tracing::warn!("Error reading container logs: {}", e);
                    break;
                }
            }
        }

        // Wait for container to finish
        let wait_options = WaitContainerOptions {
            condition: "not-running",
        };

        let mut wait_stream = self.client.wait_container(container_id, Some(wait_options));

        while let Some(result) = wait_stream.next().await {
            match result {
                Ok(response) => {
                    return Ok(response.status_code);
                }
                Err(e) => {
                    tracing::error!("Error waiting for container: {}", e);
                    return Err(e.into());
                }
            }
        }

        Ok(0)
    }

    pub async fn stop_container(&self, container_id: &str) -> Result<(), ExecutorError> {
        let options = StopContainerOptions { t: 10 };
        self.client.stop_container(container_id, Some(options)).await?;
        Ok(())
    }

    pub async fn remove_container(&self, container_id: &str) -> Result<(), ExecutorError> {
        let options = RemoveContainerOptions {
            force: true,
            ..Default::default()
        };
        self.client.remove_container(container_id, Some(options)).await?;
        Ok(())
    }

    pub async fn remove_image(&self, image: &str) -> Result<(), ExecutorError> {
        let options = RemoveImageOptions {
            force: true,
            ..Default::default()
        };
        let _ = self.client.remove_image(image, Some(options), None).await;
        Ok(())
    }
}

async fn create_tar_archive(dir: &Path) -> Result<Vec<u8>, ExecutorError> {
    use tar::Builder;

    let dir = dir.to_path_buf();
    let result = tokio::task::spawn_blocking(move || {
        let mut buffer = Vec::new();
        {
            let mut archive = Builder::new(&mut buffer);
            archive.append_dir_all(".", &dir)?;
            archive.finish()?;
        }
        Ok::<_, std::io::Error>(buffer)
    })
    .await
    .map_err(|e| ExecutorError::Build(format!("Task join error: {}", e)))?;

    result.map_err(ExecutorError::Io)
}
