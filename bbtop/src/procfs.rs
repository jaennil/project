use std::{
    collections::HashMap,
    ffi::CString,
    fs, io,
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Debug, Default)]
pub struct Process {
    pub pid: u32,
    pub name: String,
    pub state: char,
    pub cpu_percent: f64,
    pub rss_bytes: u64,
    pub virtual_bytes: u64,
    pub threads: u64,
    pub read_bytes: u64,
    pub write_bytes: u64,
    pub network_receive_bytes: u64,
    pub network_transmit_bytes: u64,
    pub gpu_percent: f64,
    pub gpu_vram_bytes: u64,
}

#[derive(Clone, Debug, Default)]
pub struct BrowserTab {
    pub pid: u32,
    pub cpu_percent: f64,
    pub memory_bytes: u64,
    /// How many documents share this process, and therefore this figure.
    pub windows: u32,
    /// Host the tab is on. Titles alone can be useless: several sites publish
    /// pages titled things like "User-ID".
    pub site: String,
    pub title: String,
}

#[derive(Clone, Debug, Default)]
pub struct Gpu {
    pub card: String,
    pub driver: String,
    pub busy_percent: f64,
    pub vram_used_bytes: u64,
    pub vram_total_bytes: u64,
}

#[derive(Clone, Debug, Default)]
pub struct NetworkInterface {
    pub name: String,
    /// `physical` when the kernel backs the interface with a real device.
    /// Bridges, tunnels and loopback are `virtual` and carry copies of traffic
    /// that its uplink counts again, so summing every interface triple counts.
    pub kind: String,
    pub receive_bytes: u64,
    pub transmit_bytes: u64,
}

#[derive(Clone, Debug, Default)]
pub struct Fan {
    pub chip: String,
    pub sensor: String,
    pub rpm: u64,
}

#[derive(Clone, Debug, Default)]
pub struct Temperature {
    pub chip: String,
    pub sensor: String,
    pub celsius: f64,
}

#[derive(Clone, Debug, Default)]
pub struct Power {
    pub chip: String,
    pub sensor: String,
    pub reading: String,
    pub watts: f64,
}

#[derive(Clone, Debug, Default)]
pub struct BatteryPower {
    pub battery: String,
    pub status: String,
    pub watts: f64,
    pub charge_percent: Option<f64>,
}

#[derive(Clone, Debug, Default)]
pub struct ElectricalReading {
    pub chip: String,
    pub sensor: String,
    pub value: f64,
}

#[derive(Clone, Debug, Default)]
pub struct TemperatureLimit {
    pub chip: String,
    pub sensor: String,
    pub limit: String,
    pub celsius: f64,
}

#[derive(Clone, Debug, Default)]
pub struct TemperatureAlarm {
    pub chip: String,
    pub sensor: String,
    pub value: u64,
}

#[derive(Clone, Debug, Default)]
pub struct CpuFrequency {
    pub cpu: String,
    pub hertz: u64,
}

#[derive(Clone, Debug, Default)]
pub struct MainsSupply {
    pub supply: String,
    pub online: bool,
}

#[derive(Clone, Debug, Default)]
pub struct NvmeSmart {
    pub device: String,
    pub percentage_used: u64,
    pub available_spare: u64,
    pub available_spare_threshold: u64,
    pub critical_warning: u64,
    pub data_units_read: u64,
    pub data_units_written: u64,
    pub power_cycles: u64,
    pub power_on_hours: u64,
    pub unsafe_shutdowns: u64,
    pub media_errors: u64,
    pub error_log_entries: u64,
}

#[derive(Clone, Debug, Default)]
pub struct Filesystem {
    pub device: String,
    pub mountpoint: String,
    pub filesystem_type: String,
    pub size_bytes: u64,
    pub available_bytes: u64,
}

#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub timestamp: u64,
    pub hostname: String,
    /// ACPI power profile the firmware is in: low-power, balanced, performance.
    pub platform_profile: String,
    pub cpu_percent: f64,
    pub cpu_count: usize,
    pub memory_total: u64,
    pub memory_available: u64,
    pub swap_total: u64,
    pub swap_free: u64,
    pub load: [f64; 3],
    pub uptime: f64,
    pub network_receive_bytes: u64,
    pub network_transmit_bytes: u64,
    pub networks: Vec<NetworkInterface>,
    pub disk_read_bytes: u64,
    pub disk_write_bytes: u64,
    pub processes_total: usize,
    pub processes_running: usize,
    pub processes: Vec<Process>,
    pub fans: Vec<Fan>,
    pub temperatures: Vec<Temperature>,
    pub power: Vec<Power>,
    pub battery_power: Vec<BatteryPower>,
    pub voltages: Vec<ElectricalReading>,
    pub currents: Vec<ElectricalReading>,
    pub temperature_limits: Vec<TemperatureLimit>,
    pub temperature_alarms: Vec<TemperatureAlarm>,
    pub cpu_frequencies: Vec<CpuFrequency>,
    pub gpus: Vec<Gpu>,
    pub browser_tabs: Vec<BrowserTab>,
    pub mains_supplies: Vec<MainsSupply>,
    pub nvme_smart: Vec<NvmeSmart>,
    pub filesystems: Vec<Filesystem>,
}

impl Snapshot {
    pub fn empty() -> Self {
        Self::default()
    }
}

#[derive(Clone, Copy, Default)]
struct CpuTimes {
    total: u64,
    idle: u64,
}

pub struct Collector {
    root: PathBuf,
    sys_root: PathBuf,
    filesystem_root: PathBuf,
    runtime_root: PathBuf,
    ticks_per_second: f64,
    page_size: u64,
    previous_cpu: CpuTimes,
    previous_process_cpu: HashMap<u32, u64>,
    previous_client_gpu: HashMap<(String, u64), u64>,
    previous_timestamp: Option<SystemTime>,
    previous_smart_collection: Option<SystemTime>,
    nvme_smart: Vec<NvmeSmart>,
}

