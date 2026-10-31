//! What the external MCP clients did: the records shown in the activity window and the approvals waiting for the user.

use std::collections::HashMap;

use futures::channel::oneshot;
use gpui_kit::*;

pub const MAX_RECORDS: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Running,
    /// Waiting for the user to approve.
    Pending,
    Done,
    Failed,
    Denied,
}

/// The server (terminal session) a call works on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub session_id: String,
    pub name: String,
    pub ip: String,
    pub local: bool,
}

#[derive(Debug, Clone)]
pub struct Record {
    pub id: String,
    /// `HH:MM:SS`
    pub time: String,
    pub tool: String,
    pub summary: String,
    pub target: Option<Target>,
    pub status: Status,
    pub detail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Approve,
    /// Approve and remember a grant for this kind of command.
    AlwaysAllow,
    Reject,
}

#[derive(Debug, Clone)]
pub enum ActivityEvent {
    /// A call needs the user's decision.
    ApprovalNeeded(String),
}

#[derive(Default)]
pub struct McpActivity {
    records: Vec<Record>,
    approvals: HashMap<String, oneshot::Sender<Decision>>,
    target: Option<Target>,
}

impl EventEmitter<ActivityEvent> for McpActivity {}

impl McpActivity {
    pub fn records(&self) -> &[Record] {
        &self.records
    }

    /// Records waiting for a decision, oldest first.
    pub fn pending(&self) -> Vec<&Record> {
        self.records
            .iter()
            .filter(|r| r.status == Status::Pending)
            .collect()
    }

    /// The server the latest call worked on.
    pub fn target(&self) -> Option<&Target> {
        self.target.as_ref()
    }

    pub fn record(&self, id: &str) -> Option<&Record> {
        self.records.iter().find(|r| r.id == id)
    }

    /// Starts a record (running) and returns its id.
    pub fn begin(
        &mut self,
        tool: &str,
        summary: String,
        target: Option<Target>,
        cx: &mut Context<Self>,
    ) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        if target.is_some() {
            self.target = target.clone();
        }
        self.records.push(Record {
            id: id.clone(),
            time: chrono::Local::now().format("%H:%M:%S").to_string(),
            tool: tool.to_string(),
            summary,
            target,
            status: Status::Running,
            detail: String::new(),
        });
        if self.records.len() > MAX_RECORDS {
            let over = self.records.len() - MAX_RECORDS;
            // only finished records are dropped
            let mut dropped = 0;
            self.records.retain(|r| {
                if dropped < over && !matches!(r.status, Status::Running | Status::Pending) {
                    dropped += 1;
                    false
                } else {
                    true
                }
            });
        }
        cx.notify();
        id
    }

    /// Replaces the summary of a running record (once the call's ids are resolved to names).
    pub fn set_summary(&mut self, id: &str, summary: String, cx: &mut Context<Self>) {
        if let Some(r) = self.records.iter_mut().find(|r| r.id == id) {
            r.summary = summary;
            cx.notify();
        }
    }

    pub fn finish(&mut self, id: &str, status: Status, detail: String, cx: &mut Context<Self>) {
        if let Some(r) = self.records.iter_mut().find(|r| r.id == id) {
            r.status = status;
            r.detail = detail;
        }
        self.approvals.remove(id);
        cx.notify();
    }

    /// Marks the record as waiting and returns the receiver of the user's decision.
    pub fn request_approval(
        &mut self,
        id: &str,
        cx: &mut Context<Self>,
    ) -> oneshot::Receiver<Decision> {
        let (tx, rx) = oneshot::channel();
        if let Some(r) = self.records.iter_mut().find(|r| r.id == id) {
            r.status = Status::Pending;
        }
        self.approvals.insert(id.to_string(), tx);
        cx.emit(ActivityEvent::ApprovalNeeded(id.to_string()));
        cx.notify();
        rx
    }

    /// The user's answer; the waiting call continues (an unknown or already answered id does nothing).
    pub fn decide(&mut self, id: &str, decision: Decision, cx: &mut Context<Self>) -> bool {
        let Some(tx) = self.approvals.remove(id) else {
            return false;
        };
        if let Some(r) = self.records.iter_mut().find(|r| r.id == id) {
            r.status = Status::Running;
        }
        let _ = tx.send(decision);
        cx.notify();
        true
    }

    pub fn clear_finished(&mut self, cx: &mut Context<Self>) {
        self.records
            .retain(|r| matches!(r.status, Status::Running | Status::Pending));
        cx.notify();
    }
}
