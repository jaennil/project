use std::{
    io::{self, Read, Write},
    process::Command,
    sync::{Arc, RwLock, mpsc},
    thread,
    time::Duration,
};

use crate::procfs::{Process, Snapshot};

#[derive(Clone, Copy)]
enum SortBy {
    Cpu,
    Memory,
    Read,
    Write,
    Network,
    Gpu,
    Pid,
}

struct TerminalGuard;

impl TerminalGuard {
    fn enter() -> io::Result<Self> {
        let status = Command::new("stty")
            .args(["-echo", "-icanon", "min", "0", "time", "1"])
            .status()?;
        if !status.success() {
            return Err(io::Error::other("failed to enable raw terminal mode"));
        }
        print!("\x1b[?1049h\x1b[?25l");
        io::stdout().flush()?;
        Ok(Self)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = Command::new("stty").arg("sane").status();
        print!("\x1b[?25h\x1b[?1049l");
        let _ = io::stdout().flush();
    }
}

pub fn run(state: Arc<RwLock<Snapshot>>, listen: &str, interval: Duration) -> io::Result<()> {
    if !io::IsTerminal::is_terminal(&io::stdin()) || !io::IsTerminal::is_terminal(&io::stdout()) {
        return Err(io::Error::other("TUI needs a terminal; use --no-tui"));
    }
    let _guard = TerminalGuard::enter()?;
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let mut input = io::stdin();
        loop {
            let mut byte = [0];
            if input.read_exact(&mut byte).is_ok() {
                let _ = sender.send(byte[0]);
            }
        }
    });
    let mut sort = SortBy::Cpu;
    loop {
        while let Ok(key) = receiver.try_recv() {
            sort = match key {
                b'q' | 3 => return Ok(()),
                b'c' => SortBy::Cpu,
                b'm' => SortBy::Memory,
                b'r' => SortBy::Read,
                b'w' => SortBy::Write,
                b'n' => SortBy::Network,
                b'g' => SortBy::Gpu,
                b'p' => SortBy::Pid,
                _ => sort,
            };
        }
        draw(&state.read().unwrap(), listen, sort)?;
        thread::sleep(interval.min(Duration::from_millis(500)));
    }
}

fn draw(snapshot: &Snapshot, listen: &str, sort: SortBy) -> io::Result<()> {
    let (width, height) = terminal_size();
    let mut processes = snapshot.processes.clone();
    processes.sort_unstable_by(|a, b| match sort {
        SortBy::Cpu => b.cpu_percent.total_cmp(&a.cpu_percent),
        SortBy::Memory => b.rss_bytes.cmp(&a.rss_bytes),
        SortBy::Read => b.read_bytes.cmp(&a.read_bytes),
        SortBy::Write => b.write_bytes.cmp(&a.write_bytes),
        SortBy::Network => network_bytes(b).cmp(&network_bytes(a)),
        SortBy::Gpu => b.gpu_percent.total_cmp(&a.gpu_percent),
        SortBy::Pid => a.pid.cmp(&b.pid),
    });
    let used_memory = snapshot
        .memory_total
        .saturating_sub(snapshot.memory_available);
    let memory_percent = percent(used_memory, snapshot.memory_total);
    let swap_used = snapshot.swap_total.saturating_sub(snapshot.swap_free);
    let mut output = String::with_capacity(width * height);
    output.push_str("\x1b[H\x1b[2J\x1b[1;36m bbtop\x1b[0m  Linux observability console");
    output.push_str(&format!(
        "  \x1b[2m{}  :{}\x1b[0m\n",
        snapshot.hostname, listen
    ));
    output.push_str(&format!(
        " CPU {:>5.1}% {}  load {:.2} {:.2} {:.2}  uptime {}\n",
        snapshot.cpu_percent,
        bar(snapshot.cpu_percent, 24),
        snapshot.load[0],
        snapshot.load[1],
        snapshot.load[2],
        duration(snapshot.uptime as u64)
    ));
    output.push_str(&format!(
        " MEM {:>5.1}% {}  {} / {}  SWAP {}\n",
        memory_percent,
        bar(memory_percent, 24),
        bytes(used_memory),
        bytes(snapshot.memory_total),
        bytes(swap_used)
    ));
    output.push_str(&format!(
        " NET rx {}  tx {}    DISK read {}  write {}    TASKS {} ({} running)\n",
        bytes(snapshot.network_receive_bytes),
        bytes(snapshot.network_transmit_bytes),
        bytes(snapshot.disk_read_bytes),
        bytes(snapshot.disk_write_bytes),
        snapshot.processes_total,
        snapshot.processes_running
    ));
    output.push_str(&format!(" {}\n\n", gpu_summary(snapshot)));
    output.push_str(
        "\x1b[1m     PID S    CPU%      RSS     READ    WRITE    NETRX    NETTX    GPU%     VRAM  NAME\x1b[0m\n",
    );
    let rows = height.saturating_sub(10);
    for process in processes.iter().take(rows) {
        output.push_str(&process_row(process, width));
    }
    while output.bytes().filter(|byte| *byte == b'\n').count() < height.saturating_sub(1) {
        output.push('\n');
    }
    output.push_str(
        "\x1b[7m q quit │ sort: c cpu  m memory  r read  w write  n net  g gpu  p pid \x1b[0m",
    );
    print!("{output}");
    io::stdout().flush()
}

