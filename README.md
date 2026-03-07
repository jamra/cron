# Cron

A container-based job scheduler written in Rust. Submit Dockerfiles and code via API, run them on configurable schedules.

## Features

- **Inline Code Submission** - POST your Dockerfile and source files directly to the API
- **Container Isolation** - Each job runs in its own Docker container
- **Flexible Scheduling** - Interval, daily, weekly, or monthly schedules
- **Configurable Timeouts** - Kill jobs that exceed their time limit
- **Automatic Retries** - Exponential backoff on failure
- **Efficient Scheduling** - Event-driven min-heap architecture (not polling)
- **Persistent Storage** - SQLite for metadata, filesystem for logs

## Quick Start

```bash
# Build
cargo build --release

# Run (requires Docker)
./target/release/scheduler

# API starts on http://localhost:3000
```

## API Usage

### Create a Job

```bash
curl -X POST http://localhost:3000/api/jobs \
  -H "Content-Type: application/json" \
  -d '{
    "name": "hello-world",
    "dockerfile": "FROM alpine:latest\nCMD [\"echo\", \"Hello from cron!\"]",
    "schedule": {
      "type": "interval",
      "minutes": 30
    }
  }'
```

### Create a Job with Source Files

```bash
curl -X POST http://localhost:3000/api/jobs \
  -H "Content-Type: application/json" \
  -d '{
    "name": "python-task",
    "dockerfile": "FROM python:3.11-slim\nWORKDIR /app\nCOPY . .\nCMD [\"python\", \"main.py\"]",
    "files": {
      "main.py": "import datetime\nprint(f\"Running at {datetime.datetime.now()}\")"
    },
    "schedule": {
      "type": "daily",
      "hour": 9,
      "minute": 0
    }
  }'
```

### Trigger a Manual Run

```bash
curl -X POST http://localhost:3000/api/jobs/{job_id}/trigger
```

### View Logs

```bash
curl http://localhost:3000/api/runs/{run_id}/logs
```

### List Jobs

```bash
curl http://localhost:3000/api/jobs
```

## Job Configuration

| Field | Required | Default | Description |
|-------|----------|---------|-------------|
| `name` | Yes | - | Job name |
| `description` | No | null | Job description |
| `dockerfile` | Yes | - | Dockerfile contents |
| `files` | No | {} | Map of filename to file contents |
| `schedule` | No | null | When to run (null = manual only) |
| `enabled` | No | true | Whether job is active |
| `timeout_secs` | No | null | Max execution time before killing |
| `max_retries` | No | 0 | Retry attempts on failure |
| `retry_delay_secs` | No | 60 | Base delay between retries |

## Schedule Types

```js
// Every 30 minutes
{ "type": "interval", "minutes": 30 }

// Every 2 hours
{ "type": "interval", "hours": 2 }

// Daily at 9:00 AM UTC
{ "type": "daily", "hour": 9, "minute": 0 }

// Every Monday at 10:00 AM UTC
{ "type": "weekly", "weekday": "monday", "hour": 10, "minute": 0 }

// 1st of each month at midnight
{ "type": "monthly", "day": 1, "hour": 0, "minute": 0 }
```

## Timeouts

Jobs can have a timeout to kill frozen or runaway containers:

```json
{
  "name": "risky-job",
  "dockerfile": "FROM alpine\nCMD [\"./might-hang.sh\"]",
  "timeout_secs": 300
}
```

If the job exceeds 300 seconds, the container is killed and the run status becomes `timed_out`.

## Retries

Failed jobs can automatically retry with exponential backoff:

```json
{
  "name": "flaky-job",
  "dockerfile": "FROM alpine\nCMD [\"./sometimes-fails.sh\"]",
  "max_retries": 3,
  "retry_delay_secs": 60
}
```

Retry delays: 60s → 120s → 240s (formula: `delay * 2^attempt`)

## Run Statuses

| Status | Description |
|--------|-------------|
| `pending` | Queued, waiting to start |
| `running` | Currently executing |
| `succeeded` | Completed with exit code 0 |
| `failed` | Completed with non-zero exit code |
| `timed_out` | Killed after exceeding timeout |
| `retrying` | Failed, waiting to retry |
| `cancelled` | Manually cancelled |

## API Reference

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/api/jobs` | Create a job |
| `GET` | `/api/jobs` | List all jobs |
| `GET` | `/api/jobs/:id` | Get job details |
| `PUT` | `/api/jobs/:id` | Update a job |
| `DELETE` | `/api/jobs/:id` | Delete a job |
| `POST` | `/api/jobs/:id/trigger` | Trigger manual run |
| `GET` | `/api/jobs/:id/runs` | List runs for a job |
| `GET` | `/api/runs/:id` | Get run details |
| `GET` | `/api/runs/:id/logs` | Get stdout/stderr |
| `DELETE` | `/api/runs/:id` | Cancel a running job |

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                      REST API (Axum)                        │
│         POST /jobs  ·  GET /runs/:id/logs  ·  ...           │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│                      Core Services                          │
│  ┌─────────────┐  ┌──────────────┐  ┌───────────────────┐   │
│  │ Repository  │  │  Scheduler   │  │     Executor      │   │
│  │   (CRUD)    │  │  (min-heap)  │  │ (Docker/Bollard)  │   │
│  └─────────────┘  └──────────────┘  └───────────────────┘   │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│                      Storage Layer                          │
│  ┌─────────────────────┐    ┌───────────────────────────┐   │
│  │   SQLite (sqlx)     │    │      File System          │   │
│  │  · jobs             │    │  · logs/{run_id}/stdout   │   │
│  │  · runs             │    │  · logs/{run_id}/stderr   │   │
│  └─────────────────────┘    └───────────────────────────┘   │
└─────────────────────────────────────────────────────────────┘
```

## How the Scheduler Works

The scheduler uses an **event-driven min-heap** instead of polling:

1. All scheduled jobs are loaded into a binary min-heap, ordered by next run time
2. The scheduler sleeps until the earliest job is due
3. When a job's time arrives, it's popped from the heap, executed, and re-inserted with its next scheduled time
4. The scheduler wakes immediately when jobs are added/updated

| Operation | Time Complexity |
|-----------|-----------------|
| Get next job | O(1) |
| Execute + reschedule | O(log n) |
| Add/update job | O(log n) |

This is more efficient than polling, especially with many jobs or long intervals.

## Configuration

Environment variables:

| Variable | Default | Description |
|----------|---------|-------------|
| `DATABASE_URL` | `sqlite:./data/scheduler.db` | SQLite database path |
| `SCHEDULER_LOGS_DIR` | `./logs` | Directory for run logs |
| `SCHEDULER_WORK_DIR` | `./work` | Temp directory for builds |
| `SCHEDULER_HOST` | `0.0.0.0` | API listen address |
| `SCHEDULER_PORT` | `3000` | API listen port |

## Project Structure

```
src/
├── main.rs           # Entry point
├── config.rs         # Environment configuration
├── db/
│   ├── mod.rs        # Database pool + migrations
│   └── repository.rs # CRUD operations
├── models/
│   ├── job.rs        # Job, Schedule types
│   └── run.rs        # Run, RunStatus types
├── api/
│   ├── mod.rs        # Router setup
│   ├── jobs.rs       # Job endpoints
│   ├── runs.rs       # Run endpoints
│   └── error.rs      # Error handling
├── scheduler/
│   ├── queue.rs      # Min-heap priority queue
│   └── tick.rs       # Scheduler loop
└── executor/
    ├── mod.rs        # Execution orchestration
    └── docker.rs     # Docker build/run via Bollard
```

## License

MIT
