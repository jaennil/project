# bbtop

`bbtop` is a small Linux task monitor written in Rust. It combines a live
terminal process view with a Prometheus exporter, so the same data is useful
both while debugging now and when investigating an incident later in Grafana.

The binary has no runtime dependencies and reads Linux procfs directly.

## Run locally

```bash
cargo run --release
```

The TUI starts together with an exporter at
`http://127.0.0.1:9099/metrics`. Keys `c`, `m`, `r`, `w`, and `p` change process
sorting; `q` exits. For a headless host:

```bash
cargo run --release -- --no-tui --listen 0.0.0.0:9099
```

Options are documented by `bbtop --help`. The exporter intentionally publishes
the union of the top 50 CPU-consuming and top 50 memory-consuming processes by
default to bound Prometheus label cardinality. Change each ranking limit with
`--top`.

## Grafana with history

Start the complete local stack:

```bash
docker compose up --build -d
```

Open <http://localhost:3000> and sign in with `admin` / `bbtop`. The provisioned
dashboard is in the **bbtop** folder. Prometheus is available at
<http://localhost:9091>; samples are retained for 30 days by default.

The container uses the host PID namespace and mounts host `/proc`, `/sys`, and
the root filesystem read-only so the dashboard describes the host rather than
the container. The root mount is used only for filesystem capacity statistics.
Review that access model before deploying on a shared machine. Ports bind to
localhost by default.

## Exported data

- aggregate CPU, logical CPUs, load averages and uptime;
- total, available and swap memory;
- network and block-device byte counters;
- filesystem size, used space and available space per mount point;
- fan speed in RPM when exposed through Linux `hwmon`;
- task counts and state;
- bounded top-CPU and top-memory process series with CPU, RSS, virtual memory,
  threads and I/O counters;
- collection timestamp and host identity.

Prometheus owns historical retention and rate calculation. In particular,
network and disk values are counters, so dashboards use `rate()` instead of
storing a second copy of history in the agent.

## Scope

This initial version is Linux-only. It observes tasks but does not send signals
or change process priority; those controls should be added behind explicit
confirmation and permission checks.

An experimental read-only hwmon driver for the HONOR FMI-XX is available under
[`contrib/honor-fmi-hwmon`](contrib/honor-fmi-hwmon/README.md).

For resilient remote collection, systemd units for a high-priority host agent
and an outbound reverse tunnel are available in [`deploy/systemd`](deploy/systemd).
The exporter remains bound to localhost; the tunnel exposes it only through an
internal ClusterIP service in the homelab cluster.
