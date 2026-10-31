//! Tracked command execution (the AI/command panels run a command in the live terminal and wait for its exit code).
//! Port of the `executeCommand` machinery in TerminalView.tsx: commands run one at a time, completion is detected
//! by a marker the shell prints after the command.

use std::collections::VecDeque;
use std::time::Duration;

use futures::channel::oneshot;
use ops_terminal::tracker::{
    create_tracked_command, done_pattern, strip_tracked_command_output_artifacts,
};
use regex::Regex;

pub const DEFAULT_COMMAND_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const MAX_TRACKED_OUTPUT_CHARS: usize = 120_000;
const MAX_DONE_DETECTOR_CHARS: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecStatus {
    Completed,
    Closed,
    Timeout,
    WriteFailed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionResult {
    pub execution_id: String,
    pub status: ExecStatus,
    pub exit_code: Option<i32>,
    pub output: String,
}

struct Queued {
    command: String,
    timeout: Duration,
    tx: oneshot::Sender<ExecutionResult>,
}

struct Active {
    id: String,
    pattern: Regex,
    output: String,
    tx: oneshot::Sender<ExecutionResult>,
}

/// A command that must now be written to the terminal (bracketed paste + Enter) and given a timeout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Started {
    pub execution_id: String,
    pub write: String,
    pub timeout: Duration,
}

#[derive(Default)]
pub struct ExecutionTracker {
    queue: VecDeque<Queued>,
    active: Option<Active>,
    detector: String,
}

fn tail_chars(s: &str, max: usize) -> String {
    let n = s.chars().count();
    if n <= max {
        s.to_string()
    } else {
        s.chars().skip(n - max).collect()
    }
}