impl Collector {
    pub fn new(root: PathBuf, filesystem_root: PathBuf, runtime_root: PathBuf) -> Self {
        let page_size = detect_page_size(&root);
        let sys_root = if root == Path::new("/proc") {
            PathBuf::from("/sys")
        } else {
            root.parent().unwrap_or(Path::new("/")).join("sys")
        };
        Self {
            root,
            sys_root,
            filesystem_root,
            runtime_root,
            ticks_per_second: 100.0,
            page_size,
            previous_cpu: CpuTimes::default(),
            previous_process_cpu: HashMap::new(),
            previous_client_gpu: HashMap::new(),
            previous_timestamp: None,
            previous_smart_collection: None,
            nvme_smart: Vec::new(),
        }
    }

    pub fn collect(&mut self) -> io::Result<Snapshot> {
        let now = SystemTime::now();
        let elapsed = self
            .previous_timestamp
            .and_then(|old| now.duration_since(old).ok())
            .map_or(0.0, |duration| duration.as_secs_f64());
        let stat = fs::read_to_string(self.root.join("stat"))?;
        let (cpu, cpu_count) = parse_cpu_times(&stat)?;
        let total_delta = cpu.total.saturating_sub(self.previous_cpu.total);
        let idle_delta = cpu.idle.saturating_sub(self.previous_cpu.idle);
        let cpu_percent = if total_delta == 0 {
            0.0
        } else {
            100.0 * (total_delta.saturating_sub(idle_delta)) as f64 / total_delta as f64
        };

        let meminfo = fs::read_to_string(self.root.join("meminfo"))?;
        let memory = parse_meminfo(&meminfo);
        let load = parse_loadavg(&fs::read_to_string(self.root.join("loadavg"))?);
        let uptime = first_number(&fs::read_to_string(self.root.join("uptime"))?).unwrap_or(0.0);
        let networks = read_networks(
            &fs::read_to_string(self.root.join("net/dev"))?,
            &self.sys_root,
        );
        let (network_receive_bytes, network_transmit_bytes) = physical_totals(&networks);
        let (disk_read_bytes, disk_write_bytes) = parse_diskstats(
            &fs::read_to_string(self.root.join("diskstats"))?,
            &self.sys_root,
        );
        let fans = read_fans(&self.sys_root);
        let temperatures = read_temperatures(&self.sys_root);
        let power = read_power(&self.sys_root);
        let battery_power = read_battery_power(&self.sys_root);
        let voltages = read_electrical_readings(&self.sys_root, "in", 1_000.0);
        let currents = read_electrical_readings(&self.sys_root, "curr", 1_000.0);
        let temperature_limits = read_temperature_limits(&self.sys_root);
        let temperature_alarms = read_temperature_alarms(&self.sys_root);
        let cpu_frequencies = read_cpu_frequencies(&self.sys_root);
        let gpus = read_gpus(&self.sys_root);
        let platform_profile =
            fs::read_to_string(self.sys_root.join("firmware/acpi/platform_profile"))
                .map(|value| value.trim().to_owned())
                .unwrap_or_default();
        let browser_tabs = read_browser_tabs(&self.runtime_root);
        let mains_supplies = read_mains_supplies(&self.sys_root);
        if self
            .previous_smart_collection
            .and_then(|old| now.duration_since(old).ok())
            .is_none_or(|elapsed| elapsed.as_secs() >= 60)
        {
            self.nvme_smart = read_nvme_smart(&self.runtime_root);
            self.previous_smart_collection = Some(now);
        }
        let filesystems = read_filesystems(&self.root, &self.filesystem_root);
        let process_network = read_process_network(&self.runtime_root);

        let mut next_process_cpu = HashMap::new();
        let mut gpu_clients: HashMap<(String, u64), (u32, u32, u64, u64)> = HashMap::new();
        let mut processes = Vec::new();
        for entry in fs::read_dir(&self.root)? {
            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => continue,
            };
            let pid = match entry.file_name().to_string_lossy().parse::<u32>() {
                Ok(pid) => pid,
                Err(_) => continue,
            };
            if let Some((mut process, cpu_ticks, clients)) =
                read_process(entry.path(), pid, self.page_size)
            {
                next_process_cpu.insert(pid, cpu_ticks);
                for client in clients {
                    // A passed descriptor leaves the context visible under both
                    // processes. Charge it to the one holding the most
                    // descriptors on it, which is the process working with the
                    // GPU rather than the one that opened the device for it.
                    let entry = gpu_clients.entry(client.key).or_insert((
                        0,
                        pid,
                        client.nanoseconds,
                        client.vram_bytes,
                    ));
                    if (client.descriptors, pid) >= (entry.0, entry.1) {
                        *entry = (
                            client.descriptors,
                            pid,
                            client.nanoseconds,
                            client.vram_bytes,
                        );
                    }
                }
                if let Some((receive, transmit)) = process_network.get(&pid) {
                    process.network_receive_bytes = *receive;
                    process.network_transmit_bytes = *transmit;
                }
                if elapsed > 0.0 {
                    let delta = cpu_ticks.saturating_sub(
                        self.previous_process_cpu
                            .get(&pid)
                            .copied()
                            .unwrap_or(cpu_ticks),
                    );
                    process.cpu_percent = delta as f64 * 100.0 / self.ticks_per_second / elapsed;
                }
                processes.push(process);
            }
        }
        let mut next_client_gpu = HashMap::new();
        let mut owners: HashMap<u32, (f64, u64)> = HashMap::new();
        for (key, (_, owner, nanoseconds, vram_bytes)) in gpu_clients {
            let previous = self
                .previous_client_gpu
                .get(&key)
                .copied()
                .unwrap_or(nanoseconds);
            next_client_gpu.insert(key, nanoseconds);
            let busy = nanoseconds.saturating_sub(previous);
            let share = owners.entry(owner).or_insert((0.0, 0));
            if elapsed > 0.0 {
                share.0 += busy as f64 / 10_000_000.0 / elapsed;
            }
            share.1 += vram_bytes;
        }
        for process in &mut processes {
            if let Some((gpu_percent, vram_bytes)) = owners.get(&process.pid) {
                process.gpu_percent = *gpu_percent;
                process.gpu_vram_bytes = *vram_bytes;
            }
        }
        self.previous_client_gpu = next_client_gpu;
        processes.sort_unstable_by(|a, b| {
            b.cpu_percent
                .total_cmp(&a.cpu_percent)
                .then_with(|| b.rss_bytes.cmp(&a.rss_bytes))
        });

