# Job Scheduler

A Rust-based job scheduling system for running container-based jobs on configurable schedules. Think of it as a self-hosted hybrid of BuildKite and cron.

## Features

- **Container Execution**: Jobs are defined as Git repos with Dockerfiles
- **Flexible Scheduling**: Interval, daily, weekly, and monthly schedules
- **Automatic Retries**: Configurable retry count with exponential backoff
- **REST API**: Full CRUD for jobs and runs
- **Event-Driven Scheduler**: Uses a min-heap priority queue for efficient scheduling
- **Persistent Storage**: SQLite for metadata, filesystem for logs

## Quick Start

```bash
# Build
cargo build --release

# Run (requires Docker)
./target/release/scheduler

# The API starts on http://localhost:3000
```

## API Examples

### Create a Job

```bash
curl -X POST http://localhost:3000/api/jobs \
  -H "Content-Type: application/json" \
  -d '{
    "name": "daily-backup",
    "git_repo": "https://github.com/user/backup-scripts",
    "git_ref": "main",
    "dockerfile_path": "Dockerfile",
    "schedule": {
      "type": "daily",
      "hour": 2,
      "minute": 0
    },
    "max_retries": 3,
    "retry_delay_secs": 60
  }'
```

### Schedule Types

```json
// Run every 30 minutes
{ "type": "interval", "minutes": 30 }

// Run every 2 hours
{ "type": "interval", "hours": 2 }

// Run daily at 9:00 AM UTC
{ "type": "daily", "hour": 9, "minute": 0 }

// Run every Monday at 10:00 AM UTC
{ "type": "weekly", "weekday": "monday", "hour": 10, "minute": 0 }

// Run on the 1st of each month at midnight
{ "type": "monthly", "day": 1, "hour": 0, "minute": 0 }
```

### Retry Configuration

Jobs can be configured to automatically retry on failure with exponential backoff:

```json
{
  "max_retries": 3,
  "retry_delay_secs": 60
}
```

| Field | Default | Description |
|-------|---------|-------------|
| `max_retries` | 0 | Number of retry attempts after initial failure (0 = no retries) |
| `retry_delay_secs` | 60 | Base delay between retries in seconds |

**Backoff formula**: `delay = retry_delay_secs * 2^(attempt - 1)`

Example with `max_retries: 3` and `retry_delay_secs: 60`:

```
Attempt 1: runs immediately
    ↓ (fails)
Attempt 2: waits 60s  (60 * 2^0)
    ↓ (fails)
Attempt 3: waits 120s (60 * 2^1)
    ↓ (fails)
Attempt 4: waits 240s (60 * 2^2)
    ↓ (fails)
Job marked as Failed (no more retries)
```

Run statuses:
- `pending` - Waiting to start
- `running` - Currently executing
- `succeeded` - Completed with exit code 0
- `failed` - Completed with non-zero exit code (after all retries exhausted)
- `retrying` - Failed but will retry
- `cancelled` - Manually cancelled

### Trigger a Manual Run

```bash
curl -X POST http://localhost:3000/api/jobs/{job_id}/trigger
```

### View Logs

```bash
curl http://localhost:3000/api/runs/{run_id}/logs
```

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                        Web API (Axum)                       │
│  POST /jobs, GET /jobs, GET /jobs/:id/runs, GET /logs/:id   │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│                      Core Services                          │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────────────┐  │
│  │ JobService  │  │ Scheduler   │  │ Executor            │  │
│  │ (CRUD)      │  │ (min-heap)  │  │ (Docker + git)      │  │
│  └─────────────┘  └─────────────┘  └─────────────────────┘  │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│                      Storage Layer                          │
│  ┌─────────────────────┐  ┌──────────────────────────────┐  │
│  │ SQLite              │  │ File System                  │  │
│  │ - jobs              │  │ - /logs/{run_id}/stdout.log  │  │
│  │ - runs              │  │ - /logs/{run_id}/stderr.log  │  │
│  └─────────────────────┘  └──────────────────────────────┘  │
└─────────────────────────────────────────────────────────────┘
```

## How the Scheduler Works

The scheduler uses an **event-driven min-heap architecture** instead of polling. This is significantly more efficient for large numbers of jobs.

### The Min-Heap Priority Queue

```
                    ┌─────────────────────┐
                    │  JobQueue (Heap)    │
                    └─────────────────────┘
                              │
              ┌───────────────┼───────────────┐
              ▼               ▼               ▼
         ┌────────┐      ┌────────┐      ┌────────┐
         │ Job A  │      │ Job B  │      │ Job C  │
         │ 10:00  │      │ 10:30  │      │ 14:00  │
         └────────┘      └────────┘      └────────┘
              ▲
              │
         Next to run (heap top)
