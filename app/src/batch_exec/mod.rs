//! Batch command execution window (M6-09) and recording replay (M6-10).

pub mod player;
pub mod window;

use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Pending,
    Running,
    Success,
    Failed,
    Timeout,
}

impl Status {
    pub fn parse(s: &str) -> Self {
        match s {
            "running" => Status::Running,
            "success" => Status::Success,
            "failed" => Status::Failed,
            "timeout" => Status::Timeout,
            _ => Status::Pending,
        }
    }
    pub fn label_key(self) -> &'static str {
        match self {
            Status::Pending => "commands.pending",
            Status::Running => "commands.running",
            Status::Success => "commands.success",
            Status::Failed => "commands.failed",
            Status::Timeout => "commands.timeoutStatus",
        }
    }
    pub fn is_failure(self) -> bool {
        matches!(self, Status::Failed | Status::Timeout)
    }
    pub fn is_active(self) -> bool {
        matches!(self, Status::Pending | Status::Running)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct HostRun {
    pub host_id: String,
    pub host_name: String,
    pub host_ip: String,
    pub status: Status,
    pub output: String,
    pub exit_code: Option<i64>,
    pub duration: Option<i64>,
}

impl HostRun {
    pub fn pending(host_id: &str, name: &str, ip: &str) -> Self {
        Self {
            host_id: host_id.into(),
            host_name: name.into(),
            host_ip: ip.into(),
            status: Status::Pending,
            output: String::new(),
            exit_code: None,
            duration: None,
        }
    }
    pub fn display_name(&self) -> &str {
        if !self.host_name.is_empty() {
            &self.host_name
        } else if !self.host_ip.is_empty() {
            &self.host_ip
        } else {
            &self.host_id
        }
    }
}

/// `exec:{task}:output` payload.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputEvent {
    pub host_id: String,
    #[serde(default)]
    pub host_name: String,
    pub status: String,
    #[serde(default)]
    pub output: String,
    #[serde(default)]
    pub exit_code: Option<i64>,
    #[serde(default)]
    pub duration: Option<i64>,
}

/// `exec:{task}:done` payload.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DoneEvent {
    pub success_count: u32,
    pub fail_count: u32,
    pub duration: i64,
}

pub fn apply_start(runs: &mut Vec<HostRun>, host_id: &str) {
    match runs.iter_mut().find(|r| r.host_id == host_id) {
        Some(r) => r.status = Status::Running,
        None => runs.push(HostRun {
            status: Status::Running,
            ..HostRun::pending(host_id, "", "")
        }),
    }
}

pub fn apply_output(runs: &mut Vec<HostRun>, e: &OutputEvent) {
    let status = Status::parse(&e.status);
    match runs.iter_mut().find(|r| r.host_id == e.host_id) {
        Some(r) => {
            if !e.host_name.is_empty() {
                r.host_name = e.host_name.clone();
            }
            r.status = status;
            r.output = e.output.clone();
            r.exit_code = e.exit_code;
            r.duration = e.duration;
        }
        None => runs.push(HostRun {
            host_id: e.host_id.clone(),
            host_name: e.host_name.clone(),
            host_ip: String::new(),
            status,
            output: e.output.clone(),
            exit_code: e.exit_code,
            duration: e.duration,
        }),
    }
}

/// (total, success, failed, running) — failed includes timeouts, running includes pending.
pub fn stats(runs: &[HostRun]) -> (usize, usize, usize, usize) {
    (
        runs.len(),
        runs.iter().filter(|r| r.status == Status::Success).count(),
        runs.iter().filter(|r| r.status.is_failure()).count(),
        runs.iter().filter(|r| r.status.is_active()).count(),
    )
}

/// After "stop": everything still waiting counts as failed with a cancellation note.
pub fn mark_cancelled(runs: &mut [HostRun], note: &str) {
    for r in runs.iter_mut().filter(|r| r.status.is_active()) {
        r.status = Status::Failed;
        r.output = format!("{}\n{}", r.output, note);
    }
}

pub fn reset_for_retry(runs: &mut [HostRun], host_ids: &[String]) {
    for r in runs.iter_mut().filter(|r| host_ids.contains(&r.host_id)) {
        r.status = Status::Pending;
        r.output.clear();
        r.exit_code = None;
        r.duration = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(id: &str, name: &str, status: &str, out: &str) -> OutputEvent {
        OutputEvent {
            host_id: id.into(),
            host_name: name.into(),
            status: status.into(),
            output: out.into(),
            exit_code: Some(0),
            duration: Some(12),
        }
    }

    #[test]
    fn events_update_known_hosts_and_add_unknown_ones() {
        let mut runs = vec![HostRun::pending("a", "web", "10.0.0.1")];
        apply_start(&mut runs, "a");
        assert_eq!(runs[0].status, Status::Running);
        apply_output(&mut runs, &ev("a", "", "success", "hi"));
        assert_eq!(
            (
                runs[0].host_name.as_str(),
                runs[0].host_ip.as_str(),
                runs[0].status
            ),
            ("web", "10.0.0.1", Status::Success),
            "name/ip survive an empty event name"
        );
        apply_output(&mut runs, &ev("b", "db", "failed", "boom"));
        apply_start(&mut runs, "c");
        assert_eq!(runs.len(), 3);
        assert_eq!(stats(&runs), (3, 1, 1, 1));
    }

    #[test]
    fn timeouts_count_as_failures_and_cancel_marks_the_rest() {
        let mut runs = vec![
            HostRun::pending("a", "", ""),
            HostRun::pending("b", "", ""),
            HostRun::pending("c", "", ""),
        ];
        apply_output(&mut runs, &ev("a", "a", "timeout", ""));
        apply_start(&mut runs, "b");
        assert_eq!(stats(&runs), (3, 0, 1, 2));
        mark_cancelled(&mut runs, "[cancelled]");
        assert_eq!(stats(&runs), (3, 0, 3, 0));
        assert!(runs[2].output.ends_with("[cancelled]"));
    }

    #[test]
    fn retry_resets_only_the_chosen_hosts() {
        let mut runs = vec![HostRun::pending("a", "", ""), HostRun::pending("b", "", "")];
        apply_output(&mut runs, &ev("a", "a", "failed", "x"));
        apply_output(&mut runs, &ev("b", "b", "success", "y"));
        reset_for_retry(&mut runs, &["a".to_string()]);
        assert_eq!(
            (runs[0].status, runs[0].output.as_str(), runs[0].duration),
            (Status::Pending, "", None)
        );
        assert_eq!(runs[1].status, Status::Success);
    }

    #[test]
    fn done_payload_parses() {
        let d: DoneEvent = serde_json::from_value(
            serde_json::json!({"successCount":2,"failCount":1,"duration":340}),
        )
        .unwrap();
        assert_eq!((d.success_count, d.fail_count, d.duration), (2, 1, 340));
        assert_eq!(runs_name(&HostRun::pending("id", "", "")), "id");
    }

    fn runs_name(r: &HostRun) -> String {
        r.display_name().to_string()
    }
}
