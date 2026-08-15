use std::{cmp::Reverse, collections::HashSet, fmt::Write};

use crate::procfs::Snapshot;

pub fn render_prometheus(snapshot: &Snapshot, process_limit: usize) -> String {
    let mut output = String::with_capacity(16_384);
    metric(
        &mut output,
        "bbtop_cpu_usage_percent",
        "gauge",
        snapshot.cpu_percent,
    );
    metric(
        &mut output,
        "bbtop_cpu_logical_count",
        "gauge",
        snapshot.cpu_count,
    );
    metric(
        &mut output,
        "bbtop_memory_total_bytes",
        "gauge",
        snapshot.memory_total,
    );
    metric(
        &mut output,
        "bbtop_memory_available_bytes",
        "gauge",
        snapshot.memory_available,
    );
    metric(
        &mut output,
        "bbtop_swap_total_bytes",
        "gauge",
        snapshot.swap_total,
    );
    metric(
        &mut output,
        "bbtop_swap_free_bytes",
        "gauge",
        snapshot.swap_free,
    );
    metric(&mut output, "bbtop_load1", "gauge", snapshot.load[0]);
    metric(&mut output, "bbtop_load5", "gauge", snapshot.load[1]);
    metric(&mut output, "bbtop_load15", "gauge", snapshot.load[2]);
    metric(
        &mut output,
        "bbtop_uptime_seconds",
        "gauge",
        snapshot.uptime,
    );
    metric(
        &mut output,
        "bbtop_network_receive_bytes_total",
        "counter",
        snapshot.network_receive_bytes,
    );
    metric(
        &mut output,
        "bbtop_network_transmit_bytes_total",
        "counter",
        snapshot.network_transmit_bytes,
    );
    metric(
        &mut output,
        "bbtop_disk_read_bytes_total",
        "counter",
        snapshot.disk_read_bytes,
    );
    metric(
        &mut output,
        "bbtop_disk_write_bytes_total",
        "counter",
        snapshot.disk_write_bytes,
    );
    metric(
        &mut output,
        "bbtop_processes",
        "gauge",
        snapshot.processes_total,
    );
    metric(
        &mut output,
        "bbtop_processes_running",
        "gauge",
        snapshot.processes_running,
    );
    metric(
        &mut output,
        "bbtop_last_collection_timestamp_seconds",
        "gauge",
        snapshot.timestamp,
    );
    metric(
        &mut output,
        "bbtop_fans_detected",
        "gauge",
        snapshot.fans.len(),
    );
    let _ = writeln!(
        output,
        "# HELP bbtop_fan_speed_rpm Fan speed reported by Linux hwmon"
    );
    let _ = writeln!(output, "# TYPE bbtop_fan_speed_rpm gauge");
    for fan in &snapshot.fans {
        let _ = writeln!(
            output,
            "bbtop_fan_speed_rpm{{chip=\"{}\",sensor=\"{}\"}} {}",
            escape_label(&fan.chip),
            escape_label(&fan.sensor),
            fan.rpm
        );
    }
    metric(
        &mut output,
        "bbtop_temperatures_detected",
        "gauge",
        snapshot.temperatures.len(),
    );
    let _ = writeln!(
        output,
        "# HELP bbtop_temperature_celsius Temperature reported by Linux hwmon"
    );
    let _ = writeln!(output, "# TYPE bbtop_temperature_celsius gauge");
    for temperature in &snapshot.temperatures {
        let _ = writeln!(
            output,
            "bbtop_temperature_celsius{{chip=\"{}\",sensor=\"{}\"}} {:.3}",
            escape_label(&temperature.chip),
            escape_label(&temperature.sensor),
            temperature.celsius
        );
    }
    for name in [
        "bbtop_filesystem_size_bytes",
        "bbtop_filesystem_available_bytes",
        "bbtop_filesystem_used_bytes",
    ] {
        let _ = writeln!(output, "# TYPE {name} gauge");
    }
    for filesystem in &snapshot.filesystems {
        let labels = format!(
            "device=\"{}\",mountpoint=\"{}\",fstype=\"{}\"",
            escape_label(&filesystem.device),
            escape_label(&filesystem.mountpoint),
            escape_label(&filesystem.filesystem_type)
        );
        let used = filesystem
            .size_bytes
            .saturating_sub(filesystem.available_bytes);
        let _ = writeln!(
            output,
            "bbtop_filesystem_size_bytes{{{labels}}} {}",
            filesystem.size_bytes
        );
        let _ = writeln!(
            output,
            "bbtop_filesystem_available_bytes{{{labels}}} {}",
            filesystem.available_bytes
        );
        let _ = writeln!(output, "bbtop_filesystem_used_bytes{{{labels}}} {used}");
    }
    let _ = writeln!(output, "# HELP bbtop_info Host identity");
    let _ = writeln!(output, "# TYPE bbtop_info gauge");
    let _ = writeln!(
        output,
        "bbtop_info{{hostname=\"{}\"}} 1",
        escape_label(&snapshot.hostname)
    );

    for name in [
        "bbtop_process_cpu_usage_percent",
        "bbtop_process_resident_memory_bytes",
        "bbtop_process_virtual_memory_bytes",
        "bbtop_process_threads",
        "bbtop_process_state",
        "bbtop_process_read_bytes_total",
        "bbtop_process_write_bytes_total",
    ] {
        let kind = if name.ends_with("_total") {
            "counter"
        } else {
            "gauge"
        };
        let _ = writeln!(output, "# TYPE {name} {kind}");
    }
    let mut selected = Vec::new();
    let mut pids = HashSet::new();
    for process in snapshot.processes.iter().take(process_limit) {
        pids.insert(process.pid);
        selected.push(process);
    }
    let mut by_memory: Vec<_> = snapshot.processes.iter().collect();
    by_memory.sort_unstable_by_key(|process| Reverse(process.rss_bytes));
    for process in by_memory.into_iter().take(process_limit) {
        if pids.insert(process.pid) {
            selected.push(process);
        }
    }
    for process in selected {
        let labels = format!(
            "pid=\"{}\",name=\"{}\"",
            process.pid,
            escape_label(&process.name)
        );
        let _ = writeln!(
            output,
            "bbtop_process_cpu_usage_percent{{{labels}}} {:.3}",
            process.cpu_percent
        );
        let _ = writeln!(
            output,
            "bbtop_process_resident_memory_bytes{{{labels}}} {}",
            process.rss_bytes
        );
        let _ = writeln!(
            output,
            "bbtop_process_virtual_memory_bytes{{{labels}}} {}",
            process.virtual_bytes
        );
        let _ = writeln!(
            output,
            "bbtop_process_threads{{{labels}}} {}",
            process.threads
        );
        let _ = writeln!(
            output,
            "bbtop_process_state{{{labels},state=\"{}\"}} 1",
            process.state
        );
        let _ = writeln!(
            output,
            "bbtop_process_read_bytes_total{{{labels}}} {}",
            process.read_bytes
        );
        let _ = writeln!(
            output,
            "bbtop_process_write_bytes_total{{{labels}}} {}",
            process.write_bytes
        );
    }
    output
}

