use std::path::Path;
use tokio::process::Command;

use super::ExecutorError;

pub async fn clone_repo(
    repo_url: &str,
    git_ref: &str,
    target_dir: &Path,
) -> Result<(), ExecutorError> {
    // Remove target directory if it exists
    if target_dir.exists() {
        tokio::fs::remove_dir_all(target_dir).await?;
    }

    // Clone the repository
    let output = Command::new("git")
        .args([
            "clone",
            "--depth",
            "1",
            "--branch",
            git_ref,
            repo_url,
            target_dir.to_str().unwrap(),
        ])
        .output()
        .await?;

    if !output.status.success() {
        // Try without --branch flag (might be a commit hash)
        let output = Command::new("git")
            .args(["clone", repo_url, target_dir.to_str().unwrap()])
            .output()
            .await?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(ExecutorError::Git(format!(
                "Failed to clone repository: {}",
                stderr
            )));
        }

        // Checkout the specific ref
        let output = Command::new("git")
            .current_dir(target_dir)
            .args(["checkout", git_ref])
            .output()
            .await?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(ExecutorError::Git(format!(
                "Failed to checkout ref {}: {}",
                git_ref, stderr
            )));
        }
    }

    Ok(())
}
