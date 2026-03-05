use chrono::{DateTime, Utc};
use std::cmp::Ordering;
use std::collections::BinaryHeap;
use uuid::Uuid;

/// A scheduled job entry in the priority queue.
/// Ordered by next_run time (earliest first).
#[derive(Debug, Clone)]
pub struct ScheduledJob {
    pub next_run: DateTime<Utc>,
    pub job_id: Uuid,
}

impl ScheduledJob {
    pub fn new(job_id: Uuid, next_run: DateTime<Utc>) -> Self {
        Self { next_run, job_id }
    }
}

// Implement ordering for min-heap behavior (earliest time = highest priority)
impl PartialEq for ScheduledJob {
    fn eq(&self, other: &Self) -> bool {
        self.next_run == other.next_run && self.job_id == other.job_id
    }
}

impl Eq for ScheduledJob {}

impl PartialOrd for ScheduledJob {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ScheduledJob {
    fn cmp(&self, other: &Self) -> Ordering {
        // Reverse ordering for min-heap (BinaryHeap is a max-heap by default)
        // Earlier times should have higher priority
        other.next_run.cmp(&self.next_run)
            .then_with(|| self.job_id.cmp(&other.job_id))
    }
}

/// A min-heap priority queue for scheduled jobs.
///
/// # Time Complexity
/// - `push`: O(log n)
/// - `pop`: O(log n)
/// - `peek`: O(1)
/// - `remove`: O(n) - requires rebuild
///
/// # How it works
///
/// The queue maintains jobs sorted by their next execution time.
/// The job with the earliest `next_run` is always at the top.
///
/// When a job completes, we calculate its next run time and re-insert it.
/// When a job is updated/deleted, we remove it and optionally re-insert.
#[derive(Debug, Default)]
pub struct JobQueue {
    heap: BinaryHeap<ScheduledJob>,
}

impl JobQueue {
    pub fn new() -> Self {
        Self {
            heap: BinaryHeap::new(),
        }
    }

    /// Add a job to the queue. O(log n)
    pub fn push(&mut self, job: ScheduledJob) {
        self.heap.push(job);
    }

    /// Get the next job to run without removing it. O(1)
    pub fn peek(&self) -> Option<&ScheduledJob> {
        self.heap.peek()
    }

    /// Remove and return the next job to run. O(log n)
    pub fn pop(&mut self) -> Option<ScheduledJob> {
        self.heap.pop()
    }

    /// Remove a specific job by ID. O(n) - rebuilds the heap.
    /// Returns true if the job was found and removed.
    pub fn remove(&mut self, job_id: Uuid) -> bool {
        let original_len = self.heap.len();
        let items: Vec<_> = self.heap.drain().filter(|j| j.job_id != job_id).collect();
        self.heap.extend(items);
        self.heap.len() < original_len
    }

    /// Update a job's next run time. O(n) due to remove + O(log n) insert.
    #[allow(dead_code)]
    pub fn update(&mut self, job_id: Uuid, next_run: DateTime<Utc>) {
        self.remove(job_id);
        self.push(ScheduledJob::new(job_id, next_run));
    }

    /// Check if the queue is empty. O(1)
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.heap.is_empty()
    }

    /// Get the number of scheduled jobs. O(1)
    pub fn len(&self) -> usize {
        self.heap.len()
    }

    /// Check if a job exists in the queue. O(n)
    #[allow(dead_code)]
    pub fn contains(&self, job_id: Uuid) -> bool {
        self.heap.iter().any(|j| j.job_id == job_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn test_min_heap_ordering() {
        let mut queue = JobQueue::new();

        let job1 = ScheduledJob::new(
            Uuid::new_v4(),
            Utc.with_ymd_and_hms(2024, 1, 1, 12, 0, 0).unwrap(),
        );
        let job2 = ScheduledJob::new(
            Uuid::new_v4(),
            Utc.with_ymd_and_hms(2024, 1, 1, 10, 0, 0).unwrap(), // Earlier
        );
        let job3 = ScheduledJob::new(
            Uuid::new_v4(),
            Utc.with_ymd_and_hms(2024, 1, 1, 14, 0, 0).unwrap(),
        );

        queue.push(job1);
        queue.push(job2.clone());
        queue.push(job3);

        // Should pop in chronological order (earliest first)
        let first = queue.pop().unwrap();
        assert_eq!(first.job_id, job2.job_id);
        assert_eq!(first.next_run, Utc.with_ymd_and_hms(2024, 1, 1, 10, 0, 0).unwrap());
    }

    #[test]
    fn test_remove_job() {
        let mut queue = JobQueue::new();
        let job_id = Uuid::new_v4();

        queue.push(ScheduledJob::new(
            job_id,
            Utc.with_ymd_and_hms(2024, 1, 1, 10, 0, 0).unwrap(),
        ));
        queue.push(ScheduledJob::new(
            Uuid::new_v4(),
            Utc.with_ymd_and_hms(2024, 1, 1, 12, 0, 0).unwrap(),
        ));

        assert_eq!(queue.len(), 2);
        assert!(queue.remove(job_id));
        assert_eq!(queue.len(), 1);
        assert!(!queue.contains(job_id));
    }

    #[test]
    fn test_update_job() {
        let mut queue = JobQueue::new();
        let job_id = Uuid::new_v4();

        queue.push(ScheduledJob::new(
            job_id,
            Utc.with_ymd_and_hms(2024, 1, 1, 10, 0, 0).unwrap(),
        ));

        let new_time = Utc.with_ymd_and_hms(2024, 1, 1, 15, 0, 0).unwrap();
        queue.update(job_id, new_time);

        let job = queue.peek().unwrap();
        assert_eq!(job.next_run, new_time);
    }

    #[test]
    fn test_peek_does_not_remove() {
        let mut queue = JobQueue::new();
        queue.push(ScheduledJob::new(
            Uuid::new_v4(),
            Utc.with_ymd_and_hms(2024, 1, 1, 10, 0, 0).unwrap(),
        ));

        assert!(queue.peek().is_some());
        assert!(queue.peek().is_some());
        assert_eq!(queue.len(), 1);
    }
}