        self.previous_cpu = cpu;
        self.previous_process_cpu = next_process_cpu;
        self.previous_timestamp = Some(now);
        Ok(Snapshot {
            timestamp: now.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs(),
            platform_profile,
            hostname: fs::read_to_string(self.root.join("sys/kernel/hostname"))
                .unwrap_or_else(|_| "unknown".into())
                .trim()
                .to_owned(),
            cpu_percent,
            cpu_count,
            memory_total: memory.get("MemTotal").copied().unwrap_or(0),
            memory_available: memory.get("MemAvailable").copied().unwrap_or(0),
            swap_total: memory.get("SwapTotal").copied().unwrap_or(0),
            swap_free: memory.get("SwapFree").copied().unwrap_or(0),
            load,
            uptime,
            network_receive_bytes,
            network_transmit_bytes,
            networks,
            disk_read_bytes,
            disk_write_bytes,
            processes_total: processes.len(),
            processes_running: processes
                .iter()
                .filter(|process| process.state == 'R')
                .count(),
            processes,
            fans,
            temperatures,
            power,
            battery_power,
            voltages,
            currents,
            temperature_limits,
            temperature_alarms,
            cpu_frequencies,
            gpus,
            browser_tabs,
            mains_supplies,
            nvme_smart: self.nvme_smart.clone(),
            filesystems,
        })
    }
}

fn read_filesystems(proc_root: &Path, filesystem_root: &Path) -> Vec<Filesystem> {
    let mountinfo = fs::read_to_string(proc_root.join("1/mountinfo"))
        .or_else(|_| fs::read_to_string(proc_root.join("self/mountinfo")))
        .unwrap_or_default();
    parse_mountinfo(&mountinfo, filesystem_root)
}

fn parse_mountinfo(input: &str, filesystem_root: &Path) -> Vec<Filesystem> {
    let mut filesystems = Vec::new();
    for line in input.lines() {
        let Some((mount, filesystem)) = line.split_once(" - ") else {
            continue;
        };
        let mount_fields: Vec<&str> = mount.split_whitespace().collect();
        let filesystem_fields: Vec<&str> = filesystem.split_whitespace().collect();
        if mount_fields.len() < 5 || filesystem_fields.len() < 2 {
            continue;
        }
        let filesystem_type = filesystem_fields[0];
        if is_pseudo_filesystem(filesystem_type) {
            continue;
        }
        let mountpoint = decode_mount_field(mount_fields[4]);
        if filesystems
            .iter()
            .any(|entry: &Filesystem| entry.mountpoint == mountpoint)
        {
            continue;
        }
        let path = filesystem_root.join(mountpoint.trim_start_matches('/'));
        let Some((size_bytes, available_bytes)) = filesystem_space(&path) else {
            continue;
        };
        filesystems.push(Filesystem {
            device: decode_mount_field(filesystem_fields[1]),
            mountpoint,
            filesystem_type: filesystem_type.to_owned(),
            size_bytes,
            available_bytes,
        });
    }
    filesystems.sort_unstable_by(|a, b| a.mountpoint.cmp(&b.mountpoint));
    filesystems
}

fn filesystem_space(path: &Path) -> Option<(u64, u64)> {
    let path = CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut stats = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    // SAFETY: path is a valid NUL-terminated string and stats points to writable memory.
    if unsafe { libc::statvfs(path.as_ptr(), stats.as_mut_ptr()) } != 0 {
        return None;
    }
    // SAFETY: statvfs returned success and initialized stats.
    let stats = unsafe { stats.assume_init() };
    let block_size = stats.f_frsize;
    Some((
        stats.f_blocks.saturating_mul(block_size),
        stats.f_bavail.saturating_mul(block_size),
    ))
}

fn decode_mount_field(value: &str) -> String {
    value
        .replace("\\040", " ")
        .replace("\\011", "\t")
        .replace("\\012", "\n")
        .replace("\\134", "\\")
}

fn is_pseudo_filesystem(filesystem_type: &str) -> bool {
    matches!(
        filesystem_type,
        "autofs"
            | "binfmt_misc"
            | "bpf"
            | "cgroup"
            | "cgroup2"
            | "configfs"
            | "debugfs"
            | "devpts"
            | "devtmpfs"
            | "efivarfs"
            | "fusectl"
            | "hugetlbfs"
            | "mqueue"
            | "proc"
            | "pstore"
            | "securityfs"
            | "sysfs"
            | "tmpfs"
            | "tracefs"
    )
}

fn read_fans(sys_root: &Path) -> Vec<Fan> {
    let mut fans = Vec::new();
    let Ok(chips) = fs::read_dir(sys_root.join("class/hwmon")) else {
        return fans;
    };
    for chip in chips.flatten() {
        let path = chip.path();
        let chip_name = fs::read_to_string(path.join("name"))
            .unwrap_or_else(|_| chip.file_name().to_string_lossy().into_owned())
            .trim()
            .to_owned();
        let Ok(entries) = fs::read_dir(&path) else {
            continue;
        };
        for entry in entries.flatten() {
            let file_name = entry.file_name().to_string_lossy().into_owned();
            let Some(default_sensor) = fan_sensor_name(&file_name) else {
                continue;
            };
            let Ok(rpm) = fs::read_to_string(entry.path()) else {
                continue;
            };
            let Ok(rpm) = rpm.trim().parse::<u64>() else {
                continue;
            };
            let sensor = fs::read_to_string(path.join(format!("{default_sensor}_label")))
                .unwrap_or(default_sensor)
                .trim()
                .to_owned();
            fans.push(Fan {
                chip: chip_name.clone(),
                sensor,
                rpm,
            });
        }
    }
    fans.sort_unstable_by(|a, b| (&a.chip, &a.sensor).cmp(&(&b.chip, &b.sensor)));
    fans
}