```

Jobs are stored in a binary min-heap, ordered by their `next_run` time. The job with the earliest scheduled time is always at the top.

### The Algorithm

```rust
loop {
    // 1. Peek at the next job (O(1))
    let next_job = queue.peek();

    // 2. Sleep until it's time (no CPU usage while waiting)
    sleep_until(next_job.next_run).await;

    // 3. Pop and execute (O(log n))
    let job = queue.pop();
    execute(job);

    // 4. Calculate next run and re-insert (O(log n))
    let next_run = calculate_next_run(job.schedule, now);
    queue.push(ScheduledJob { job_id, next_run });
}
```

### Why Not Polling?

| Approach | Per-tick Cost | Wake Frequency | CPU While Idle |
|----------|---------------|----------------|----------------|
| Polling (scan all jobs every minute) | O(n) | Fixed (every 60s) | Higher |
| Min-heap (sleep until next job) | O(log n) | On-demand | Near zero |

For 10,000 jobs:
- **Polling**: Scan 10,000 jobs every minute = 10,000 checks/minute
- **Min-heap**: Only wake when a job is due = ~number of job executions

### Time Complexity

| Operation | Complexity | Notes |
|-----------|------------|-------|
| Startup | O(n log n) | Build heap from all jobs |
| Get next job | O(1) | Peek at heap root |
| Execute job | O(log n) | Pop from heap |
| Reschedule job | O(log n) | Push to heap |
| Add new job | O(log n) | Push to heap |
| Remove job | O(n) | Must search and rebuild |
| Update job | O(n) | Remove + insert |

### Handling Updates

When a job is created, updated, or deleted via the API:

```rust
// Create: Add to heap, notify scheduler
queue.push(new_job);
scheduler.notify();  // Wakes the scheduler to re-check

// Update: Remove old entry, add new one
queue.remove(job_id);  // O(n)
queue.push(updated_job);  // O(log n)
scheduler.notify();

// Delete: Remove from heap
queue.remove(job_id);  // O(n)
```

The `notify()` call uses a `tokio::sync::Notify` to wake the scheduler immediately if a new job might need to run sooner than the current sleep target.

## Execution Flow

When a job runs:

```
1. Clone git repo (shallow clone with --depth 1)
      ↓
2. Build Docker image from Dockerfile
      ↓
3. Create and start container
      ↓
4. Stream stdout/stderr to log files
      ↓
5. Wait for container exit
      ↓
6. Record exit code, update run status
      ↓
7. Cleanup: remove container, image, and cloned repo
```

## Project Structure

```
scheduler/
├── Cargo.toml
├── src/
│   ├── main.rs              # Entry point
│   ├── config.rs            # Environment configuration
│   ├── db/
│   │   ├── mod.rs           # Database pool + migrations
│   │   └── repository.rs    # CRUD operations
│   ├── models/
│   │   ├── mod.rs
│   │   ├── job.rs           # Job, Schedule types
│   │   └── run.rs           # Run, RunStatus types
│   ├── api/
│   │   ├── mod.rs           # Router setup
│   │   ├── error.rs         # Error handling
│   │   ├── jobs.rs          # Job endpoints
│   │   └── runs.rs          # Run endpoints
│   ├── scheduler/
│   │   ├── mod.rs
│   │   ├── queue.rs         # Min-heap implementation
│   │   └── tick.rs          # Scheduler loop
│   └── executor/
│       ├── mod.rs           # Execution orchestration
│       ├── docker.rs        # Docker build/run
│       └── git.rs           # Git clone
├── logs/                    # Run logs (gitignored)
├── data/                    # SQLite database (gitignored)
└── work/                    # Temp git clones (gitignored)
```

## Configuration

Environment variables:

| Variable | Default | Description |
|----------|---------|-------------|
| `DATABASE_URL` | `sqlite:./data/scheduler.db` | SQLite database path |
| `SCHEDULER_LOGS_DIR` | `./logs` | Directory for run logs |
| `SCHEDULER_WORK_DIR` | `./work` | Directory for git clones |
| `SCHEDULER_HOST` | `0.0.0.0` | API listen address |
| `SCHEDULER_PORT` | `3000` | API listen port |

## API Reference

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/api/jobs` | Create a job |
| `GET` | `/api/jobs` | List all jobs |
| `GET` | `/api/jobs/:id` | Get job details |
| `PUT` | `/api/jobs/:id` | Update a job |
| `DELETE` | `/api/jobs/:id` | Delete a job |
| `POST` | `/api/jobs/:id/trigger` | Trigger a manual run |
| `GET` | `/api/jobs/:id/runs` | List runs for a job |
| `GET` | `/api/runs/:id` | Get run details |
| `GET` | `/api/runs/:id/logs` | Get run logs |
| `DELETE` | `/api/runs/:id` | Cancel a running job |

## Scaling Considerations

### Current Limitations

- **Single process**: No distributed execution
- **O(n) remove**: Updating/deleting jobs requires heap rebuild
- **In-memory queue**: Queue state lost on restart (rebuilt from DB)

### Future Improvements

For very large scale (10,000+ jobs), consider:

1. **Timing Wheel**: O(1) insertion for high-frequency scheduling
   ```
   [Second 0] -> [Job A, Job B]
   [Second 1] -> [Job C]
   [Second 2] -> []
   ...
   [Second 59] -> [Job D]
   ```

2. **Sharding**: Partition jobs across multiple scheduler instances
   ```
   Scheduler 1: Jobs A-M
   Scheduler 2: Jobs N-Z
   ```

3. **Lock-free structures**: Use `crossbeam` for concurrent access

4. **Persistent queue**: Store queue state in Redis for fast recovery

## Testing

```bash
# Run all tests
cargo test

# Run with output
cargo test -- --nocapture
```

## License

MIT
