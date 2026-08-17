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
    let _ = writeln!(output, "# TYPE bbtop_network_receive_bytes_total counter");
    let _ = writeln!(output, "# TYPE bbtop_network_transmit_bytes_total counter");
    for interface in &snapshot.networks {
        let labels = format!(
            "interface=\"{}\",kind=\"{}\"",
            escape_label(&interface.name),
            interface.kind
        );
        let _ = writeln!(
            output,
            "bbtop_network_receive_bytes_total{{{labels}}} {}",
            interface.receive_bytes
        );
        let _ = writeln!(
            output,
            "bbtop_network_transmit_bytes_total{{{labels}}} {}",
            interface.transmit_bytes
        );
    }
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
    metric(
        &mut output,
        "bbtop_power_sensors_detected",
        "gauge",
        snapshot.power.len(),
    );
    let _ = writeln!(
        output,
        "# HELP bbtop_power_watts Power reported by Linux hwmon in watts"
    );
    let _ = writeln!(output, "# TYPE bbtop_power_watts gauge");
    for power in &snapshot.power {
        let _ = writeln!(
            output,
            "bbtop_power_watts{{chip=\"{}\",sensor=\"{}\",reading=\"{}\"}} {:.6}",
            escape_label(&power.chip),
            escape_label(&power.sensor),
            escape_label(&power.reading),
            power.watts
        );
    }
    let _ = writeln!(
        output,
        "# HELP bbtop_battery_power_watts Electrical power flowing through the battery"
    );
    let _ = writeln!(output, "# TYPE bbtop_battery_power_watts gauge");
    let _ = writeln!(
        output,
        "# HELP bbtop_laptop_power_estimate_watts Estimated whole-laptop draw while discharging"
    );
    let _ = writeln!(output, "# TYPE bbtop_laptop_power_estimate_watts gauge");
    let _ = writeln!(
        output,
        "# HELP bbtop_battery_charge_percent Battery state of charge in percent"
    );
    let _ = writeln!(output, "# TYPE bbtop_battery_charge_percent gauge");
    let mut discharge_watts = 0.0;
    for battery in &snapshot.battery_power {
        let _ = writeln!(
            output,
            "bbtop_battery_power_watts{{battery=\"{}\",status=\"{}\"}} {:.6}",
            escape_label(&battery.battery),
            escape_label(&battery.status),
            battery.watts
        );
        if let Some(charge_percent) = battery.charge_percent {
            let _ = writeln!(
                output,
                "bbtop_battery_charge_percent{{battery=\"{}\",status=\"{}\"}} {:.3}",
                escape_label(&battery.battery),
                escape_label(&battery.status),
                charge_percent
            );
        }
        if battery.status == "Discharging" {
            discharge_watts += battery.watts;
        }
    }
    if discharge_watts > 0.0 {
        let _ = writeln!(
            output,
            "bbtop_laptop_power_estimate_watts{{source=\"battery\"}} {:.6}",
            discharge_watts
        );
    }
    render_electrical_readings(
        &mut output,
        "bbtop_voltage_volts",
        "Voltage reported by Linux hwmon",
        &snapshot.voltages,
    );
    render_electrical_readings(
        &mut output,
        "bbtop_current_amperes",
        "Current reported by Linux hwmon",
        &snapshot.currents,
    );
    let _ = writeln!(
        output,
        "# HELP bbtop_temperature_limit_celsius Temperature limits reported by Linux hwmon"
    );
    let _ = writeln!(output, "# TYPE bbtop_temperature_limit_celsius gauge");
    for limit in &snapshot.temperature_limits {
        let _ = writeln!(
            output,
            "bbtop_temperature_limit_celsius{{chip=\"{}\",sensor=\"{}\",limit=\"{}\"}} {:.3}",
            escape_label(&limit.chip),
            escape_label(&limit.sensor),
            escape_label(&limit.limit),
            limit.celsius
        );
    }
    let _ = writeln!(
        output,
        "# HELP bbtop_temperature_alarm Temperature alarm state reported by Linux hwmon"
    );
    let _ = writeln!(output, "# TYPE bbtop_temperature_alarm gauge");
    for alarm in &snapshot.temperature_alarms {
        let _ = writeln!(
            output,
            "bbtop_temperature_alarm{{chip=\"{}\",sensor=\"{}\"}} {}",
            escape_label(&alarm.chip),
            escape_label(&alarm.sensor),
            alarm.value
        );
    }
    let _ = writeln!(
        output,
        "# HELP bbtop_cpu_frequency_hertz Current CPU frequency by logical CPU"
    );
    let _ = writeln!(output, "# TYPE bbtop_cpu_frequency_hertz gauge");
    for frequency in &snapshot.cpu_frequencies {
        let _ = writeln!(
            output,
            "bbtop_cpu_frequency_hertz{{cpu=\"{}\"}} {}",
            escape_label(&frequency.cpu),
            frequency.hertz
        );
    }
    let _ = writeln!(
        output,
        "# HELP bbtop_mains_online Whether an external power supply is online"
    );
    let _ = writeln!(output, "# TYPE bbtop_mains_online gauge");
    for supply in &snapshot.mains_supplies {
        let _ = writeln!(
            output,
            "bbtop_mains_online{{supply=\"{}\"}} {}",
            escape_label(&supply.supply),
            u8::from(supply.online)
        );
    }
    render_nvme_smart(&mut output, &snapshot.nvme_smart);
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
        "bbtop_process_network_receive_bytes_total",
        "bbtop_process_network_transmit_bytes_total",
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
    // A process can saturate a link while barely registering on CPU or memory,
    // so network talkers get their own ranking. Processes without traced bytes
    // add nothing but cardinality, and the ranking is descending, so stop early.
    let mut by_network: Vec<_> = snapshot.processes.iter().collect();
    by_network.sort_unstable_by_key(|process| Reverse(process_network_bytes(process)));
    for process in by_network.into_iter().take(process_limit) {
        if process_network_bytes(process) == 0 {
            break;
        }
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
        let _ = writeln!(
            output,
            "bbtop_process_network_receive_bytes_total{{{labels}}} {}",
            process.network_receive_bytes
        );
        let _ = writeln!(
            output,
            "bbtop_process_network_transmit_bytes_total{{{labels}}} {}",
            process.network_transmit_bytes
        );
    }
    output
}

