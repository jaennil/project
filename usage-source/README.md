# Agent usage source

`usage-source` tails local Claude Code and Codex JSONL sessions and sends normalized
`agent_usage` and `agent_session_metrics` events to the gateway. It extracts token
counters, session metadata, and privacy-safe engineering activity counters. Prompt
text, model responses, tool arguments, tool output, images, commands, diffs, and
complete file paths are never included in emitted events.

It also exports current Codex and Claude Code account limits for Prometheus.
Rate-limit snapshots are operational state and are not sent through Kafka. Only the
provider, limit identifier, window, percentage, and timestamps are persisted.

The Compose service mounts only these source directories as read-only:

- `${HOME}/.codex/sessions`
- `${HOME}/.claude/projects`

On its first run, the source backfills existing usage records. It persists file
checkpoints and deterministic sent event IDs in the `usage-source-data` volume, so
subsequent scans send only new records and copied Claude history is deduplicated.

## Event shape

```json
{
  "schema_version": 1,
  "event_id": "f39c...",
  "event_type": "agent_usage",
  "occurred_at": "2026-07-17T10:00:02Z",
  "source": "agent-usage-source",
  "session_id": "019...",
  "properties": {
    "provider": "codex",
    "project": "project",
    "model": "gpt-5",
    "input_tokens": 120,
    "cached_input_tokens": 80,
    "output_tokens": 30,
    "reasoning_output_tokens": 10,
    "total_tokens": 150
  }
}
```

Session metrics are cumulative snapshots for a transcript:

```json
{
  "schema_version": 1,
  "event_type": "agent_session_metrics",
  "occurred_at": "2026-07-17T10:08:02Z",
  "source": "agent-usage-source",
  "session_id": "019...",
  "properties": {
    "metrics_version": 1,
    "provider": "codex",
    "project": "project",
    "model": "gpt-5",
    "tool_calls": 12,
    "tool_errors": 2,
    "tests_run": 3,
    "tests_failed": 1,
    "user_interruptions": 1,
    "files_changed": 5,
    "lines_added": 140,
    "lines_deleted": 30,
    "committed": true,
    "reverted": false,
    "session_duration_ms": 480000
  }
}
```

The counters are derived as follows:

- tool calls and explicit tool failures come from structured tool records;
- test runs are successful or failed shell invocations of common test runners;
- changed files and approximate line counts come from successful structured patch,
  Edit, and Write tools;
- commits and reverts come from successful `git commit`, `git revert`, `git reset`,
  and `git restore` commands;
- duration is wall-clock time between the first and last relevant transcript records,
  so it can include idle time;
- inherited history in a forked Claude transcript is part of that transcript's
  cumulative metrics.

## Configuration

| Variable | Default in Compose | Meaning |
| --- | --- | --- |
| `GATEWAY_URL` | `http://gateway:1234/events` | Gateway event endpoint |
| `SCAN_INTERVAL` | `2s` | Delay between completed filesystem scans |
| `WORKERS` | `8` | Number of session files processed concurrently |
| `BACKFILL` | `true` | Send historical records on a fresh state volume |
| `STATE_DIR` | `/var/lib/usage-source` | Checkpoints and sent event IDs |
| `HTTP_ADDR` | `:9469` | Health, Claude ingest, and Prometheus listen address |

## Account rate limits

Codex rate limits are read from structured `token_count` transcript records. The
available windows come from the service and must not be assumed to always be five
hours. Claude Code does not write its account limits to transcripts, so the status-line
helper reads the current account usage from Claude's OAuth usage endpoint. It caches a
sanitized response for 30 seconds across all running Claude processes and uses fresh
status-line data only as a fallback. Expired windows are ignored.

Configure this command in the global `~/.claude/settings.json`:

```json
{
  "statusLine": {
    "type": "command",
    "command": "/home/jaennil/dev/pet/project/usage-source/claude-rate-limits.sh",
    "refreshInterval": 30
  }
}
```

The script reads the access token from `~/.claude/.credentials.json`, but never writes
the token to its cache, command arguments, `usage-source`, or Prometheus. It sends only
the five-hour and seven-day percentages and reset timestamps to
`http://127.0.0.1:9469/v1/rate-limits/claude`, and displays the percentages in Claude
Code. It requires `curl`, `flock`, `jq`, and GNU `stat` on the host.

Set `AGENT_USAGE_SOURCE_URL` to override the local ingest endpoint,
`CLAUDE_CREDENTIALS_FILE` to override the credentials path,
`CLAUDE_RATE_LIMIT_CACHE_SECONDS` to change the 30-second refresh interval,
`CLAUDE_RATE_LIMIT_MAX_CACHE_AGE` to change the 120-second stale cutoff, or
`CLAUDE_RATE_LIMIT_CACHE_DIR` to override the runtime cache directory.

Prometheus scrapes these gauges from `/metrics`:

- `agent_rate_limit_used_ratio`
- `agent_rate_limit_reset_timestamp_seconds`
- `agent_rate_limit_last_update_timestamp_seconds`
- `agent_rate_limit_window_seconds`

The Compose stack provisions the **Agent rate limits** Grafana dashboard. Its dashed
budget line starts at zero at `reset_timestamp - window_seconds` and reaches 100% at
the provider-reported reset timestamp. For a five-hour limit this is 20 percentage
points per hour. Prometheus sends a warning when five-hour usage stays above this
line for five minutes. The same proportional pace check applies to every other
provider-reported window, and a critical alert fires at 95%. Alerts automatically
stop after the reported reset timestamp; connect Prometheus to an Alertmanager or
configure a Grafana contact point to deliver notifications.

Set `LOCAL_UID` and `LOCAL_GID` before building if the session files belong to a
host user other than `1000:1000`.

Start the source with:

```sh
docker compose up -d --build usage-source
docker compose logs -f usage-source
```
