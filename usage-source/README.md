# Agent usage source

`usage-source` tails local Claude Code and Codex JSONL sessions and sends normalized
`agent_usage` events to the gateway. It extracts token counters and session metadata
only. Prompt text, model responses, tool arguments, tool output, images, and complete
file paths are never included in emitted events.

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

## Configuration

| Variable | Default in Compose | Meaning |
| --- | --- | --- |
| `GATEWAY_URL` | `http://gateway:1234/events` | Gateway event endpoint |
| `SCAN_INTERVAL` | `2s` | Delay between completed filesystem scans |
| `WORKERS` | `8` | Number of session files processed concurrently |
| `BACKFILL` | `true` | Send historical records on a fresh state volume |
| `STATE_DIR` | `/var/lib/usage-source` | Checkpoints and sent event IDs |

Set `LOCAL_UID` and `LOCAL_GID` before building if the session files belong to a
host user other than `1000:1000`.

Start the source with:

```sh
docker compose up -d --build usage-source
docker compose logs -f usage-source
```
