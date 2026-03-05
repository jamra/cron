use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Job {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub dockerfile: String,
    pub files: HashMap<String, String>,
    pub schedule: Option<Schedule>,
    pub enabled: bool,
    pub max_retries: u32,
    pub retry_delay_secs: u32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Schedule {
    Interval {
        #[serde(skip_serializing_if = "Option::is_none")]
        minutes: Option<u32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        hours: Option<u32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        days: Option<u32>,
    },
    Daily {
        hour: u8,
        minute: u8,
    },
    Weekly {
        weekday: Weekday,
        hour: u8,
        minute: u8,
    },
    Monthly {
        day: u8,
        hour: u8,
        minute: u8,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Weekday {
    Monday,
    Tuesday,
    Wednesday,
    Thursday,
    Friday,
    Saturday,
    Sunday,
}

impl Weekday {
    pub fn to_chrono(&self) -> chrono::Weekday {
        match self {
            Weekday::Monday => chrono::Weekday::Mon,
            Weekday::Tuesday => chrono::Weekday::Tue,
            Weekday::Wednesday => chrono::Weekday::Wed,
            Weekday::Thursday => chrono::Weekday::Thu,
            Weekday::Friday => chrono::Weekday::Fri,
            Weekday::Saturday => chrono::Weekday::Sat,
            Weekday::Sunday => chrono::Weekday::Sun,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct CreateJobRequest {
    pub name: String,
    pub description: Option<String>,
    pub dockerfile: String,
    #[serde(default)]
    pub files: HashMap<String, String>,
    pub schedule: Option<Schedule>,
    pub enabled: Option<bool>,
    pub max_retries: Option<u32>,
    pub retry_delay_secs: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateJobRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub dockerfile: Option<String>,
    pub files: Option<HashMap<String, String>>,
    pub schedule: Option<Schedule>,
    pub enabled: Option<bool>,
    pub max_retries: Option<u32>,
    pub retry_delay_secs: Option<u32>,
}
