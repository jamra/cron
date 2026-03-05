#[cfg(test)]
mod tests {
    use crate::models::{Schedule, Weekday};

    #[test]
    fn test_schedule_interval_serialization() {
        let schedule = Schedule::Interval {
            minutes: Some(30),
            hours: None,
            days: None,
        };
        let json = serde_json::to_string(&schedule).unwrap();
        assert!(json.contains("\"type\":\"interval\""));
        assert!(json.contains("\"minutes\":30"));

        let parsed: Schedule = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, schedule);
    }

    #[test]
    fn test_schedule_daily_serialization() {
        let schedule = Schedule::Daily {
            hour: 9,
            minute: 30,
        };
        let json = serde_json::to_string(&schedule).unwrap();
        assert!(json.contains("\"type\":\"daily\""));
        assert!(json.contains("\"hour\":9"));
        assert!(json.contains("\"minute\":30"));

        let parsed: Schedule = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, schedule);
    }

    #[test]
    fn test_schedule_weekly_serialization() {
        let schedule = Schedule::Weekly {
            weekday: Weekday::Monday,
            hour: 10,
            minute: 0,
        };
        let json = serde_json::to_string(&schedule).unwrap();
        assert!(json.contains("\"type\":\"weekly\""));
        assert!(json.contains("\"weekday\":\"monday\""));

        let parsed: Schedule = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, schedule);
    }

    #[test]
    fn test_schedule_monthly_serialization() {
        let schedule = Schedule::Monthly {
            day: 15,
            hour: 12,
            minute: 0,
        };
        let json = serde_json::to_string(&schedule).unwrap();
        assert!(json.contains("\"type\":\"monthly\""));
        assert!(json.contains("\"day\":15"));

        let parsed: Schedule = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, schedule);
    }

    #[test]
    fn test_weekday_to_chrono() {
        assert_eq!(Weekday::Monday.to_chrono(), chrono::Weekday::Mon);
        assert_eq!(Weekday::Tuesday.to_chrono(), chrono::Weekday::Tue);
        assert_eq!(Weekday::Wednesday.to_chrono(), chrono::Weekday::Wed);
        assert_eq!(Weekday::Thursday.to_chrono(), chrono::Weekday::Thu);
        assert_eq!(Weekday::Friday.to_chrono(), chrono::Weekday::Fri);
        assert_eq!(Weekday::Saturday.to_chrono(), chrono::Weekday::Sat);
        assert_eq!(Weekday::Sunday.to_chrono(), chrono::Weekday::Sun);
    }
}
