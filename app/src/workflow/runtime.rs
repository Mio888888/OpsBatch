//! The workflow executor's window onto the app: commands and transfers go through the same services as the batch
//! window, confirmations are handed to the UI, sleeping uses the executor's timers.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use futures::channel::oneshot;
use futures::FutureExt as _;
use gpui_kit::BackgroundExecutor;
use ops_ssh::Ctx;
use ops_workflow::executor::{CommandOutput, CommandSpec, WorkflowRuntime};

use crate::batch_exec::OutputEvent;
use crate::library::exec;
use crate::sftp::store::blocking;

pub struct ConfirmRequest {
    pub node_name: String,
    pub description: String,
    pub reply: oneshot::Sender<bool>,
}

pub struct AppRuntime {
    ctx: Arc<Ctx>,
    executor: BackgroundExecutor,
    confirm_tx: flume::Sender<ConfirmRequest>,
    cancelled: Arc<AtomicBool>,
}

impl AppRuntime {
    pub fn new(
        ctx: Arc<Ctx>,
        executor: BackgroundExecutor,
        confirm_tx: flume::Sender<ConfirmRequest>,
        cancelled: Arc<AtomicBool>,
    ) -> Self {
        Self {
            ctx,
            executor,
            confirm_tx,
            cancelled,
        }
    }
}

/// Fold the per-host result events into the node output (`[host] text` lines; any failed/timeout host fails the node).
pub fn fold_outputs(events: impl IntoIterator<Item = OutputEvent>) -> CommandOutput {
    let mut output = String::new();
    let mut success = true;
    let mut exit_code = 0;
    for ev in events {
        if !ev.output.is_empty() {
            if !output.is_empty() {
                output.push('\n');
            }
            let who = if ev.host_name.is_empty() {
                &ev.host_id
            } else {
                &ev.host_name
            };
            output.push_str(&format!("[{who}] {}", ev.output));
        }
        if ev.status == "failed" || ev.status == "timeout" {
            success = false;
            exit_code = match ev.exit_code {
                Some(c) if c != 0 => c as i32,
                _ => 1,
            };
        }
    }
    CommandOutput {
        output,
        success,
        exit_code,
    }
}

impl WorkflowRuntime for AppRuntime {
    async fn run_command(
        &self,
        host_ids: &[String],
        command: &str,
        spec: CommandSpec,
    ) -> Result<CommandOutput, String> {
        let task_id = uuid::Uuid::new_v4().to_string();
        // Subscribe first so nothing emitted while the task starts is lost.
        let outputs = self.ctx.events.subscribe(format!("exec:{task_id}:output"));
        let done = self.ctx.events.subscribe(format!("exec:{task_id}:done"));
        exec::dispatch(
            self.ctx.clone(),
            task_id,
            host_ids.to_vec(),
            command.to_string(),
            10,
            spec.ssh_timeout_secs,
            None,
        )
        .await?;

        // Wait for the done event, but never longer than the cap (a stuck host must not hang the workflow).
        let mut done_fut = done.recv_async().fuse();
        let mut cap = self
            .executor
            .timer(Duration::from_secs(spec.wait_cap_secs))
            .fuse();
        futures::select! {
            _ = done_fut => {}
            _ = cap => {}
        }
        Ok(fold_outputs(
            outputs
                .try_iter()
                .filter_map(|ev| ev.parse::<OutputEvent>()),
        ))
    }

    async fn transfer(
        &self,
        host_ids: &[String],
        direction: &str,
        local_path: &str,
        remote_path: &str,
        timeout_secs: u64,
    ) -> Result<(), String> {
        let ctx = self.ctx.clone();
        let request = ops_ssh::transfer::TransferRequest {
            task_id: None,
            direction: direction.to_string(),
            host_ids: host_ids.to_vec(),
            local_path: local_path.to_string(),
            remote_path: remote_path.to_string(),
            remote_paths: None,
            timeout: Some(timeout_secs),
        };
        blocking(move || {
            ops_ssh::transfer::file_transfer(ctx.clone(), &ctx.db, request).map(|_| ())
        })
        .await
    }

    async fn confirm(&self, node_name: &str, description: &str) -> bool {
        let (reply, answer) = oneshot::channel();
        if self
            .confirm_tx
            .send(ConfirmRequest {
                node_name: node_name.into(),
                description: description.into(),
                reply,
            })
            .is_err()
        {
            return false;
        }
        answer.await.unwrap_or(false)
    }

    async fn sleep(&self, duration: Duration) {
        self.executor.timer(duration).await;
    }

    fn cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(host: &str, name: &str, status: &str, output: &str, code: Option<i64>) -> OutputEvent {
        OutputEvent {
            host_id: host.into(),
            host_name: name.into(),
            status: status.into(),
            output: output.into(),
            exit_code: code,
            duration: None,
        }
    }

    #[test]
    fn outputs_are_labelled_per_host_and_failures_win() {
        let ok = fold_outputs([
            ev("h1", "web", "success", "up", Some(0)),
            ev("h2", "", "success", "also up", Some(0)),
        ]);
        assert_eq!(
            ok,
            CommandOutput {
                output: "[web] up\n[h2] also up".into(),
                success: true,
                exit_code: 0
            }
        );
        let bad = fold_outputs([
            ev("h1", "web", "success", "up", Some(0)),
            ev("h2", "db", "failed", "no", Some(5)),
            ev("h3", "x", "timeout", "", None),
        ]);
        assert_eq!(
            (bad.success, bad.exit_code),
            (false, 1),
            "the last failing host decides the exit code; 0/None become 1"
        );
        assert!(bad.output.contains("[db] no"));
        assert_eq!(
            fold_outputs([]),
            CommandOutput {
                output: String::new(),
                success: true,
                exit_code: 0
            }
        );
    }
}
