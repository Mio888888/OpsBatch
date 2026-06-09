use serde::Serialize;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Serialize)]
pub struct LocalPerformanceSnapshot {
    pub cpu_time_ms: Option<f64>,
    pub memory_rss_mb: Option<u64>,
    pub logical_cpu_count: Option<usize>,
    pub timestamp: u64,
}

#[tauri::command]
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
    let output = Command::new("powershell")
        .args([
            "-NoProfile",
            "-Command",
            &format!("(Get-Process -Id {}).TotalProcessorTime.TotalMilliseconds", std::process::id()),
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    String::from_utf8_lossy(&output.stdout).trim().parse::<f64>().ok()
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
        rest.split_whitespace().next()?.parse::<u64>().ok().map(|rss_kb| rss_kb / 1024)
    })
}

#[cfg(target_os = "windows")]
fn read_process_memory_rss_mb() -> Option<u64> {
    let output = Command::new("powershell")
        .args([
            "-NoProfile",
            "-Command",
            &format!("(Get-Process -Id {}).WorkingSet64", std::process::id()),
        ])
        .output()
        .ok()?;
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

    String::from_utf8_lossy(&output.stdout).trim().parse::<u64>().ok()
}
