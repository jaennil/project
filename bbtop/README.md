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
`http://127.0.0.1:9099/metrics`. Keys `c`, `m`, `r`, `w`, `n`, `g`, and `p` change
process sorting; `q` exits. For a headless host:

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
- network byte counters per interface, labeled `physical` or `virtual`, and
  block-device byte counters;
- filesystem size, used space and available space per mount point;
- fan speed in RPM when exposed through Linux `hwmon`;
- temperatures from every Linux `hwmon` sensor, labeled by chip and sensor;
- power in watts where a Linux `hwmon` driver exposes it, including instantaneous
  and averaged readings as reported by the driver;
- estimated whole-laptop draw from battery voltage and current while discharging;
  charging systems expose battery charging power but not reliable wall power;
- battery charge percentage and charging state.
- hwmon voltage/current readings, valid thermal safety limits and alarms, CPU
  frequency per logical core, and external-power connection state.
- task counts and state;
- GPU utilisation and VRAM per card, where the DRM driver reports them;
- bounded top-CPU, top-memory, top-network and top-GPU process series with CPU,
  RSS, virtual memory, threads, disk and network byte counters, GPU busy time
  and VRAM;
- collection timestamp and host identity.

Prometheus owns historical retention and rate calculation. In particular,
network and disk values are counters, so dashboards use `rate()` instead of
storing a second copy of history in the agent.

Network counters are exported one series per interface rather than as a single
host total. A total would drop whenever an interface goes away - a container
teardown is enough - and a falling counter reads as a restart, which Prometheus
turns into a burst of several gigabytes that never happened. Rate each
interface, then sum. Veth pairs are skipped because each container lifetime
would leave another dead series behind, and dashboards should sum only
`kind="physical"`: bridges and tunnels carry copies of traffic that their
uplink counts again.

## More than one host

The exporter publishes its own hostname on `bbtop_info`, and the provisioned
dashboard turns that into a Host dropdown: a visible variable over
`label_values(bbtop_info, hostname)` picks the machine, a hidden one resolves it
to the Prometheus `instance`, and every panel filters on that. One dashboard
serves any number of hosts, so a panel is only ever edited once.

Hosts differ in what they can report, and the dashboard does not hide it. A
desktop has no battery, so the battery and mains panels stay empty for it; a
machine whose GPU driver omits `gpu_busy_percent` shows no GPU load. Empty means
the hardware or driver does not expose the reading.

Two units cover the two ways a host is reached.
[`bbtop.service`](deploy/systemd/bbtop.service) binds localhost and pairs with
an outbound reverse tunnel, which suits a laptop that is not always on the
monitored network. [`bbtop-node.service`](deploy/systemd/bbtop-node.service) is
for a host Prometheus can reach directly: it listens on all interfaces but
restricts who may connect with systemd's `IPAddressAllow`, because process names
and PIDs are in these metrics.

## GPU load and per-process GPU usage

Card utilisation and VRAM come from the DRM driver through sysfs:
`gpu_busy_percent`, `mem_info_vram_used` and `mem_info_vram_total` under
`/sys/class/drm/cardN/device`. Drivers that leave `gpu_busy_percent` out, Intel
integrated graphics among them, report no card rather than a card at zero.

Per-process figures come from the kernel's DRM fdinfo interface. For every
descriptor a process holds on `/dev/dri/*`, `/proc/PID/fdinfo/N` lists
`drm-engine-*` busy nanoseconds and `drm-memory-vram`. Engines run in parallel,
so their busy times are summed and the result can pass 100% exactly the way
process CPU does across cores. Descriptors duplicated within a process repeat a
`drm-client-id`, so each client is counted once. Reading a descriptor's link is
much cheaper than opening fdinfo and hardly any process holds a DRM handle, so
the link is checked first; the whole sweep costs under 10 ms per collection on a
machine with 500 processes.

No root privileges, driver tools or vendor libraries are involved. This works
for amdgpu and for any driver implementing the fdinfo interface.

## Per-process network throughput

Linux keeps no per-process network counters. `/proc/PID/io` covers disk only,
and `/proc/PID/net/dev` describes the whole network namespace rather than the
process that reads it, so every process in a namespace reports identical
numbers. Attributing bytes to a process requires tracing the socket layer.

`bbtop-net.service` is an optional collector that does exactly that. It runs
`bpftrace` against [`bbtop-net.bt`](deploy/systemd/bbtop-net.bt), which attaches
to `tcp_sendmsg`, `tcp_cleanup_rbuf`, `udp_sendmsg` and `udp_recvmsg`,
accumulates bytes per PID in kernel maps, and publishes a snapshot to
`/run/bbtop/process-net.txt`. The exporter reads that file and joins it with the
process table it already builds from procfs, so it still needs no capabilities
of its own. The collector runs with `CAP_BPF` and `CAP_PERFMON` only.

```bash
sudo pacman -S bpftrace
sudo mkdir -p /usr/local/libexec
sudo install -m 755 deploy/systemd/bbtop-net-collect /usr/local/libexec/
sudo install -m 755 deploy/systemd/bbtop-net.bt /usr/local/libexec/
sudo install -m 644 deploy/systemd/bbtop-net.service /etc/systemd/system/
sudo systemctl enable --now bbtop-net.service
```

The exporter looks for helper snapshots in `/run/bbtop`; `BBTOP_RUNTIME_ROOT`
points it elsewhere, which is how the container reads the host directory.

Without the collector the network counters stay at zero and no process series
is added for them. What is measured is payload bytes charged to the process
issuing the syscall: protocol headers, retransmits, and traffic generated by
the kernel itself are excluded, so the sum over processes is lower than the
interface counters in `/proc/net/dev`. Bytes are attributed to the process that
touches the socket, which for proxied traffic is the proxy rather than its
client.

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

`bbtop-smart.timer` is an optional, separate read-only NVMe SMART collector.
It runs `smartctl` once per minute as a tightly scoped systemd service and
writes a world-readable snapshot to `/run/bbtop`; the main exporter receives no
additional capabilities.
