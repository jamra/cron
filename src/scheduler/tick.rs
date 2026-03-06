use chrono::{DateTime, Datelike, Duration, Timelike, Utc};
use std::sync::Arc;
use tokio::sync::{watch, Notify, RwLock};

use super::queue::{JobQueue, ScheduledJob};
use crate::db::Repository;
use crate::executor::Executor;
use crate::models::{Job, Run, Schedule, TriggerType};

/// The Scheduler manages job execution timing using a min-heap priority queue.
///
/// # Architecture
///
/// ```text
/// ┌─────────────────────────────────────────────────────────────┐
/// │                     Scheduler                               │
/// │  ┌─────────────────────────────────────────────────────┐    │
/// │  │              JobQueue (Min-Heap)                    │    │
/// │  │  ┌─────┐ ┌─────┐ ┌─────┐ ┌─────┐                   │    │
/// │  │  │Job A│ │Job B│ │Job C│ │Job D│  ... (by time)    │    │
/// │  │  │10:00│ │10:30│ │11:00│ │14:00│                   │    │
/// │  │  └─────┘ └─────┘ └─────┘ └─────┘                   │    │
/// │  └─────────────────────────────────────────────────────┘    │
/// │                         │                                   │
/// │                         ▼                                   │
/// │  ┌─────────────────────────────────────────────────────┐    │
/// │  │  sleep_until(next_job.time) ──► Execute ──► Reschedule  │
/// │  └─────────────────────────────────────────────────────┘    │
/// └─────────────────────────────────────────────────────────────┘
/// ```
///
/// # Algorithm
///
/// 1. On startup, load all enabled jobs with schedules from the database
/// 2. Calculate each job's next run time and insert into the min-heap
/// 3. Sleep until the earliest job's scheduled time
/// 4. Wake up, pop the job, execute it
/// 5. Calculate the job's next run time and re-insert into the heap
/// 6. Repeat from step 3
///
/// # Time Complexity
///
/// - Startup: O(n log n) to build the heap
/// - Per execution: O(log n) for pop + insert
/// - Job update/delete: O(n) to find and remove, O(log n) to insert
///
/// # Comparison with Polling
///
/// | Approach | Per-tick cost | Wake frequency | CPU idle |
/// |----------|---------------|----------------|----------|
/// | Polling  | O(n)          | Fixed interval | Low      |
/// | Min-heap | O(log n)      | On-demand      | High     |
///
pub struct Scheduler {
    repo: Repository,
    executor: Arc<Executor>,
    queue: Arc<RwLock<JobQueue>>,
    notify: Arc<Notify>,
}

impl Scheduler {
    pub fn new(repo: Repository, executor: Arc<Executor>) -> Self {
        Self {
            repo,
            executor,
            queue: Arc::new(RwLock::new(JobQueue::new())),
            notify: Arc::new(Notify::new()),
        }
    }

    /// Initialize the scheduler by loading all scheduled jobs into the heap.
    pub async fn initialize(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let jobs = self.repo.list_enabled_jobs_with_schedule().await?;
        let now = Utc::now();

        let mut queue = self.queue.write().await;
        for job in jobs {
            if let Some(next_run) = self.calculate_initial_run_time(&job, now).await? {
                tracing::debug!(
                    "Scheduling job {} ({}) for {}",
                    job.name,
                    job.id,
                    next_run
                );
                queue.push(ScheduledJob::new(job.id, next_run));
            }
        }

        tracing::info!("Initialized scheduler with {} jobs", queue.len());
        Ok(())
    }

    /// Calculate when a job should first run.
    /// If the job has run before, calculate next run from the last run.
    /// Otherwise, check if it should run now based on schedule type.
    async fn calculate_initial_run_time(
        &self,
        job: &Job,
        now: DateTime<Utc>,
    ) -> Result<Option<DateTime<Utc>>, Box<dyn std::error::Error + Send + Sync>> {
        let schedule = match &job.schedule {
            Some(s) => s,
            None => return Ok(None),
        };

        let last_run = self.repo.get_last_run_for_job(job.id).await?;

        let next_run = match last_run {
            Some(run) => calculate_next_run(schedule, run.created_at),
            None => {
                // Never run before - calculate next run from now
                calculate_next_run(schedule, now)
            }
        };

        Ok(Some(next_run))
    }