fn process_network_bytes(process: &crate::procfs::Process) -> u64 {
    process
        .network_receive_bytes
        .saturating_add(process.network_transmit_bytes)
}

fn metric(output: &mut String, name: &str, kind: &str, value: impl std::fmt::Display) {
    let _ = writeln!(output, "# TYPE {name} {kind}");
    let _ = writeln!(output, "{name} {value}");
}

fn render_electrical_readings(
    output: &mut String,
    name: &str,
    help: &str,
    readings: &[crate::procfs::ElectricalReading],
) {
    let _ = writeln!(output, "# HELP {name} {help}");
    let _ = writeln!(output, "# TYPE {name} gauge");
    for reading in readings {
        let _ = writeln!(
            output,
            "{name}{{chip=\"{}\",sensor=\"{}\"}} {:.6}",
            escape_label(&reading.chip),
            escape_label(&reading.sensor),
            reading.value
        );
    }
}

fn render_nvme_smart(output: &mut String, devices: &[crate::procfs::NvmeSmart]) {
    for name in [
        "bbtop_nvme_percentage_used",
        "bbtop_nvme_available_spare_percent",
        "bbtop_nvme_available_spare_threshold_percent",
        "bbtop_nvme_critical_warning",
        "bbtop_nvme_data_read_bytes_total",
        "bbtop_nvme_data_written_bytes_total",
        "bbtop_nvme_power_cycles_total",
        "bbtop_nvme_power_on_hours_total",
        "bbtop_nvme_unsafe_shutdowns_total",
        "bbtop_nvme_media_errors_total",
        "bbtop_nvme_error_log_entries_total",
    ] {
        let metric_type = if name.ends_with("_total") {
            "counter"
        } else {
            "gauge"
        };
        let _ = writeln!(output, "# TYPE {name} {metric_type}");
    }
    for smart in devices {
        let labels = format!("device=\"{}\"", escape_label(&smart.device));
        let _ = writeln!(
            output,
            "bbtop_nvme_percentage_used{{{labels}}} {}",
            smart.percentage_used
        );
        let _ = writeln!(
            output,
            "bbtop_nvme_available_spare_percent{{{labels}}} {}",
            smart.available_spare
        );
        let _ = writeln!(
            output,
            "bbtop_nvme_available_spare_threshold_percent{{{labels}}} {}",
            smart.available_spare_threshold
        );
        let _ = writeln!(
            output,
            "bbtop_nvme_critical_warning{{{labels}}} {}",
            smart.critical_warning
        );
        let _ = writeln!(
            output,
            "bbtop_nvme_data_read_bytes_total{{{labels}}} {}",
            smart.data_units_read.saturating_mul(512_000)
        );
        let _ = writeln!(
            output,
            "bbtop_nvme_data_written_bytes_total{{{labels}}} {}",
            smart.data_units_written.saturating_mul(512_000)
        );
        let _ = writeln!(
            output,
            "bbtop_nvme_power_cycles_total{{{labels}}} {}",
            smart.power_cycles
        );
        let _ = writeln!(
            output,
            "bbtop_nvme_power_on_hours_total{{{labels}}} {}",
            smart.power_on_hours
        );
        let _ = writeln!(
            output,
            "bbtop_nvme_unsafe_shutdowns_total{{{labels}}} {}",
            smart.unsafe_shutdowns
        );
        let _ = writeln!(
            output,
            "bbtop_nvme_media_errors_total{{{labels}}} {}",
            smart.media_errors
        );
        let _ = writeln!(
            output,
            "bbtop_nvme_error_log_entries_total{{{labels}}} {}",
            smart.error_log_entries
        );
    }
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
    use crate::procfs::{BatteryPower, Filesystem, Power, Process, Temperature};

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
    fn renders_hwmon_power() {
        let snapshot = Snapshot {
            power: vec![Power {
                chip: "amdgpu".into(),
                sensor: "PPT".into(),
                reading: "average".into(),
                watts: 16.053,
            }],
            ..Snapshot::default()
        };
        let rendered = render_prometheus(&snapshot, 10);
        assert!(rendered.contains(
            "bbtop_power_watts{chip=\"amdgpu\",sensor=\"PPT\",reading=\"average\"} 16.053000"
        ));
    }

    #[test]
    fn renders_battery_power_and_discharge_estimate() {
        let snapshot = Snapshot {
            battery_power: vec![BatteryPower {
                battery: "BATT".into(),
                status: "Discharging".into(),
                watts: 22.5,
                charge_percent: Some(75.0),
            }],
            ..Snapshot::default()
        };
        let rendered = render_prometheus(&snapshot, 10);
        assert!(rendered.contains(
            "bbtop_battery_power_watts{battery=\"BATT\",status=\"Discharging\"} 22.500000"
        ));
        assert!(
            rendered.contains("bbtop_laptop_power_estimate_watts{source=\"battery\"} 22.500000")
        );
        assert!(rendered.contains(
            "bbtop_battery_charge_percent{battery=\"BATT\",status=\"Discharging\"} 75.000"
        ));
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

    #[test]
    fn renders_one_series_per_network_interface() {
        let snapshot = Snapshot {
            networks: vec![
                crate::procfs::NetworkInterface {
                    name: "wlp2s0".into(),
                    kind: "physical".into(),
                    receive_bytes: 10,
                    transmit_bytes: 20,
                },
                crate::procfs::NetworkInterface {
                    name: "docker0".into(),
                    kind: "virtual".into(),
                    receive_bytes: 30,
                    transmit_bytes: 40,
                },
            ],
            ..Snapshot::default()
        };
        let rendered = render_prometheus(&snapshot, 1);
        assert!(rendered.contains(
            "bbtop_network_receive_bytes_total{interface=\"wlp2s0\",kind=\"physical\"} 10"
        ));
        assert!(rendered.contains(
            "bbtop_network_transmit_bytes_total{interface=\"docker0\",kind=\"virtual\"} 40"
        ));
    }

    #[test]
    fn exports_network_talkers_that_are_idle_otherwise() {
        let snapshot = Snapshot {
            processes: vec![
                Process {
                    pid: 1,
                    name: "cpu".into(),
                    cpu_percent: 99.0,
                    rss_bytes: 1_000_000,
                    ..Process::default()
                },
                Process {
                    pid: 2,
                    name: "downloader".into(),
                    network_receive_bytes: 4_000,
                    network_transmit_bytes: 100,
                    ..Process::default()
                },
                Process {
                    pid: 3,
                    name: "silent".into(),
                    ..Process::default()
                },
            ],
            ..Snapshot::default()
        };
        let rendered = render_prometheus(&snapshot, 1);
        assert!(rendered.contains(
            "bbtop_process_network_receive_bytes_total{pid=\"2\",name=\"downloader\"} 4000"
        ));
        assert!(!rendered.contains("name=\"silent\""));
    }
}