fn metric(output: &mut String, name: &str, kind: &str, value: impl std::fmt::Display) {
    let _ = writeln!(output, "# TYPE {name} {kind}");
    let _ = writeln!(output, "{name} {value}");
}

fn escape_label(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::procfs::{Filesystem, Process, Temperature};

    #[test]
    fn escapes_prometheus_labels() {
        assert_eq!(escape_label("a\\\"b\nc"), "a\\\\\\\"b\\nc");
    }

    #[test]
    fn renders_core_metrics() {
        let snapshot = Snapshot {
            cpu_percent: 12.5,
            hostname: "box".into(),
            ..Snapshot::default()
        };
        let rendered = render_prometheus(&snapshot, 10);
        assert!(rendered.contains("bbtop_cpu_usage_percent 12.5"));
        assert!(rendered.contains("bbtop_info{hostname=\"box\"} 1"));
    }

    #[test]
    fn renders_filesystem_capacity() {
        let snapshot = Snapshot {
            filesystems: vec![Filesystem {
                device: "/dev/test".into(),
                mountpoint: "/data".into(),
                filesystem_type: "ext4".into(),
                size_bytes: 1_000,
                available_bytes: 250,
            }],
            ..Snapshot::default()
        };
        let rendered = render_prometheus(&snapshot, 10);
        assert!(rendered.contains(
            "bbtop_filesystem_used_bytes{device=\"/dev/test\",mountpoint=\"/data\",fstype=\"ext4\"} 750"
        ));
    }

    #[test]
    fn renders_hwmon_temperatures() {
        let snapshot = Snapshot {
            temperatures: vec![Temperature {
                chip: "amdgpu".into(),
                sensor: "edge".into(),
                celsius: 64.125,
            }],
            ..Snapshot::default()
        };
        let rendered = render_prometheus(&snapshot, 10);
        assert!(
            rendered.contains("bbtop_temperature_celsius{chip=\"amdgpu\",sensor=\"edge\"} 64.125")
        );
    }

    #[test]
    fn exports_top_cpu_and_top_memory_processes() {
        let snapshot = Snapshot {
            processes: vec![
                Process {
                    pid: 1,
                    name: "cpu".into(),
                    cpu_percent: 99.0,
                    rss_bytes: 1,
                    ..Process::default()
                },
                Process {
                    pid: 2,
                    name: "memory".into(),
                    rss_bytes: 1_000_000,
                    ..Process::default()
                },
            ],
            ..Snapshot::default()
        };
        let rendered = render_prometheus(&snapshot, 1);
        assert!(rendered.contains("pid=\"1\",name=\"cpu\""));
        assert!(rendered.contains("pid=\"2\",name=\"memory\""));
    }
}