fn read_temperatures(sys_root: &Path) -> Vec<Temperature> {
    let mut temperatures = Vec::new();
    let Ok(chips) = fs::read_dir(sys_root.join("class/hwmon")) else {
        return temperatures;
    };

    for chip in chips.flatten() {
        let chip_name = fs::read_to_string(chip.path().join("name"))
            .unwrap_or_else(|_| "unknown".into())
            .trim()
            .to_owned();
        let Ok(files) = fs::read_dir(chip.path()) else {
            continue;
        };
        for file in files.flatten() {
            let file_name = file.file_name().to_string_lossy().to_string();
            let Some(number) = temperature_sensor_number(&file_name) else {
                continue;
            };
            let Ok(value) = fs::read_to_string(file.path()) else {
                continue;
            };
            let Ok(millidegrees) = value.trim().parse::<i64>() else {
                continue;
            };
            let label_path = chip.path().join(format!("temp{number}_label"));
            let sensor = fs::read_to_string(label_path)
                .ok()
                .map(|label| label.trim().to_owned())
                .filter(|label| !label.is_empty())
                .unwrap_or_else(|| format!("temp{number}"));
            temperatures.push(Temperature {
                chip: chip_name.clone(),
                sensor,
                celsius: millidegrees as f64 / 1000.0,
            });
        }
    }
    temperatures.sort_unstable_by(|a, b| (&a.chip, &a.sensor).cmp(&(&b.chip, &b.sensor)));
    temperatures
}

fn read_power(sys_root: &Path) -> Vec<Power> {
    let mut power = Vec::new();
    let Ok(chips) = fs::read_dir(sys_root.join("class/hwmon")) else {
        return power;
    };

    for chip in chips.flatten() {
        let chip_name = fs::read_to_string(chip.path().join("name"))
            .unwrap_or_else(|_| "unknown".into())
            .trim()
            .to_owned();
        let Ok(files) = fs::read_dir(chip.path()) else {
            continue;
        };
        for file in files.flatten() {
            let file_name = file.file_name().to_string_lossy().to_string();
            let Some((number, reading)) = power_sensor(&file_name) else {
                continue;
            };
            let Ok(value) = fs::read_to_string(file.path()) else {
                continue;
            };
            let Ok(microwatts) = value.trim().parse::<u64>() else {
                continue;
            };
            let label_path = chip.path().join(format!("power{number}_label"));
            let sensor = fs::read_to_string(label_path)
                .ok()
                .map(|label| label.trim().to_owned())
                .filter(|label| !label.is_empty())
                .unwrap_or_else(|| format!("power{number}"));
            power.push(Power {
                chip: chip_name.clone(),
                sensor,
                reading: reading.into(),
                watts: microwatts as f64 / 1_000_000.0,
            });
        }
    }
    power.sort_unstable_by(|a, b| {
        (&a.chip, &a.sensor, &a.reading).cmp(&(&b.chip, &b.sensor, &b.reading))
    });
    power
}

fn read_battery_power(sys_root: &Path) -> Vec<BatteryPower> {
    let mut batteries = Vec::new();
    let Ok(entries) = fs::read_dir(sys_root.join("class/power_supply")) else {
        return batteries;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if fs::read_to_string(path.join("type"))
            .ok()
            .as_deref()
            .map(str::trim)
            != Some("Battery")
        {
            continue;
        }
        let Some(voltage_uv) = read_i64(&path.join("voltage_now")) else {
            continue;
        };
        let Some(current_ua) = read_i64(&path.join("current_now")) else {
            continue;
        };
        let status = fs::read_to_string(path.join("status"))
            .unwrap_or_else(|_| "Unknown".into())
            .trim()
            .to_owned();
        batteries.push(BatteryPower {
            battery: entry.file_name().to_string_lossy().into_owned(),
            status,
            watts: battery_power_watts(voltage_uv, current_ua),
            charge_percent: read_i64(&path.join("capacity")).map(|value| value as f64),
        });
    }
    batteries.sort_unstable_by(|a, b| a.battery.cmp(&b.battery));
    batteries
}

fn read_electrical_readings(sys_root: &Path, prefix: &str, divisor: f64) -> Vec<ElectricalReading> {
    let mut readings = Vec::new();
    let Ok(chips) = fs::read_dir(sys_root.join("class/hwmon")) else {
        return readings;
    };
    for chip in chips.flatten() {
        let chip_name = fs::read_to_string(chip.path().join("name"))
            .unwrap_or_else(|_| "unknown".into())
            .trim()
            .to_owned();
        let Ok(files) = fs::read_dir(chip.path()) else {
            continue;
        };
        for file in files.flatten() {
            let file_name = file.file_name().to_string_lossy().to_string();
            let Some(number) = input_sensor_number(&file_name, prefix) else {
                continue;
            };
            let Some(value) = read_i64(&file.path()) else {
                continue;
            };
            let sensor = fs::read_to_string(chip.path().join(format!("{prefix}{number}_label")))
                .ok()
                .map(|label| label.trim().to_owned())
                .filter(|label| !label.is_empty())
                .unwrap_or_else(|| format!("{prefix}{number}"));
            readings.push(ElectricalReading {
                chip: chip_name.clone(),
                sensor,
                value: value as f64 / divisor,
            });
        }
    }
    readings.sort_unstable_by(|a, b| (&a.chip, &a.sensor).cmp(&(&b.chip, &b.sensor)));
    readings
}

fn read_temperature_limits(sys_root: &Path) -> Vec<TemperatureLimit> {
    let mut limits = Vec::new();
    let Ok(chips) = fs::read_dir(sys_root.join("class/hwmon")) else {
        return limits;
    };
    for chip in chips.flatten() {
        let chip_name = fs::read_to_string(chip.path().join("name"))
            .unwrap_or_else(|_| "unknown".into())
            .trim()
            .to_owned();
        let Ok(files) = fs::read_dir(chip.path()) else {
            continue;
        };
        for file in files.flatten() {
            let file_name = file.file_name().to_string_lossy().to_string();
            let Some((number, limit)) = temperature_limit(&file_name) else {
                continue;
            };
            let Some(millidegrees) = read_i64(&file.path()) else {
                continue;
            };
            let celsius = millidegrees as f64 / 1_000.0;
            if !(-100.0..=250.0).contains(&celsius) {
                continue;
            }
            let sensor = fs::read_to_string(chip.path().join(format!("temp{number}_label")))
                .ok()
                .map(|label| label.trim().to_owned())
                .filter(|label| !label.is_empty())
                .unwrap_or_else(|| format!("temp{number}"));
            limits.push(TemperatureLimit {
                chip: chip_name.clone(),
                sensor,
                limit: limit.into(),
                celsius,
            });
        }
    }
    limits.sort_unstable_by(|a, b| {
        (&a.chip, &a.sensor, &a.limit).cmp(&(&b.chip, &b.sensor, &b.limit))
    });
    limits
}

