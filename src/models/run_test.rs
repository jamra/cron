#[cfg(test)]
mod tests {
    use crate::models::{RunStatus, TriggerType};

    #[test]
    fn test_run_status_as_str() {
        assert_eq!(RunStatus::Pending.as_str(), "pending");
        assert_eq!(RunStatus::Running.as_str(), "running");
        assert_eq!(RunStatus::Succeeded.as_str(), "succeeded");
        assert_eq!(RunStatus::Failed.as_str(), "failed");
        assert_eq!(RunStatus::Retrying.as_str(), "retrying");
        assert_eq!(RunStatus::Cancelled.as_str(), "cancelled");
    }

    #[test]
    fn test_run_status_from_str() {
        assert_eq!(RunStatus::from_str("pending"), Some(RunStatus::Pending));
        assert_eq!(RunStatus::from_str("running"), Some(RunStatus::Running));
        assert_eq!(RunStatus::from_str("succeeded"), Some(RunStatus::Succeeded));
        assert_eq!(RunStatus::from_str("failed"), Some(RunStatus::Failed));
        assert_eq!(RunStatus::from_str("retrying"), Some(RunStatus::Retrying));
        assert_eq!(RunStatus::from_str("cancelled"), Some(RunStatus::Cancelled));
        assert_eq!(RunStatus::from_str("invalid"), None);
    }

    #[test]
    fn test_run_status_serialization() {
        let status = RunStatus::Running;
        let json = serde_json::to_string(&status).unwrap();
        assert_eq!(json, "\"running\"");

        let parsed: RunStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, status);
    }

    #[test]
    fn test_trigger_type_as_str() {
        assert_eq!(TriggerType::Scheduled.as_str(), "scheduled");
        assert_eq!(TriggerType::Manual.as_str(), "manual");
    }

    #[test]
    fn test_trigger_type_from_str() {
        assert_eq!(TriggerType::from_str("scheduled"), Some(TriggerType::Scheduled));
        assert_eq!(TriggerType::from_str("manual"), Some(TriggerType::Manual));
        assert_eq!(TriggerType::from_str("invalid"), None);
    }

    #[test]
    fn test_trigger_type_serialization() {
        let trigger = TriggerType::Manual;
        let json = serde_json::to_string(&trigger).unwrap();
        assert_eq!(json, "\"manual\"");

        let parsed: TriggerType = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, trigger);
    }
}
