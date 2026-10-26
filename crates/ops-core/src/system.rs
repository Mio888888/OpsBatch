use serde::Serialize;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

#[derive(Debug, Clone, Serialize)]
pub struct LocalPerformanceSnapshot {
    pub cpu_time_ms: Option<f64>,
    pub memory_rss_mb: Option<u64>,
    pub logical_cpu_count: Option<usize>,
    pub timestamp: u64,
}
pub async fn get_local_performance_snapshot() -> Result<LocalPerformanceSnapshot, String> {
    tokio::task::spawn_blocking(read_process_performance_snapshot)
        .await
        .map_err(|e| format!("performance task failed: {}", e))
}

fn read_process_performance_snapshot() -> LocalPerformanceSnapshot {
    LocalPerformanceSnapshot {
        cpu_time_ms: read_process_cpu_time_ms(),
        memory_rss_mb: read_process_memory_rss_mb(),
        logical_cpu_count: std::thread::available_parallelism().ok().map(usize::from),
        timestamp: now_millis(),
    }
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(target_os = "macos")]
fn read_process_cpu_time_ms() -> Option<f64> {
    let output = Command::new("ps")
        .args(["-p", &std::process::id().to_string(), "-o", "time="])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    parse_ps_cpu_time_ms(String::from_utf8_lossy(&output.stdout).trim())
}

#[cfg(target_os = "linux")]
fn read_process_cpu_time_ms() -> Option<f64> {
    let contents = std::fs::read_to_string("/proc/self/stat").ok()?;
    let after_comm = contents.rsplit_once(") ")?.1;
    let fields = after_comm.split_whitespace().collect::<Vec<_>>();
    let utime = fields.get(11)?.parse::<u64>().ok()?;
    let stime = fields.get(12)?.parse::<u64>().ok()?;
    let clock_ticks = read_linux_clock_ticks().unwrap_or(100);
    Some(((utime + stime) as f64 / clock_ticks as f64) * 1000.0)
}

