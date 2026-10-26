//! Batch upload window (M5-11): send a file or folder to many hosts at once.

pub mod window;

use serde::Deserialize;

/// Per-host result as the backend reports it (`transfer:{task}:progress` and the final `done` list).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostResult {
    pub host_id: String,
    pub host_name: String,
    pub success: bool,
    #[serde(default)]
    pub file_size: i64,
    #[serde(default)]
    pub duration: i64,
    #[serde(default)]
    pub error: Option<String>,
}

/// Insert or replace results by host id, keeping first-seen order.
pub fn merge_results(current: &mut Vec<HostResult>, incoming: impl IntoIterator<Item = HostResult>) {
    for r in incoming {
        match current.iter_mut().find(|c| c.host_id == r.host_id) {
            Some(slot) => *slot = r,
            None => current.push(r),
        }
    }
}

/// (success, failed, pending, percent) given the number of target hosts.
pub fn summarize(results: &[HostResult], total: usize) -> (usize, usize, usize, f32) {
    let ok = results.iter().filter(|r| r.success).count();
    let failed = results.len() - ok;
    let done = ok + failed;
    let pending = total.saturating_sub(done);
    let percent = if total > 0 { done as f32 / total as f32 * 100.0 } else { 0.0 };
    (ok, failed, pending, percent)
}

pub fn format_size(bytes: i64) -> String {
    if bytes <= 0 {
        "-".into()
    } else if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / 1024.0 / 1024.0)
    }
}

/// Quick-fill buttons for the remote path: (label key or literal, value).
pub const QUICK_FILLS: &[(&str, &str)] = &[
    ("@hostName", "/home/{host}/"),
    ("@firstDir", "/home/{firstdir:/home}/"),
    ("/tmp/", "/tmp/"),
    ("/opt/", "/opt/"),
    ("/home/", "/home/"),
    ("/var/log/", "/var/log/"),
    ("/usr/local/bin/", "/usr/local/bin/"),
    ("/etc/", "/etc/"),
];

#[cfg(test)]
mod tests {
    use super::*;

    fn r(id: &str, ok: bool) -> HostResult {
        HostResult { host_id: id.into(), host_name: id.into(), success: ok, file_size: 10, duration: 5, error: (!ok).then(|| "boom".into()) }
    }

    #[test]
    fn results_merge_by_host_and_keep_order() {
        let mut v = Vec::new();
        merge_results(&mut v, [r("a", true), r("b", false)]);
        merge_results(&mut v, [r("b", true), r("c", true)]);
        assert_eq!(v.iter().map(|x| x.host_id.as_str()).collect::<Vec<_>>(), ["a", "b", "c"]);
        assert!(v[1].success, "later result replaces the earlier one");
    }

    #[test]
    fn summary_counts_and_percent() {
        let v = vec![r("a", true), r("b", false)];
        let (ok, failed, pending, pct) = summarize(&v, 4);
        assert_eq!((ok, failed, pending), (1, 1, 2));
        assert_eq!(pct, 50.0);
        assert_eq!(summarize(&[], 0).3, 0.0);
        assert_eq!(summarize(&v, 1).2, 0, "never negative");
    }

    #[test]
    fn payload_deserializes_camel_case() {
        let j = serde_json::json!({"hostId":"h","hostName":"web","success":true,"fileSize":2048,"duration":12,"error":null});
        let x: HostResult = serde_json::from_value(j).unwrap();
        assert_eq!((x.host_name.as_str(), x.file_size, x.duration), ("web", 2048, 12));
        assert_eq!(format_size(x.file_size), "2.0 KB");
        assert_eq!(format_size(0), "-");
        assert_eq!(format_size(500), "500 B");
    }
}