    /// Main scheduler loop using event-driven wakeups.
    ///
    /// Instead of polling at fixed intervals, we sleep until the next
    /// scheduled job and only wake when needed.
    pub async fn run(&self, mut shutdown: watch::Receiver<bool>) {
        tracing::info!("Starting event-driven scheduler");

        loop {
            // Get the next scheduled job
            let next_job = {
                let queue = self.queue.read().await;
                queue.peek().cloned()
            };

            match next_job {
                Some(scheduled) => {
                    let now = Utc::now();
                    let wait_duration = (scheduled.next_run - now)
                        .to_std()
                        .unwrap_or(std::time::Duration::ZERO);

                    tracing::debug!(
                        "Next job {} scheduled for {} (waiting {:?})",
                        scheduled.job_id,
                        scheduled.next_run,
                        wait_duration
                    );

                    tokio::select! {
                        // Wait until the scheduled time
                        _ = tokio::time::sleep(wait_duration) => {
                            self.execute_due_jobs().await;
                        }
                        // Or wake up if notified (new job added, job updated)
                        _ = self.notify.notified() => {
                            tracing::debug!("Scheduler notified, rechecking queue");
                        }
                        // Or shutdown
                        _ = shutdown.changed() => {
                            if *shutdown.borrow() {
                                tracing::info!("Scheduler shutting down");
                                return;
                            }
                        }
                    }
                }
                None => {
                    // No jobs scheduled, wait for notification
                    tracing::debug!("No jobs scheduled, waiting for notification");
                    tokio::select! {
                        _ = self.notify.notified() => {
                            tracing::debug!("Scheduler notified, checking for new jobs");
                        }
                        _ = shutdown.changed() => {
                            if *shutdown.borrow() {
                                tracing::info!("Scheduler shutting down");
                                return;
                            }
                        }
                    }
                }
            }
        }
    }

    /// Execute all jobs that are due (next_run <= now).
    async fn execute_due_jobs(&self) {
        let now = Utc::now();

        loop {
            // Check if the next job is due
            let job_to_run = {
                let mut queue = self.queue.write().await;
                match queue.peek() {
                    Some(scheduled) if scheduled.next_run <= now => queue.pop(),
                    _ => None,
                }
            };

            match job_to_run {
                Some(scheduled) => {
                    // Fetch the job details and execute
                    match self.repo.get_job(scheduled.job_id).await {
                        Ok(Some(job)) => {
                            tracing::info!("Executing scheduled job: {} ({})", job.name, job.id);
                            if let Err(e) = self.trigger_run(&job).await {
                                tracing::error!("Failed to trigger job {}: {}", job.id, e);
                            }

                            // Reschedule the job
                            if let Some(schedule) = &job.schedule {
                                let next_run = calculate_next_run(schedule, now);
                                tracing::debug!("Rescheduling job {} for {}", job.id, next_run);
                                let mut queue = self.queue.write().await;
                                queue.push(ScheduledJob::new(job.id, next_run));
                            }
                        }
                        Ok(None) => {
                            tracing::warn!("Job {} no longer exists, skipping", scheduled.job_id);
                        }
                        Err(e) => {
                            tracing::error!("Failed to fetch job {}: {}", scheduled.job_id, e);
                        }
                    }
                }
                None => break, // No more due jobs
            }
        }
    }

    async fn trigger_run(&self, job: &Job) -> Result<Run, Box<dyn std::error::Error + Send + Sync>> {
        let run = self.repo.create_run(job.id, TriggerType::Scheduled).await?;

        let executor = self.executor.clone();
        let repo = self.repo.clone();
        let job = job.clone();
        let run_clone = run.clone();

        tokio::spawn(async move {
            if let Err(e) = executor.execute_run(&repo, &job, &run_clone).await {
                tracing::error!("Failed to execute scheduled run {}: {}", run_clone.id, e);
            }
        });

        Ok(run)
    }

    /// Notify the scheduler that a job was added or updated.
    /// This wakes the scheduler to recheck the queue.
    pub fn notify_change(&self) {
        self.notify.notify_one();
    }

