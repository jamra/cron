use std::env;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    pub logs_dir: PathBuf,
    pub work_dir: PathBuf,
    pub api_host: String,
    pub api_port: u16,
    #[allow(dead_code)]
    pub scheduler_interval_secs: u64,
}

impl Config {
    pub fn from_env() -> Self {
        let data_dir = env::var("SCHEDULER_DATA_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("./data"));

        let logs_dir = env::var("SCHEDULER_LOGS_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("./logs"));

        let work_dir = env::var("SCHEDULER_WORK_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("./work"));

        let database_url = env::var("DATABASE_URL")
            .unwrap_or_else(|_| format!("sqlite:{}/scheduler.db?mode=rwc", data_dir.display()));

        let api_host = env::var("SCHEDULER_HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
        let api_port = env::var("SCHEDULER_PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(3000);

        let scheduler_interval_secs = env::var("SCHEDULER_INTERVAL_SECS")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(60);

        Config {
            database_url,
            logs_dir,
            work_dir,
            api_host,
            api_port,
            scheduler_interval_secs,
        }
    }
}