fn read_temperature_alarms(sys_root: &Path) -> Vec<TemperatureAlarm> {
    let mut alarms = Vec::new();
    let Ok(chips) = fs::read_dir(sys_root.join("class/hwmon")) else {
        return alarms;
    };
    for chip in chips.flatten() {
        let chip_name = fs::read_to_string(chip.path().join("name"))
            .unwrap_or_else(|_| "unknown".into())
            .trim()
            .to_owned();
        let Ok(files) = fs::read_dir(chip.path()) else {
            continue;
        };
        for file in files.flatten() {
            let file_name = file.file_name().to_string_lossy().to_string();
            let Some(number) = sensor_number_with_suffix(&file_name, "temp", "alarm") else {
                continue;
            };
            let Some(value) = read_i64(&file.path()).and_then(|value| u64::try_from(value).ok())
            else {
                continue;
            };
            let sensor = fs::read_to_string(chip.path().join(format!("temp{number}_label")))
                .ok()
                .map(|label| label.trim().to_owned())
                .filter(|label| !label.is_empty())
                .unwrap_or_else(|| format!("temp{number}"));
            alarms.push(TemperatureAlarm {
                chip: chip_name.clone(),
                sensor,
                value,
            });
        }
    }
    alarms.sort_unstable_by(|a, b| (&a.chip, &a.sensor).cmp(&(&b.chip, &b.sensor)));
    alarms
}

fn read_cpu_frequencies(sys_root: &Path) -> Vec<CpuFrequency> {
    let mut frequencies = Vec::new();
    let Ok(policies) = fs::read_dir(sys_root.join("devices/system/cpu/cpufreq")) else {
        return frequencies;
    };
    for policy in policies.flatten() {
        let Ok(cpus) = fs::read_to_string(policy.path().join("affected_cpus")) else {
            continue;
        };
        let Some(kilohertz) = read_i64(&policy.path().join("scaling_cur_freq")) else {
            continue;
        };
        for cpu in cpus.split_whitespace() {
            frequencies.push(CpuFrequency {
                cpu: cpu.into(),
                hertz: kilohertz.max(0) as u64 * 1_000,
            });
        }
    }
    frequencies.sort_unstable_by(|a, b| a.cpu.cmp(&b.cpu));
    frequencies
}

/// Utilisation and VRAM as the DRM driver reports them. Intel and Nouveau leave
/// `gpu_busy_percent` out, so cards without it are skipped rather than reported
/// as idle.
fn read_gpus(sys_root: &Path) -> Vec<Gpu> {
    let mut gpus = Vec::new();
    let Ok(cards) = fs::read_dir(sys_root.join("class/drm")) else {
        return gpus;
    };
    for card in cards.flatten() {
        let name = card.file_name().to_string_lossy().into_owned();
        // class/drm also holds connectors like card1-eDP-1 and render nodes.
        if !name
            .strip_prefix("card")
            .is_some_and(|index| !index.is_empty() && index.bytes().all(|b| b.is_ascii_digit()))
        {
            continue;
        }
        let device = card.path().join("device");
        let Some(busy) = read_i64(&device.join("gpu_busy_percent")) else {
            continue;
        };
        let driver = fs::read_link(device.join("driver"))
            .ok()
            .and_then(|path| {
                path.file_name()
                    .map(|name| name.to_string_lossy().into_owned())
            })
            .unwrap_or_else(|| "unknown".into());
        gpus.push(Gpu {
            card: name,
            driver,
            busy_percent: busy as f64,
            vram_used_bytes: read_i64(&device.join("mem_info_vram_used"))
                .unwrap_or(0)
                .max(0) as u64,
            vram_total_bytes: read_i64(&device.join("mem_info_vram_total"))
                .unwrap_or(0)
                .max(0) as u64,
        });
    }
    gpus.sort_unstable_by(|a, b| a.card.cmp(&b.card));
    gpus
}

/// One GPU context as the DRM fdinfo interface reports it. The counters belong
/// to the context rather than to a process: logind opens the device and passes
/// the descriptor on, so one context appears under several processes and has to
/// be charged to a single one of them.
#[derive(Clone, Debug)]
struct DrmClient {
    /// Client ids restart per device, so the address is part of the identity.
    key: (String, u64),
    nanoseconds: u64,
    vram_bytes: u64,
    descriptors: u32,
}

/// The DRM contexts one process holds. Engines run in parallel, so their busy
/// times are summed and the result can pass 100% the same way process CPU does
/// across cores.
fn read_process_gpu(path: &Path) -> Vec<DrmClient> {
    let Ok(descriptors) = fs::read_dir(path.join("fd")) else {
        return Vec::new();
    };
    let mut clients: HashMap<(String, u64), DrmClient> = HashMap::new();
    for descriptor in descriptors.flatten() {
        // Reading a link is far cheaper than opening fdinfo, and hardly any
        // process holds a DRM handle, so the cheap test comes first.
        if !fs::read_link(descriptor.path())
            .is_ok_and(|target| target.starts_with(Path::new("/dev/dri")))
        {
            continue;
        }
        let Ok(info) = fs::read_to_string(path.join("fdinfo").join(descriptor.file_name())) else {
            continue;
        };
        let Some(client) = parse_drm_fdinfo(&info) else {
            continue;
        };
        // Descriptors duplicated inside one process repeat a single context.
        clients
            .entry(client.key.clone())
            .and_modify(|known| known.descriptors += 1)
            .or_insert(client);
    }
    clients.into_values().collect()
}