    /// Add a job to the schedule queue.
    pub async fn schedule_job(&self, job: &Job) {
        if let Some(schedule) = &job.schedule {
            if job.enabled {
                let now = Utc::now();
                let last_run = self.repo.get_last_run_for_job(job.id).await.ok().flatten();
                let next_run = match last_run {
                    Some(run) => calculate_next_run(schedule, run.created_at),
                    None => calculate_next_run(schedule, now),
                };

                let mut queue = self.queue.write().await;
                queue.remove(job.id); // Remove if already exists
                queue.push(ScheduledJob::new(job.id, next_run));
                drop(queue);

                self.notify_change();
            }
        }
    }

    /// Remove a job from the schedule queue.
    pub async fn unschedule_job(&self, job_id: uuid::Uuid) {
        let mut queue = self.queue.write().await;
        queue.remove(job_id);
    }
}

/// Calculate the next run time based on the schedule and last run time.
///
/// # Time Complexity: O(1)
///
/// Each schedule type uses simple arithmetic to compute the next time.
pub fn calculate_next_run(schedule: &Schedule, last_run: DateTime<Utc>) -> DateTime<Utc> {
    match schedule {
        Schedule::Interval { minutes, hours, days } => {
            // Sum up all interval components
            let mut duration = Duration::zero();
            if let Some(m) = minutes {
                duration = duration + Duration::minutes(*m as i64);
            }
            if let Some(h) = hours {
                duration = duration + Duration::hours(*h as i64);
            }
            if let Some(d) = days {
                duration = duration + Duration::days(*d as i64);
            }
            // Default to 1 hour if no interval specified
            if duration.is_zero() {
                duration = Duration::hours(1);
            }
            last_run + duration
        }
        Schedule::Daily { hour, minute } => {
            // Find next occurrence of this time
            let mut next = last_run
                .date_naive()
                .and_hms_opt(*hour as u32, *minute as u32, 0)
                .unwrap()
                .and_utc();
            if next <= last_run {
                next = next + Duration::days(1);
            }
            next
        }
        Schedule::Weekly { weekday, hour, minute } => {
            let target_weekday = weekday.to_chrono();
            let mut next = last_run
                .date_naive()
                .and_hms_opt(*hour as u32, *minute as u32, 0)
                .unwrap()
                .and_utc();

            // Find the next occurrence of the target weekday
            while next.weekday() != target_weekday || next <= last_run {
                next = next + Duration::days(1);
            }
            next
        }
        Schedule::Monthly { day, hour, minute } => {
            let mut next = last_run
                .date_naive()
                .with_day(*day as u32)
                .unwrap_or(last_run.date_naive())
                .and_hms_opt(*hour as u32, *minute as u32, 0)
                .unwrap()
                .and_utc();

            if next <= last_run {
                // Move to next month
                let next_month = if last_run.month() == 12 {
                    next.with_year(last_run.year() + 1)
                        .and_then(|d| d.with_month(1))
                } else {
                    next.with_month(last_run.month() + 1)
                };
                next = next_month.unwrap_or(next + Duration::days(30));
            }
            next
        }
    }
}