impl ExecutionTracker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_busy(&self) -> bool {
        self.active.is_some()
    }

    pub fn pending_count(&self) -> usize {
        self.queue.len() + self.active.is_some() as usize
    }

    /// Queue a command. Returns what to write now when nothing else is running.
    pub fn submit(
        &mut self,
        command: String,
        timeout: Duration,
    ) -> (oneshot::Receiver<ExecutionResult>, Option<Started>) {
        let (tx, rx) = oneshot::channel();
        self.queue.push_back(Queued {
            command,
            timeout: timeout.max(Duration::from_secs(1)),
            tx,
        });
        let started = self.start_next();
        (rx, started)
    }

    fn start_next(&mut self) -> Option<Started> {
        if self.active.is_some() {
            return None;
        }
        let next = self.queue.pop_front()?;
        let id = uuid::Uuid::new_v4().simple().to_string();
        let prefix = format!("__OPSBATCH_AI_DONE_{id}__:");
        let tracked = create_tracked_command(&next.command, &prefix);
        let write = format!(
            "\x15\x1b[200~{}{}\x1b[201~\r",
            tracked.visible_command, tracked.hidden_tracker
        );
        self.detector.clear();
        self.active = Some(Active {
            pattern: done_pattern(&id),
            id: id.clone(),
            output: String::new(),
            tx: next.tx,
        });
        Some(Started {
            execution_id: id,
            write,
            timeout: next.timeout,
        })
    }

    fn settle(&mut self, status: ExecStatus, exit_code: Option<i32>) -> Option<Started> {
        let a = self.active.take()?;
        let output = strip_tracked_command_output_artifacts(&a.output)
            .trim()
            .to_string();
        let _ = a.tx.send(ExecutionResult {
            execution_id: a.id,
            status,
            exit_code,
            output,
        });
        self.start_next()
    }

    /// `raw` is the terminal payload as received (used to find the marker), `cleaned` what was displayed
    /// (collected as the command's output). Returns the next queued command to write, if one was started.
    pub fn on_output(&mut self, raw: &str, cleaned: &str) -> Option<Started> {
        let active = self.active.as_mut()?;
        if !cleaned.is_empty() {
            active.output = tail_chars(
                &format!("{}{}", active.output, cleaned),
                MAX_TRACKED_OUTPUT_CHARS,
            );
        }
        if raw.is_empty() {
            return None;
        }
        self.detector = tail_chars(
            &format!("{}{}", self.detector, raw),
            MAX_DONE_DETECTOR_CHARS,
        );
        let code = active
            .pattern
            .captures(&self.detector)
            .map(|c| c[1].parse::<i32>().ok());
        match code {
            Some(code) => self.settle(ExecStatus::Completed, code),
            None => None,
        }
    }

    pub fn timeout(&mut self, execution_id: &str) -> Option<Started> {
        if self.active.as_ref().is_some_and(|a| a.id == execution_id) {
            self.settle(ExecStatus::Timeout, None)
        } else {
            None
        }
    }

    pub fn write_failed(&mut self, execution_id: &str) -> Option<Started> {
        if self.active.as_ref().is_some_and(|a| a.id == execution_id) {
            self.settle(ExecStatus::WriteFailed, None)
        } else {
            None
        }
    }

    /// The session ended: everything running or waiting finishes as `Closed`.
    pub fn fail_all(&mut self) {
        while self.active.is_some() {
            self.settle(ExecStatus::Closed, None);
        }
        while let Some(q) = self.queue.pop_front() {
            let _ = q.tx.send(ExecutionResult {
                execution_id: String::new(),
                status: ExecStatus::Closed,
                exit_code: None,
                output: String::new(),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn done_line(id: &str, code: i32) -> String {
        format!("__OPSBATCH_AI_DONE_{id}__:{code}__")
    }

    fn recv(rx: &mut oneshot::Receiver<ExecutionResult>) -> ExecutionResult {
        rx.try_recv().unwrap().expect("result delivered")
    }

    #[test]
    fn completes_with_exit_code_and_clean_output() {
        let mut t = ExecutionTracker::new();
        let (mut rx, started) = t.submit("echo hi".into(), DEFAULT_COMMAND_TIMEOUT);
        let s = started.unwrap();
        assert!(s.write.starts_with("\x15\x1b[200~echo hi"));
        assert!(s.write.ends_with("\x1b[201~\r"));
        assert!(rx.try_recv().unwrap().is_none());
        t.on_output("hi\r\n", "hi\r\n");
        let raw = format!("{}\r\n", done_line(&s.execution_id, 3));
        t.on_output(&raw, "");
        let r = recv(&mut rx);
        assert_eq!(r.status, ExecStatus::Completed);
        assert_eq!(r.exit_code, Some(3));
        assert_eq!(r.output, "hi");
        assert!(!t.is_busy());
    }

    #[test]
    fn marker_split_across_chunks_is_found() {
        let mut t = ExecutionTracker::new();
        let (mut rx, s) = t.submit("true".into(), DEFAULT_COMMAND_TIMEOUT);
        let line = done_line(&s.unwrap().execution_id, 0);
        let (a, b) = line.split_at(line.len() / 2);
        t.on_output(a, "");
        assert!(rx.try_recv().unwrap().is_none());
        t.on_output(b, "");
        assert_eq!(recv(&mut rx).exit_code, Some(0));
    }

    #[test]
    fn commands_run_strictly_one_after_another() {
        let mut t = ExecutionTracker::new();
        let (mut rx1, s1) = t.submit("one".into(), DEFAULT_COMMAND_TIMEOUT);
        let (mut rx2, s2) = t.submit("two".into(), DEFAULT_COMMAND_TIMEOUT);
        assert!(s2.is_none(), "second waits for the first");
        let s1 = s1.unwrap();
        let next = t
            .on_output(&done_line(&s1.execution_id, 0), "")
            .expect("second starts when first completes");
        assert!(next.write.contains("two"));
        assert_eq!(recv(&mut rx1).status, ExecStatus::Completed);
        assert!(rx2.try_recv().unwrap().is_none());
        t.on_output(&done_line(&next.execution_id, 7), "");
        assert_eq!(recv(&mut rx2).exit_code, Some(7));
    }

    #[test]
    fn a_stale_marker_from_the_previous_command_does_not_complete_the_next() {
        let mut t = ExecutionTracker::new();
        let (_rx1, s1) = t.submit("one".into(), DEFAULT_COMMAND_TIMEOUT);
        let (mut rx2, _) = t.submit("two".into(), DEFAULT_COMMAND_TIMEOUT);
        let s1 = s1.unwrap();
        t.on_output(&done_line(&s1.execution_id, 0), "");
        // Marker 1 is still in the detector window when command 2 starts; it must have been cleared.
        t.on_output("more output\r\n", "more output\r\n");
        assert!(rx2.try_recv().unwrap().is_none());
    }

    #[test]
    fn timeout_and_write_failure_settle_with_their_status() {
        let mut t = ExecutionTracker::new();
        let (mut rx, s) = t.submit("sleep".into(), DEFAULT_COMMAND_TIMEOUT);
        let id = s.unwrap().execution_id;
        assert!(t.timeout("other-id").is_none());
        assert!(t.is_busy());
        t.timeout(&id);
        assert_eq!(recv(&mut rx).status, ExecStatus::Timeout);

        let (mut rx, s) = t.submit("x".into(), DEFAULT_COMMAND_TIMEOUT);
        t.write_failed(&s.unwrap().execution_id);
        assert_eq!(recv(&mut rx).status, ExecStatus::WriteFailed);
    }

    #[test]
    fn closing_the_session_fails_running_and_queued_commands() {
        let mut t = ExecutionTracker::new();
        let (mut a, _) = t.submit("a".into(), DEFAULT_COMMAND_TIMEOUT);
        let (mut b, _) = t.submit("b".into(), DEFAULT_COMMAND_TIMEOUT);
        t.on_output("partial", "partial");
        t.fail_all();
        let ra = recv(&mut a);
        assert_eq!(
            (ra.status, ra.output.as_str()),
            (ExecStatus::Closed, "partial")
        );
        assert_eq!(recv(&mut b).status, ExecStatus::Closed);
        assert_eq!(t.pending_count(), 0);
    }

    #[test]
    fn output_is_bounded() {
        let mut t = ExecutionTracker::new();
        let (mut rx, s) = t.submit("big".into(), DEFAULT_COMMAND_TIMEOUT);
        let chunk = "x".repeat(50_000);
        for _ in 0..5 {
            t.on_output(&chunk, &chunk);
        }
        t.on_output(&done_line(&s.unwrap().execution_id, 0), "");
        assert!(recv(&mut rx).output.chars().count() <= 120_000);
    }
}
