//! Dispatching a command to hosts (shared by quick actions, the batch window and the inline Docker panel).

use std::sync::Arc;

use ops_ssh::Ctx;

use crate::sftp::store::blocking;

/// Start `command` on `host_ids`. Results arrive as `exec:{task}:*` events; resolves once the task is dispatched.
pub async fn dispatch(ctx: Arc<Ctx>, task_id: String, host_ids: Vec<String>, command: String, concurrency: u32, timeout: u64, quick_action_id: Option<String>) -> Result<String, String> {
    blocking(move || ops_ssh::execution::execute_command(ctx.clone(), &ctx.db, task_id, host_ids, command, concurrency, timeout, quick_action_id)).await
}