#[cfg(target_os = "windows")]
fn read_process_cpu_time_ms() -> Option<f64> {
    let mut command = Command::new("powershell");
    command
        .args([
            "-NoProfile",
            "-Command",
            &format!(
                "(Get-Process -Id {}).TotalProcessorTime.TotalMilliseconds",
                std::process::id()
            ),
        ])
        .creation_flags(CREATE_NO_WINDOW);

    let output = command.output().ok()?;
    if !output.status.success() {
        return None;
    }

    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<f64>()
        .ok()
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
fn read_process_cpu_time_ms() -> Option<f64> {
    None
}

#[cfg(target_os = "macos")]
fn read_process_memory_rss_mb() -> Option<u64> {
    let output = Command::new("ps")
        .args(["-p", &std::process::id().to_string(), "-o", "rss="])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<u64>()
        .ok()
        .map(|rss_kb| rss_kb / 1024)
}

#[cfg(target_os = "linux")]
fn read_process_memory_rss_mb() -> Option<u64> {
    let contents = std::fs::read_to_string("/proc/self/status").ok()?;
    contents.lines().find_map(|line| {
        let rest = line.strip_prefix("VmRSS:")?;
        rest.split_whitespace()
            .next()?
            .parse::<u64>()
            .ok()
            .map(|rss_kb| rss_kb / 1024)
    })
}

#[cfg(target_os = "windows")]
fn read_process_memory_rss_mb() -> Option<u64> {
    let mut command = Command::new("powershell");
    command
        .args([
            "-NoProfile",
            "-Command",
            &format!("(Get-Process -Id {}).WorkingSet64", std::process::id()),
        ])
        .creation_flags(CREATE_NO_WINDOW);

    let output = command.output().ok()?;
    if !output.status.success() {
        return None;
    }

    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<u64>()
        .ok()
        .map(|bytes| bytes / 1024 / 1024)
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
fn read_process_memory_rss_mb() -> Option<u64> {
    None
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn parse_ps_cpu_time_ms(value: &str) -> Option<f64> {
    let parts = value.split(':').collect::<Vec<_>>();
    match parts.as_slice() {
        [minutes, seconds] => {
            let minutes = minutes.trim().parse::<f64>().ok()?;
            let seconds = seconds.trim().parse::<f64>().ok()?;
            Some((minutes * 60.0 + seconds) * 1000.0)
        }
        [hours, minutes, seconds] => {
            let hours = hours.trim().parse::<f64>().ok()?;
            let minutes = minutes.trim().parse::<f64>().ok()?;
            let seconds = seconds.trim().parse::<f64>().ok()?;
            Some((hours * 3600.0 + minutes * 60.0 + seconds) * 1000.0)
        }
        _ => None,
    }
}

#[cfg(target_os = "linux")]
fn read_linux_clock_ticks() -> Option<u64> {
    let output = Command::new("getconf").arg("CLK_TCK").output().ok()?;
    if !output.status.success() {
        return None;
    }

    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<u64>()
        .ok()
}

// ---- derived metrics for the title-bar monitor (port of utils/localPerformance.ts) ----

const PROCESS_MEMORY_BAR_MAX_MB: f64 = 1024.0;

pub fn clamp_metric_percent(value: Option<f64>) -> f64 {
    match value {
        Some(v) if v.is_finite() => v.clamp(0.0, 100.0),
        _ => 0.0,
    }
}

/// Process CPU% from two snapshots (CPU-time delta / wall delta / logical cores); `None` if either is incomplete.
pub fn calculate_process_cpu_percent(
    previous: Option<&LocalPerformanceSnapshot>,
    current: &LocalPerformanceSnapshot,
) -> Option<f64> {
    let previous = previous?;
    let (prev_cpu, cur_cpu) = (previous.cpu_time_ms?, current.cpu_time_ms?);
    let cores = current.logical_cpu_count.filter(|c| *c > 0)?;
    let elapsed_ms = current.timestamp as f64 - previous.timestamp as f64;
    let cpu_delta_ms = cur_cpu - prev_cpu;
    if elapsed_ms <= 0.0 || cpu_delta_ms < 0.0 {
        return None;
    }
    Some(clamp_metric_percent(Some(cpu_delta_ms / elapsed_ms / cores as f64 * 100.0)))
}

pub fn format_process_memory(memory_rss_mb: Option<f64>) -> String {
    match memory_rss_mb {
        Some(mb) if mb.is_finite() && mb >= 0.0 => {
            if mb >= 1024.0 {
                format!("{:.1}G", mb / 1024.0)
            } else {
                format!("{}M", mb.round() as i64)
            }
        }
        _ => "--".to_string(),
    }
}

pub fn process_memory_bar_percent(memory_rss_mb: Option<f64>) -> f64 {
    match memory_rss_mb {
        Some(mb) if mb.is_finite() && mb >= 0.0 => clamp_metric_percent(Some(mb / PROCESS_MEMORY_BAR_MAX_MB * 100.0)),
        _ => 0.0,
    }
}

#[cfg(test)]
mod metric_tests {
    use super::*;

    fn snap(cpu: Option<f64>, ts: u64, cores: Option<usize>) -> LocalPerformanceSnapshot {
        LocalPerformanceSnapshot { cpu_time_ms: cpu, memory_rss_mb: None, logical_cpu_count: cores, timestamp: ts }
    }

    #[test]
    fn clamps_metric_percentages() {
        assert_eq!(clamp_metric_percent(Some(-8.0)), 0.0);
        assert_eq!(clamp_metric_percent(Some(48.4)), 48.4);
        assert_eq!(clamp_metric_percent(Some(180.0)), 100.0);
        assert_eq!(clamp_metric_percent(None), 0.0);
        assert_eq!(clamp_metric_percent(Some(f64::NAN)), 0.0);
    }

    #[test]
    fn cpu_percent_from_deltas() {
        let a = snap(Some(1200.0), 1000, Some(4));
        let b = snap(Some(2200.0), 2000, Some(4));
        assert_eq!(calculate_process_cpu_percent(Some(&a), &b), Some(25.0));
        assert_eq!(calculate_process_cpu_percent(Some(&a), &snap(None, 2000, Some(4))), None);
        assert_eq!(calculate_process_cpu_percent(None, &b), None);
        assert_eq!(calculate_process_cpu_percent(Some(&b), &a), None, "time went backwards");
        assert_eq!(calculate_process_cpu_percent(Some(&a), &snap(Some(2200.0), 2000, Some(0))), None);
    }

    #[test]
    fn memory_formatting_and_bar() {
        assert_eq!(format_process_memory(Some(384.0)), "384M");
        assert_eq!(format_process_memory(Some(1536.0)), "1.5G");
        assert_eq!(format_process_memory(None), "--");
        assert_eq!(process_memory_bar_percent(Some(128.0)), 12.5);
        assert_eq!(process_memory_bar_percent(Some(2048.0)), 100.0);
    }
}
