# cron

A Rust-based job scheduler for running container-based jobs on configurable schedules. Submit your Dockerfile and code directly via the API.

## Features

- **Inline Code Submission**: Submit Dockerfile and source files directly in the API request
- **Container Execution**: Jobs run in isolated Docker containers
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
    "name": "hello-world",
    "dockerfile": "FROM alpine:latest\nCOPY script.sh /app/\nRUN chmod +x /app/script.sh\nCMD [\"/app/script.sh\"]",
    "files": {
      "script.sh": "#!/bin/sh\necho \"Hello from cron!\"\ndate"
    },
    "schedule": {
      "type": "interval",
      "minutes": 30
    }
  }'
```

### Python Example

```bash
curl -X POST http://localhost:3000/api/jobs \
  -H "Content-Type: application/json" \
  -d '{
    "name": "python-task",
    "dockerfile": "FROM python:3.11-slim\nWORKDIR /app\nCOPY . .\nCMD [\"python\", \"main.py\"]",
    "files": {
      "main.py": "import datetime\nprint(f\"Running at {datetime.datetime.now()}\")",
      "requirements.txt": ""
    },
    "schedule": {
      "type": "daily",
      "hour": 9,
      "minute": 0
    },
    "max_retries": 3,
    "retry_delay_secs": 60
  }'
```

### Job Request Fields

| Field | Required | Default | Description |
|-------|----------|---------|-------------|
| `name` | Yes | - | Job name |
| `description` | No | null | Job description |
| `dockerfile` | Yes | - | Dockerfile contents as a string |
| `files` | No | {} | Map of filename → file contents |
| `schedule` | No | null | When to run (null = manual only) |
| `enabled` | No | true | Whether job is active |
| `max_retries` | No | 0 | Retry attempts on failure |
| `retry_delay_secs` | No | 60 | Base delay between retries |

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

## Execution Flow

When a job runs:

```
1. Create temp directory with Dockerfile and files
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
7. Cleanup: remove container, image, and temp directory
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
│  │ (CRUD)      │  │ (min-heap)  │  │ (Docker)            │  │
│  └─────────────┘  └─────────────┘  └─────────────────────┘  │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│                      Storage Layer                          │
│  ┌─────────────────────┐  ┌──────────────────────────────┐  │
│  │ SQLite              │  │ File System                  │  │
│  │ - jobs (+ code)     │  │ - /logs/{run_id}/stdout.log  │  │
│  │ - runs              │  │ - /logs/{run_id}/stderr.log  │  │
│  └─────────────────────┘  └──────────────────────────────┘  │
└─────────────────────────────────────────────────────────────┘
```

## How the Scheduler Works

The scheduler uses an **event-driven min-heap architecture** instead of polling.

Jobs are stored in a binary min-heap, ordered by their `next_run` time. The scheduler sleeps until the next job is due, executes it, calculates the next run time, and re-inserts it into the heap.

| Operation | Complexity |
|-----------|------------|
| Get next job | O(1) |
| Execute job | O(log n) |
| Add/reschedule job | O(log n) |

## Project Structure

```
src/
├── main.rs              # Entry point
├── config.rs            # Environment configuration
├── db/
│   ├── mod.rs           # Database pool + migrations
│   └── repository.rs    # CRUD operations
├── models/
│   ├── job.rs           # Job, Schedule types
│   └── run.rs           # Run, RunStatus types
├── api/
│   ├── mod.rs           # Router setup
│   ├── error.rs         # Error handling
│   ├── jobs.rs          # Job endpoints
│   └── runs.rs          # Run endpoints
├── scheduler/
│   ├── queue.rs         # Min-heap implementation
│   └── tick.rs          # Scheduler loop
└── executor/
    ├── mod.rs           # Execution orchestration
    └── docker.rs        # Docker build/run
```

## Configuration

Environment variables:

| Variable | Default | Description |
|----------|---------|-------------|
| `DATABASE_URL` | `sqlite:./data/scheduler.db` | SQLite database path |
| `SCHEDULER_LOGS_DIR` | `./logs` | Directory for run logs |
| `SCHEDULER_WORK_DIR` | `./work` | Directory for build contexts |
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

## Run Statuses

- `pending` - Waiting to start
- `running` - Currently executing
- `succeeded` - Completed with exit code 0
- `failed` - Completed with non-zero exit code
- `retrying` - Failed but will retry
- `cancelled` - Manually cancelled

## Testing

```bash
cargo test
```

## License

MIT