fn process_row(process: &Process, width: usize) -> String {
    let fixed = 84;
    let name_width = width.saturating_sub(fixed).max(8);
    let name: String = process.name.chars().take(name_width).collect();
    format!(
        " {:>7} {} {:>7.1} {:>8} {:>8} {:>8} {:>8} {:>8} {:>7.1} {:>8}  {}\n",
        process.pid,
        process.state,
        process.cpu_percent,
        bytes(process.rss_bytes),
        bytes(process.read_bytes),
        bytes(process.write_bytes),
        bytes(process.network_receive_bytes),
        bytes(process.network_transmit_bytes),
        process.gpu_percent,
        bytes(process.gpu_vram_bytes),
        name
    )
}

fn gpu_summary(snapshot: &Snapshot) -> String {
    if snapshot.gpus.is_empty() {
        return "GPU none detected".into();
    }
    snapshot
        .gpus
        .iter()
        .map(|gpu| {
            format!(
                "GPU {} {:>3.0}%  VRAM {} / {}",
                gpu.driver,
                gpu.busy_percent,
                bytes(gpu.vram_used_bytes),
                bytes(gpu.vram_total_bytes)
            )
        })
        .collect::<Vec<_>>()
        .join("    ")
}

fn network_bytes(process: &Process) -> u64 {
    process
        .network_receive_bytes
        .saturating_add(process.network_transmit_bytes)
}

fn terminal_size() -> (usize, usize) {
    let output = Command::new("stty").arg("size").output().ok();
    output
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .and_then(|text| {
            let mut fields = text.split_whitespace();
            let rows = fields.next()?.parse().ok()?;
            let columns = fields.next()?.parse().ok()?;
            Some((columns, rows))
        })
        .unwrap_or((120, 30))
}

fn bar(value: f64, width: usize) -> String {
    let filled = ((value.clamp(0.0, 100.0) / 100.0) * width as f64).round() as usize;
    format!(
        "\x1b[32m{}\x1b[2m{}\x1b[0m",
        "█".repeat(filled),
        "░".repeat(width - filled)
    )
}

fn percent(value: u64, total: u64) -> f64 {
    if total == 0 {
        0.0
    } else {
        value as f64 * 100.0 / total as f64
    }
}

fn bytes(value: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut amount = value as f64;
    let mut unit = 0;
    while amount >= 1024.0 && unit < UNITS.len() - 1 {
        amount /= 1024.0;
        unit += 1;
    }
    format!("{amount:.1}{}", UNITS[unit])
}

fn duration(seconds: u64) -> String {
    let days = seconds / 86_400;
    let hours = seconds % 86_400 / 3_600;
    let minutes = seconds % 3_600 / 60;
    format!("{days}d {hours:02}h {minutes:02}m")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_byte_units() {
        assert_eq!(bytes(1024), "1.0KiB");
        assert_eq!(bytes(1_073_741_824), "1.0GiB");
    }

    #[test]
    fn formats_duration() {
        assert_eq!(duration(90_060), "1d 01h 01m");
    }
}