fn parse_drm_fdinfo(input: &str) -> Option<DrmClient> {
    let mut device = None;
    let mut client = None;
    let mut nanoseconds = 0;
    let mut vram_bytes = 0;
    for line in input.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        let amount = || {
            value
                .split_whitespace()
                .next()
                .and_then(|amount| amount.parse::<u64>().ok())
                .unwrap_or(0)
        };
        match key.trim() {
            "drm-pdev" => device = Some(value.to_owned()),
            "drm-client-id" => client = Some(amount()),
            "drm-memory-vram" => vram_bytes = amount() * 1024,
            key if key.starts_with("drm-engine-") => nanoseconds += amount(),
            _ => {}
        }
    }
    Some(DrmClient {
        key: (device.unwrap_or_default(), client?),
        nanoseconds,
        vram_bytes,
        descriptors: 1,
    })
}

fn read_mains_supplies(sys_root: &Path) -> Vec<MainsSupply> {
    let mut supplies = Vec::new();
    let Ok(entries) = fs::read_dir(sys_root.join("class/power_supply")) else {
        return supplies;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if fs::read_to_string(path.join("type"))
            .ok()
            .as_deref()
            .map(str::trim)
            != Some("Mains")
        {
            continue;
        }
        let online = read_i64(&path.join("online")).unwrap_or(0) != 0;
        supplies.push(MainsSupply {
            supply: entry.file_name().to_string_lossy().into_owned(),
            online,
        });
    }
    supplies.sort_unstable_by(|a, b| a.supply.cmp(&b.supply));
    supplies
}

fn read_nvme_smart(runtime_root: &Path) -> Vec<NvmeSmart> {
    let Ok(json) = fs::read_to_string(runtime_root.join("nvme-smart.json")) else {
        return Vec::new();
    };
    let Some(smart) = json
        .split_once("\"nvme_smart_health_information_log\"")
        .map(|(_, rest)| rest)
    else {
        return Vec::new();
    };
    let Some(percentage_used) = json_u64(smart, "percentage_used") else {
        return Vec::new();
    };
    let Some(available_spare) = json_u64(smart, "available_spare") else {
        return Vec::new();
    };
    vec![NvmeSmart {
        device: json_string(&json, "name").unwrap_or_else(|| "nvme0".into()),
        percentage_used,
        available_spare,
        available_spare_threshold: json_u64(smart, "available_spare_threshold").unwrap_or(0),
        critical_warning: json_u64(smart, "critical_warning").unwrap_or(0),
        data_units_read: json_u64(smart, "data_units_read").unwrap_or(0),
        data_units_written: json_u64(smart, "data_units_written").unwrap_or(0),
        power_cycles: json_u64(smart, "power_cycles").unwrap_or(0),
        power_on_hours: json_u64(smart, "power_on_hours").unwrap_or(0),
        unsafe_shutdowns: json_u64(smart, "unsafe_shutdowns").unwrap_or(0),
        media_errors: json_u64(smart, "media_errors").unwrap_or(0),
        error_log_entries: json_u64(smart, "num_err_log_entries").unwrap_or(0),
    }]
}

/// Linux exposes no per-process network counters: `/proc/PID/net/dev` describes
/// the whole network namespace, not the process reading it. The optional
/// `bbtop-net` collector traces the socket layer with eBPF and publishes a table
/// of cumulative payload bytes per PID; without it these counters stay at zero.
/// A browser tab has no identity the kernel can see: content processes carry no
/// origin, and one process may host several tabs. The optional `bbtop-tabs`
/// collector asks the browser itself and leaves the answer here.
fn read_browser_tabs(runtime_root: &Path) -> Vec<BrowserTab> {
    fs::read_to_string(runtime_root.join("browser-tabs.txt"))
        .map(|input| parse_browser_tabs(&input))
        .unwrap_or_default()
}

fn parse_browser_tabs(input: &str) -> Vec<BrowserTab> {
    input
        .lines()
        .filter_map(|line| {
            let mut fields = line.splitn(6, char::is_whitespace);
            let pid = fields.next()?.parse().ok()?;
            let cpu_percent = fields.next()?.parse().ok()?;
            let memory_bytes = fields.next()?.parse().ok()?;
            let windows = fields.next()?.parse().ok()?;
            let site = fields.next()?.to_owned();
            // The title runs to the end of the line and may contain spaces.
            let title = fields.next()?.trim().to_owned();
            Some(BrowserTab {
                pid,
                cpu_percent,
                memory_bytes,
                windows,
                site,
                title,
            })
        })
        .collect()
}

fn read_process_network(runtime_root: &Path) -> HashMap<u32, (u64, u64)> {
    fs::read_to_string(runtime_root.join("process-net.txt"))
        .map(|input| parse_process_network(&input))
        .unwrap_or_default()
}

fn parse_process_network(input: &str) -> HashMap<u32, (u64, u64)> {
    input
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let pid = fields.next()?.parse().ok()?;
            let receive = fields.next()?.parse().ok()?;
            let transmit = fields.next()?.parse().ok()?;
            Some((pid, (receive, transmit)))
        })
        .collect()
}

fn json_u64(input: &str, key: &str) -> Option<u64> {
    let (_, value) = input.split_once(&format!("\"{key}\":"))?;
    value
        .trim_start()
        .split(|ch: char| !ch.is_ascii_digit())
        .next()?
        .parse()
        .ok()
}

fn json_string(input: &str, key: &str) -> Option<String> {
    let (_, value) = input.split_once(&format!("\"{key}\":"))?;
    let value = value.trim_start().strip_prefix('"')?;
    Some(value.split_once('"')?.0.into())
}

fn read_i64(path: &Path) -> Option<i64> {
    fs::read_to_string(path).ok()?.trim().parse().ok()
}

fn battery_power_watts(voltage_uv: i64, current_ua: i64) -> f64 {
    voltage_uv.unsigned_abs() as f64 * current_ua.unsigned_abs() as f64 / 1_000_000_000_000.0
}

fn fan_sensor_name(file_name: &str) -> Option<String> {
    let number = file_name.strip_prefix("fan")?.strip_suffix("_input")?;
    (!number.is_empty() && number.chars().all(|character| character.is_ascii_digit()))
        .then(|| format!("fan{number}"))
}