/// Check if the current time matches a scheduled time (for first-run detection).
#[allow(dead_code)]
pub fn is_scheduled_time(schedule: &Schedule, now: DateTime<Utc>) -> bool {
    match schedule {
        Schedule::Interval { .. } => true,
        Schedule::Daily { hour, minute } => {
            now.hour() == *hour as u32 && now.minute() == *minute as u32
        }
        Schedule::Weekly { weekday, hour, minute } => {
            now.weekday() == weekday.to_chrono()
                && now.hour() == *hour as u32
                && now.minute() == *minute as u32
        }
        Schedule::Monthly { day, hour, minute } => {
            now.day() == *day as u32
                && now.hour() == *hour as u32
                && now.minute() == *minute as u32
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Weekday;
    use chrono::TimeZone;

    // Interval schedule tests

    #[test]
    fn test_interval_minutes_next_run() {
        let schedule = Schedule::Interval {
            minutes: Some(30),
            hours: None,
            days: None,
        };
        let last_run = Utc.with_ymd_and_hms(2024, 1, 1, 10, 0, 0).unwrap();
        let next = calculate_next_run(&schedule, last_run);
        assert_eq!(next, Utc.with_ymd_and_hms(2024, 1, 1, 10, 30, 0).unwrap());
    }

    #[test]
    fn test_interval_hours_next_run() {
        let schedule = Schedule::Interval {
            minutes: None,
            hours: Some(2),
            days: None,
        };
        let last_run = Utc.with_ymd_and_hms(2024, 1, 1, 10, 0, 0).unwrap();
        let next = calculate_next_run(&schedule, last_run);
        assert_eq!(next, Utc.with_ymd_and_hms(2024, 1, 1, 12, 0, 0).unwrap());
    }

    #[test]
    fn test_interval_days_next_run() {
        let schedule = Schedule::Interval {
            minutes: None,
            hours: None,
            days: Some(1),
        };
        let last_run = Utc.with_ymd_and_hms(2024, 1, 1, 10, 0, 0).unwrap();
        let next = calculate_next_run(&schedule, last_run);
        assert_eq!(next, Utc.with_ymd_and_hms(2024, 1, 2, 10, 0, 0).unwrap());
    }

    #[test]
    fn test_interval_combined_next_run() {
        let schedule = Schedule::Interval {
            minutes: Some(30),
            hours: Some(1),
            days: Some(1),
        };
        let last_run = Utc.with_ymd_and_hms(2024, 1, 1, 10, 0, 0).unwrap();
        let next = calculate_next_run(&schedule, last_run);
        assert_eq!(next, Utc.with_ymd_and_hms(2024, 1, 2, 11, 30, 0).unwrap());
    }

    #[test]
    fn test_interval_default_one_hour() {
        let schedule = Schedule::Interval {
            minutes: None,
            hours: None,
            days: None,
        };
        let last_run = Utc.with_ymd_and_hms(2024, 1, 1, 10, 0, 0).unwrap();
        let next = calculate_next_run(&schedule, last_run);
        assert_eq!(next, Utc.with_ymd_and_hms(2024, 1, 1, 11, 0, 0).unwrap());
    }

    // Daily schedule tests

    #[test]
    fn test_daily_next_run() {
        let schedule = Schedule::Daily { hour: 9, minute: 0 };
        let last_run = Utc.with_ymd_and_hms(2024, 1, 1, 9, 0, 0).unwrap();
        let next = calculate_next_run(&schedule, last_run);
        assert_eq!(next, Utc.with_ymd_and_hms(2024, 1, 2, 9, 0, 0).unwrap());
    }

    #[test]
    fn test_daily_same_day_later() {
        let schedule = Schedule::Daily {
            hour: 14,
            minute: 30,
        };
        let last_run = Utc.with_ymd_and_hms(2024, 1, 1, 9, 0, 0).unwrap();
        let next = calculate_next_run(&schedule, last_run);
        assert_eq!(next, Utc.with_ymd_and_hms(2024, 1, 1, 14, 30, 0).unwrap());
    }

    #[test]
    fn test_daily_next_day() {
        let schedule = Schedule::Daily { hour: 8, minute: 0 };
        let last_run = Utc.with_ymd_and_hms(2024, 1, 1, 9, 0, 0).unwrap();
        let next = calculate_next_run(&schedule, last_run);
        assert_eq!(next, Utc.with_ymd_and_hms(2024, 1, 2, 8, 0, 0).unwrap());
    }

    // Weekly schedule tests

    #[test]
    fn test_weekly_next_run_same_week() {
        let schedule = Schedule::Weekly {
            weekday: Weekday::Friday,
            hour: 10,
            minute: 0,
        };
        let last_run = Utc.with_ymd_and_hms(2024, 1, 1, 9, 0, 0).unwrap();
        let next = calculate_next_run(&schedule, last_run);
        assert_eq!(next, Utc.with_ymd_and_hms(2024, 1, 5, 10, 0, 0).unwrap());
    }

    #[test]
    fn test_weekly_next_run_next_week() {
        let schedule = Schedule::Weekly {
            weekday: Weekday::Monday,
            hour: 10,
            minute: 0,
        };
        let last_run = Utc.with_ymd_and_hms(2024, 1, 1, 10, 0, 0).unwrap();
        let next = calculate_next_run(&schedule, last_run);
        assert_eq!(next, Utc.with_ymd_and_hms(2024, 1, 8, 10, 0, 0).unwrap());
    }

    // Monthly schedule tests

    #[test]
    fn test_monthly_next_run_same_month() {
        let schedule = Schedule::Monthly {
            day: 15,
            hour: 9,
            minute: 0,
        };
        let last_run = Utc.with_ymd_and_hms(2024, 1, 1, 9, 0, 0).unwrap();
        let next = calculate_next_run(&schedule, last_run);
        assert_eq!(next, Utc.with_ymd_and_hms(2024, 1, 15, 9, 0, 0).unwrap());
    }

    #[test]
    fn test_monthly_next_run_next_month() {
        let schedule = Schedule::Monthly {
            day: 1,
            hour: 9,
            minute: 0,
        };
        let last_run = Utc.with_ymd_and_hms(2024, 1, 1, 9, 0, 0).unwrap();
        let next = calculate_next_run(&schedule, last_run);
        assert_eq!(next, Utc.with_ymd_and_hms(2024, 2, 1, 9, 0, 0).unwrap());
    }

    #[test]
    fn test_monthly_year_rollover() {
        let schedule = Schedule::Monthly {
            day: 15,
            hour: 9,
            minute: 0,
        };
        let last_run = Utc.with_ymd_and_hms(2024, 12, 15, 9, 0, 0).unwrap();
        let next = calculate_next_run(&schedule, last_run);
        assert_eq!(next, Utc.with_ymd_and_hms(2025, 1, 15, 9, 0, 0).unwrap());
    }

    // is_scheduled_time tests

    #[test]
    fn test_is_scheduled_time_interval_always_true() {
        let schedule = Schedule::Interval {
            minutes: Some(30),
            hours: None,
            days: None,
        };
        let now = Utc.with_ymd_and_hms(2024, 1, 1, 10, 0, 0).unwrap();
        assert!(is_scheduled_time(&schedule, now));
    }

    #[test]
    fn test_is_scheduled_time_daily_match() {
        let schedule = Schedule::Daily { hour: 10, minute: 0 };
        let now = Utc.with_ymd_and_hms(2024, 1, 1, 10, 0, 0).unwrap();
        assert!(is_scheduled_time(&schedule, now));
    }

    #[test]
    fn test_is_scheduled_time_daily_no_match() {
        let schedule = Schedule::Daily { hour: 10, minute: 0 };
        let now = Utc.with_ymd_and_hms(2024, 1, 1, 11, 0, 0).unwrap();
        assert!(!is_scheduled_time(&schedule, now));
    }

    #[test]
    fn test_is_scheduled_time_weekly_match() {
        let schedule = Schedule::Weekly {
            weekday: Weekday::Monday,
            hour: 10,
            minute: 0,
        };
        let now = Utc.with_ymd_and_hms(2024, 1, 1, 10, 0, 0).unwrap();
        assert!(is_scheduled_time(&schedule, now));
    }

    #[test]
    fn test_is_scheduled_time_weekly_wrong_day() {
        let schedule = Schedule::Weekly {
            weekday: Weekday::Tuesday,
            hour: 10,
            minute: 0,
        };
        let now = Utc.with_ymd_and_hms(2024, 1, 1, 10, 0, 0).unwrap();
        assert!(!is_scheduled_time(&schedule, now));
    }

    #[test]
    fn test_is_scheduled_time_monthly_match() {
        let schedule = Schedule::Monthly {
            day: 1,
            hour: 10,
            minute: 0,
        };
        let now = Utc.with_ymd_and_hms(2024, 1, 1, 10, 0, 0).unwrap();
        assert!(is_scheduled_time(&schedule, now));
    }

    #[test]
    fn test_is_scheduled_time_monthly_wrong_day() {
        let schedule = Schedule::Monthly {
            day: 15,
            hour: 10,
            minute: 0,
        };
        let now = Utc.with_ymd_and_hms(2024, 1, 1, 10, 0, 0).unwrap();
        assert!(!is_scheduled_time(&schedule, now));
    }
}
