use std::{
    collections::HashMap,
    fs, io,
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
}

#[derive(Clone, Debug, Default)]
pub struct Fan {
    pub chip: String,
    pub sensor: String,
    pub rpm: u64,
}

#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub timestamp: u64,
    pub hostname: String,
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
    pub disk_read_bytes: u64,
    pub disk_write_bytes: u64,
    pub processes_total: usize,
    pub processes_running: usize,
    pub processes: Vec<Process>,
    pub fans: Vec<Fan>,
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
    ticks_per_second: f64,
    page_size: u64,
    previous_cpu: CpuTimes,
    previous_process_cpu: HashMap<u32, u64>,
    previous_timestamp: Option<SystemTime>,
}

impl Collector {
    pub fn new(root: PathBuf) -> Self {
        let page_size = detect_page_size(&root);
        let sys_root = if root == Path::new("/proc") {
            PathBuf::from("/sys")
        } else {
            root.parent().unwrap_or(Path::new("/")).join("sys")
        };
        Self {
            root,
            sys_root,
            ticks_per_second: 100.0,
            page_size,
            previous_cpu: CpuTimes::default(),
            previous_process_cpu: HashMap::new(),
            previous_timestamp: None,
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
        let (network_receive_bytes, network_transmit_bytes) =
            parse_net_dev(&fs::read_to_string(self.root.join("net/dev"))?);
        let (disk_read_bytes, disk_write_bytes) = parse_diskstats(
            &fs::read_to_string(self.root.join("diskstats"))?,
            &self.sys_root,
        );
        let fans = read_fans(&self.sys_root);

        let mut next_process_cpu = HashMap::new();
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
            if let Some((mut process, cpu_ticks)) = read_process(entry.path(), pid, self.page_size)
            {
                next_process_cpu.insert(pid, cpu_ticks);
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
            disk_read_bytes,
            disk_write_bytes,
            processes_total: processes.len(),
            processes_running: processes
                .iter()
                .filter(|process| process.state == 'R')
                .count(),
            processes,
            fans,
        })
    }
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

fn fan_sensor_name(file_name: &str) -> Option<String> {
    let number = file_name.strip_prefix("fan")?.strip_suffix("_input")?;
    (!number.is_empty() && number.chars().all(|character| character.is_ascii_digit()))
        .then(|| format!("fan{number}"))
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

fn parse_net_dev(input: &str) -> (u64, u64) {
    input.lines().filter_map(|line| line.split_once(':')).fold(
        (0, 0),
        |(receive, transmit), (_, fields)| {
            let values: Vec<u64> = fields
                .split_whitespace()
                .filter_map(|value| value.parse().ok())
                .collect();
            (
                receive + values.first().copied().unwrap_or(0),
                transmit + values.get(8).copied().unwrap_or(0),
            )
        },
    )
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

fn read_process(path: PathBuf, pid: u32, page_size: u64) -> Option<(Process, u64)> {
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
        },
        user_ticks + system_ticks,
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
    fn parses_network_totals() {
        let input = "Inter-| Receive | Transmit\n eth0: 10 0 0 0 0 0 0 0 20 0 0 0 0 0 0 0\n";
        assert_eq!(parse_net_dev(input), (10, 20));
    }

    #[test]
    fn recognizes_fan_input_names() {
        assert_eq!(fan_sensor_name("fan12_input").as_deref(), Some("fan12"));
        assert_eq!(fan_sensor_name("fan1_min"), None);
        assert_eq!(fan_sensor_name("temp1_input"), None);
    }
}