fn temperature_sensor_number(file_name: &str) -> Option<&str> {
    let number = file_name.strip_prefix("temp")?.strip_suffix("_input")?;
    (!number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit())).then_some(number)
}

fn power_sensor(file_name: &str) -> Option<(&str, &str)> {
    let number = file_name.strip_prefix("power")?;
    for (suffix, reading) in [("_input", "input"), ("_average", "average")] {
        if let Some(number) = number.strip_suffix(suffix) {
            return (!number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit()))
                .then_some((number, reading));
        }
    }
    None
}

fn input_sensor_number<'a>(file_name: &'a str, prefix: &str) -> Option<&'a str> {
    sensor_number_with_suffix(file_name, prefix, "input")
}

fn sensor_number_with_suffix<'a>(
    file_name: &'a str,
    prefix: &str,
    suffix: &str,
) -> Option<&'a str> {
    let number = file_name
        .strip_prefix(prefix)?
        .strip_suffix(&format!("_{suffix}"))?;
    (!number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit())).then_some(number)
}

fn temperature_limit(file_name: &str) -> Option<(&str, &str)> {
    let number = file_name.strip_prefix("temp")?;
    for suffix in ["min", "max", "crit", "emergency"] {
        if let Some(number) = number.strip_suffix(&format!("_{suffix}")) {
            return (!number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit()))
                .then_some((number, suffix));
        }
    }
    None
}

fn parse_cpu_times(input: &str) -> io::Result<(CpuTimes, usize)> {
    let mut lines = input.lines();
    let fields: Vec<u64> = lines
        .next()
        .ok_or_else(|| io::Error::other("missing aggregate CPU line"))?
        .split_whitespace()
        .skip(1)
        .filter_map(|field| field.parse().ok())
        .collect();
    if fields.len() < 4 {
        return Err(io::Error::other("invalid aggregate CPU line"));
    }
    let idle = fields[3] + fields.get(4).copied().unwrap_or(0);
    let total = fields.iter().sum();
    let cpu_count = input
        .lines()
        .filter(|line| {
            line.strip_prefix("cpu")
                .and_then(|rest| rest.chars().next())
                .is_some_and(|character| character.is_ascii_digit())
        })
        .count();
    Ok((CpuTimes { total, idle }, cpu_count))
}

fn parse_meminfo(input: &str) -> HashMap<&str, u64> {
    input
        .lines()
        .filter_map(|line| {
            let (key, rest) = line.split_once(':')?;
            let value = rest.split_whitespace().next()?.parse::<u64>().ok()?;
            Some((key, value * 1024))
        })
        .collect()
}

fn parse_loadavg(input: &str) -> [f64; 3] {
    let mut values = input
        .split_whitespace()
        .filter_map(|value| value.parse().ok());
    [
        values.next().unwrap_or(0.0),
        values.next().unwrap_or(0.0),
        values.next().unwrap_or(0.0),
    ]
}

fn first_number(input: &str) -> Option<f64> {
    input.split_whitespace().next()?.parse().ok()
}

fn read_networks(input: &str, sys_root: &Path) -> Vec<NetworkInterface> {
    let mut interfaces = parse_net_dev(input);
    for interface in &mut interfaces {
        interface.kind = if sys_root
            .join("class/net")
            .join(&interface.name)
            .join("device")
            .exists()
        {
            "physical".into()
        } else {
            "virtual".into()
        };
    }
    interfaces
}

/// Every interface gets its own series so that a counter reset stays local to
/// it. Summing them into one number made a container teardown look like a
/// multi-gigabyte burst: the total dropped with the departing interface, and
/// Prometheus reads a falling counter as a restart. Veth pairs are skipped
/// because each container lifetime would otherwise leave a dead series behind.
fn parse_net_dev(input: &str) -> Vec<NetworkInterface> {
    let mut interfaces: Vec<NetworkInterface> = input
        .lines()
        .filter_map(|line| {
            let (name, fields) = line.split_once(':')?;
            let name = name.trim();
            if name.is_empty() || name.starts_with("veth") {
                return None;
            }
            let values: Vec<u64> = fields
                .split_whitespace()
                .filter_map(|value| value.parse().ok())
                .collect();
            Some(NetworkInterface {
                name: name.to_owned(),
                kind: String::new(),
                receive_bytes: values.first().copied().unwrap_or(0),
                transmit_bytes: values.get(8).copied().unwrap_or(0),
            })
        })
        .collect();
    interfaces.sort_unstable_by(|a, b| a.name.cmp(&b.name));
    interfaces
}

fn physical_totals(interfaces: &[NetworkInterface]) -> (u64, u64) {
    interfaces
        .iter()
        .filter(|interface| interface.kind == "physical")
        .fold((0, 0), |(receive, transmit), interface| {
            (
                receive + interface.receive_bytes,
                transmit + interface.transmit_bytes,
            )
        })
}

fn parse_diskstats(input: &str, sys_root: &Path) -> (u64, u64) {
    input.lines().fold((0, 0), |(read, written), line| {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() < 14 || fields[2].starts_with("loop") || fields[2].starts_with("ram") {
            return (read, written);
        }
        let device = sys_root
            .join("dev/block")
            .join(format!("{}:{}", fields[0], fields[1]));
        if device.join("partition").exists() {
            return (read, written);
        }
        let sectors_read = fields[5].parse::<u64>().unwrap_or(0);
        let sectors_written = fields[9].parse::<u64>().unwrap_or(0);
        (read + sectors_read * 512, written + sectors_written * 512)
    })
}

fn read_process(path: PathBuf, pid: u32, page_size: u64) -> Option<(Process, u64, Vec<DrmClient>)> {
    let stat = fs::read_to_string(path.join("stat")).ok()?;
    let name_start = stat.find('(')? + 1;
    let name_end = stat.rfind(')')?;
    let name = stat[name_start..name_end].to_owned();
    let fields: Vec<&str> = stat[name_end + 2..].split_whitespace().collect();
    let state = fields.first()?.chars().next()?;
    let user_ticks = fields.get(11)?.parse::<u64>().ok()?;
    let system_ticks = fields.get(12)?.parse::<u64>().ok()?;
    let threads = fields.get(17)?.parse::<u64>().unwrap_or(0);
    let virtual_bytes = fields.get(20)?.parse::<u64>().unwrap_or(0);
    let rss_pages = fields.get(21)?.parse::<u64>().unwrap_or(0);
    let gpu_clients = read_process_gpu(&path);
    let (read_bytes, write_bytes) = fs::read_to_string(path.join("io"))
        .ok()
        .map(|input| {
            let values: HashMap<&str, u64> = input
                .lines()
                .filter_map(|line| {
                    let (key, value) = line.split_once(':')?;
                    Some((key, value.trim().parse().ok()?))
                })
                .collect();
            (
                values.get("read_bytes").copied().unwrap_or(0),
                values.get("write_bytes").copied().unwrap_or(0),
            )
        })
        .unwrap_or_default();
    Some((
        Process {
            pid,
            name,
            state,
            cpu_percent: 0.0,
            rss_bytes: rss_pages * page_size,
            virtual_bytes,
            threads,
            read_bytes,
            write_bytes,
            network_receive_bytes: 0,
            network_transmit_bytes: 0,
            gpu_percent: 0.0,
            gpu_vram_bytes: 0,
        },
        user_ticks + system_ticks,
        gpu_clients,
    ))
}

fn detect_page_size(root: &Path) -> u64 {
    fs::read_to_string(root.join("self/smaps"))
        .ok()
        .and_then(|input| {
            input.lines().find_map(|line| {
                let value = line.strip_prefix("KernelPageSize:")?;
                value.split_whitespace().next()?.parse::<u64>().ok()
            })
        })
        .map(|kilobytes| kilobytes * 1024)
        .unwrap_or(4096)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_cpu_and_core_count() {
        let input = "cpu  100 2 30 400 10 0 3 0 0 0\ncpu0 1 2 3 4\ncpu1 1 2 3 4\n";
        let (times, cores) = parse_cpu_times(input).unwrap();
        assert_eq!(times.total, 545);
        assert_eq!(times.idle, 410);
        assert_eq!(cores, 2);
    }

    #[test]
    fn parses_memory_as_bytes() {
        let values = parse_meminfo("MemTotal: 100 kB\nMemAvailable: 40 kB\n");
        assert_eq!(values["MemTotal"], 102_400);
        assert_eq!(values["MemAvailable"], 40_960);
    }

    #[test]
    fn parses_network_interfaces_and_skips_veth() {
        let input = "Inter-| Receive | Transmit\n \
             eth0: 10 0 0 0 0 0 0 0 20 0 0 0 0 0 0 0\n \
             veth1234@if2: 30 0 0 0 0 0 0 0 40 0 0 0 0 0 0 0\n \
             lo: 50 0 0 0 0 0 0 0 60 0 0 0 0 0 0 0\n";
        let interfaces = parse_net_dev(input);
        assert_eq!(
            interfaces
                .iter()
                .map(|interface| interface.name.as_str())
                .collect::<Vec<_>>(),
            ["eth0", "lo"]
        );
        assert_eq!(interfaces[0].receive_bytes, 10);
        assert_eq!(interfaces[0].transmit_bytes, 20);
    }

    #[test]
    fn totals_count_physical_interfaces_only() {
        let interfaces = vec![
            NetworkInterface {
                name: "eth0".into(),
                kind: "physical".into(),
                receive_bytes: 10,
                transmit_bytes: 20,
            },
            NetworkInterface {
                name: "docker0".into(),
                kind: "virtual".into(),
                receive_bytes: 1_000,
                transmit_bytes: 2_000,
            },
        ];
        assert_eq!(physical_totals(&interfaces), (10, 20));
    }

    #[test]
    fn recognizes_fan_input_names() {
        assert_eq!(fan_sensor_name("fan12_input").as_deref(), Some("fan12"));
        assert_eq!(fan_sensor_name("fan1_min"), None);
        assert_eq!(fan_sensor_name("temp1_input"), None);
    }

    #[test]
    fn parses_drm_fdinfo() {
        let input = "pos:\t0\ndrm-driver:\tamdgpu\ndrm-pdev:\t0000:c1:00.0\n\
             drm-client-id:\t13\ndrm-memory-vram:\t868 KiB\ndrm-engine-gfx:\t1000 ns\n\
             drm-engine-compute:\t500 ns\n";
        let client = parse_drm_fdinfo(input).unwrap();
        assert_eq!(client.key, ("0000:c1:00.0".to_owned(), 13));
        assert_eq!(client.nanoseconds, 1_500);
        assert_eq!(client.vram_bytes, 868 * 1024);
        assert_eq!(client.descriptors, 1);
    }

    #[test]
    fn ignores_fdinfo_without_a_drm_client() {
        assert!(parse_drm_fdinfo("pos:\t0\nflags:\t02\n").is_none());
    }

    #[test]
    fn parses_browser_tabs_with_spaced_titles() {
        let table = parse_browser_tabs(
            "# pid cpu_percent memory_bytes windows site title\n\
             7007 57.3 1552 2 grafana.dubrovskih.ru Grafana - bbtop Linux overview\n\
             bad line\n",
        );
        assert_eq!(table.len(), 1);
        assert_eq!(table[0].pid, 7007);
        assert_eq!(table[0].cpu_percent, 57.3);
        assert_eq!(table[0].windows, 2);
        assert_eq!(table[0].site, "grafana.dubrovskih.ru");
        assert_eq!(table[0].title, "Grafana - bbtop Linux overview");
    }

    #[test]
    fn parses_process_network_table() {
        let table = parse_process_network("# pid receive_bytes transmit_bytes\n42 100 200\nbad\n");
        assert_eq!(table[&42], (100, 200));
        assert_eq!(table.len(), 1);
    }

    #[test]
    fn decodes_mountinfo_fields() {
        assert_eq!(decode_mount_field("/media/My\\040Disk"), "/media/My Disk");
        assert!(is_pseudo_filesystem("tmpfs"));
        assert!(is_pseudo_filesystem("efivarfs"));
        assert!(!is_pseudo_filesystem("ext4"));
    }
}
