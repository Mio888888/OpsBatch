//! The SFTP, port forward and host-list tools of the external MCP bridge. Reads answer at once; everything that
//! changes something goes through [`authorize`]: observer mode refuses, auto mode runs, a matching permission grant
//! runs, anything else waits for the user's decision in the activity window.

use std::collections::BTreeMap;

use gpui_kit::*;
use ops_ai::mcp_bridge::ToolOutcome;
use ops_ai::prefs::external_mcp::ApprovalAlert;
use ops_ai::prefs::safety::{match_grant, new_grant, MatchContext};
use ops_ai::prefs::PermissionMode;
use ops_i18n::t;
use ops_ssh::forward::{ForwardConfig, ForwardType};
use serde_json::{json, Value};

use super::activity::{Decision, Status};
use super::{activity, tail, vault, window, ConnState, Session};
use crate::ai::prefs_store;
use crate::sftp::store::blocking;

const DEFAULT_READ_BYTES: u64 = 256 * 1024;
const MAX_READ_BYTES: u64 = 1024 * 1024;

pub(super) fn handles(tool: &str) -> bool {
    matches!(
        tool,
        "sftp_list"
            | "sftp_stat"
            | "sftp_read"
            | "sftp_write"
            | "sftp_mkdir"
            | "sftp_rename"
            | "sftp_delete"
            | "sftp_upload"
            | "sftp_download"
            | "sftp_chmod"
            | "portforward_list"
            | "portforward_start"
            | "portforward_stop"
            | "portforward_delete"
            | "portforward_update"
            | "portforward_duplicate"
    ) || vault::handles(tool)
}

/// The grantable capability a changing tool needs (`None` for the read-only ones).
fn capability_of(tool: &str) -> Option<&'static str> {
    Some(match tool {
        "sftp_write" => "sftp.write",
        "sftp_mkdir" => "sftp.mkdir",
        "sftp_rename" => "sftp.rename",
        "sftp_delete" => "sftp.delete",
        "portforward_start" => "portforward.start",
        "portforward_stop" => "portforward.stop",
        "sftp_upload" => "sftp.upload",
        "sftp_download" => "sftp.download",
        "sftp_chmod" => "sftp.chmod",
        "portforward_delete" => "portforward.rules.delete",
        "portforward_update" => "portforward.rules.update",
        "portforward_duplicate" => "portforward.rules.duplicate",
        other => return vault::capability_of(other),
    })
}

fn arg(args: &Value, key: &str) -> String {
    args.get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

/// What the record shows for the call (never the file content).
fn summary_of(tool: &str, args: &Value) -> String {
    match tool {
        "sftp_rename" => format!("{} → {}", arg(args, "from"), arg(args, "to")),
        "sftp_write" => format!(
            "{} ({} bytes)",
            arg(args, "path"),
            arg(args, "content").len()
        ),
        "portforward_start" => format!(
            "{} {}{}",
            arg(args, "type"),
            arg(args, "local_address"),
            if arg(args, "remote_address").is_empty() {
                String::new()
            } else {
                format!(" ↔ {}", arg(args, "remote_address"))
            }
        ),
        "portforward_stop"
        | "portforward_delete"
        | "portforward_update"
        | "portforward_duplicate" => arg(args, "forward_id"),
        "sftp_upload" => format!("{} → {}", arg(args, "local_path"), arg(args, "remote_path")),
        "sftp_download" => format!("{} → {}", arg(args, "remote_path"), arg(args, "local_path")),
        "sftp_chmod" => format!("{} {}", arg(args, "mode"), arg(args, "path")),
        other if vault::handles(other) => vault::summary_of(other, args),
        "portforward_list" => String::new(),
        _ => arg(args, "path"),
    }
}

/// An SSH session that has a saved host behind it (SFTP and forwards need the host's connection settings).
#[derive(Clone)]
struct SshTarget {
    session: Session,
    host_id: String,
}

fn resolve_ssh(session_id: Option<&str>, cx: &App) -> Result<SshTarget, String> {
    let all = super::sessions(cx);
    let pick = match session_id.filter(|s| !s.is_empty()) {
        Some(id) => all.into_iter().find(|s| s.id == id).ok_or_else(|| {
            format!("Unknown session_id: {id}. Use list_sessions to see the open sessions.")
        })?,
        None => {
            let ssh =
                |s: &&Session| !s.local && s.host_id.is_some() && s.state == ConnState::Connected;
            all.iter()
                .find(|s| s.active && ssh(s))
                .or_else(|| all.iter().find(ssh))
                .cloned()
                .ok_or_else(|| "No connected SSH session is open in OpsBatch.".to_string())?
        }
    };
    if pick.local {
        return Err("SFTP and port forwards need an SSH session; this is a local terminal.".into());
    }
    if pick.state != ConnState::Connected {
        return Err("The session is not connected.".into());
    }
    let host_id = pick
        .host_id
        .clone()
        .ok_or("The session has no saved host.")?;
    Ok(SshTarget {
        session: pick,
        host_id,
    })
}

pub(super) fn dispatch(
    tool: String,
    args: Value,
    reply: tokio::sync::oneshot::Sender<ToolOutcome>,
    cx: &mut App,
) {
    let activity = activity(cx);
    let summary = summary_of(&tool, &args);
    // which server the call works on (portforward_stop / list / vault calls name none)
    let wants_session = tool.starts_with("sftp_") || tool == "portforward_start";
    let target = if wants_session {
        match resolve_ssh(args.get("session_id").and_then(Value::as_str), cx) {
            Ok(t) => Some(t),
            Err(e) => {
                let id = activity.update(cx, |a, cx| a.begin(&tool, summary, None, cx));
                activity.update(cx, |a, cx| a.finish(&id, Status::Failed, e.clone(), cx));
                let _ = reply.send(ToolOutcome::error(e));
                return;
            }
        }
    } else {
        None
    };
    let id = activity.update(cx, |a, cx| {
        a.begin(
            &tool,
            summary,
            target.as_ref().map(|t| t.session.target()),
            cx,
        )
    });
    cx.spawn(async move |cx| {
        let outcome = run(&id, &tool, &args, target, cx).await;
        let status = if !outcome.is_error {
            Status::Done
        } else if outcome.text.starts_with("Refused") || outcome.text.starts_with("Rejected") {
            Status::Denied
        } else {
            Status::Failed
        };
        let detail = detail_of(&tool, &outcome);
        cx.update(|cx| activity.update(cx, |a, cx| a.finish(&id, status, detail, cx)));
        let _ = reply.send(outcome);
    })
    .detach();
}

/// The line the record shows under the call.
fn detail_of(tool: &str, outcome: &ToolOutcome) -> String {
    if outcome.is_error {
        return tail(outcome.text.lines().next().unwrap_or(""), 300);
    }
    let value: Option<Value> = serde_json::from_str(&outcome.text).ok();
    let count = value.as_ref().and_then(Value::as_array).map(Vec::len);
    match (tool, count) {
        ("sftp_list", Some(n)) => t!("mcpwin.detail.entries", count = n),
        ("vault_list_hosts", Some(n)) => t!("mcpwin.detail.hosts", count = n),
        ("vault_list_scripts", Some(n)) => t!("mcpwin.detail.scripts", count = n),
        ("vault_list_groups", Some(n)) => t!("mcpwin.detail.groups", count = n),
        ("portforward_list", Some(n)) => t!("mcpwin.detail.forwards", count = n),
        ("sftp_read", _) => t!("mcpwin.detail.bytes", count = outcome.text.len()),
        ("vault_run_script" | "vault_get_script_run", _) => match value.as_ref().and_then(|v| {
            v.get("status").and_then(Value::as_str).map(|s| {
                (
                    s.to_string(),
                    v["success"].as_i64().unwrap_or(0),
                    v["failed"].as_i64().unwrap_or(0),
                    v["hosts_total"].as_i64().unwrap_or(0),
                )
            })
        }) {
            Some((status, ok, failed, total)) => t!(
                "mcpwin.detail.run",
                status = status,
                ok = ok,
                failed = failed,
                total = total
            ),
            None => tail(outcome.text.lines().next().unwrap_or(""), 300),
        },
        _ => tail(outcome.text.lines().next().unwrap_or(""), 300),
    }
}

async fn run(
    id: &str,
    tool: &str,
    args: &Value,
    target: Option<SshTarget>,
    cx: &mut AsyncApp,
) -> ToolOutcome {
    if let Some(capability) = capability_of(tool) {
        let mut policy_args = BTreeMap::new();
        for key in [
            "path",
            "from",
            "to",
            "type",
            "local_address",
            "remote_address",
            "forward_id",
            "local_path",
            "remote_path",
            "mode",
            "host_id",
            "group_id",
            "script_id",
            "name",
        ] {
            let v = arg(args, key);
            if !v.is_empty() {
                policy_args.insert(key.to_string(), v);
            }
        }
        let named = cx.update(|cx| crate::services::ctx(cx));
        // a call that cannot run is answered before anyone is asked to approve it
        if let Err(e) = validate(tool, args, &named, cx).await {
            return ToolOutcome::error(e);
        }
        if let Some(text) = describe(tool, args, &named, cx).await {
            let activity = cx.update(|cx| activity(cx));
            let id = id.to_string();
            cx.update(|cx| activity.update(cx, |a, cx| a.set_summary(&id, text, cx)));
        }
        if let Err(refusal) = authorize(id, capability, target.as_ref(), policy_args, cx).await {
            return refusal;
        }
    }
    let ctx = cx.update(|cx| crate::services::ctx(cx));
    match tool {
        t if vault::handles(t) => vault::run(t, args, &ctx, cx, false).await,
        "portforward_list" => list_forwards(cx).await,
        "portforward_start" => match target {
            Some(t) => start_forward(&ctx, &t, args).await,
            None => ToolOutcome::error("No SSH session."),
        },
        "portforward_stop" | "portforward_delete" => {
            end_forward(
                &ctx,
                &arg(args, "forward_id"),
                tool == "portforward_delete",
                cx,
            )
            .await
        }
        "portforward_update" | "portforward_duplicate" => {
            change_forward(&ctx, args, tool == "portforward_duplicate", cx).await
        }
        _ => match target {
            Some(t) => sftp(&ctx, tool, &t.host_id, args).await,
            None => ToolOutcome::error("No SSH session."),
        },
    }
}

/// Everything about a changing call that can be checked without changing anything.
async fn validate(
    tool: &str,
    args: &Value,
    ctx: &std::sync::Arc<ops_ssh::Ctx>,
    cx: &mut AsyncApp,
) -> Result<(), String> {
    match tool {
        t if vault::handles(t) => {
            let checked = vault::run(t, args, ctx, cx, true).await;
            if checked.is_error {
                Err(checked.text)
            } else {
                Ok(())
            }
        }
        "portforward_start" => forward_config(args, None).map(|_| ()),
        "portforward_stop"
        | "portforward_delete"
        | "portforward_update"
        | "portforward_duplicate" => {
            let id = arg(args, "forward_id");
            let Some((_, _, forward)) = find_forward(ctx, &id, cx).await else {
                return Err(unknown_forward(&id).text);
            };
            if tool.ends_with("update") || tool.ends_with("duplicate") {
                forward_config(args, Some(&forward.config))?;
            }
            Ok(())
        }
        t => validate_sftp(t, args),
    }
}

/// Permission check of a changing tool. `Err` carries the answer to send back.
async fn authorize(
    id: &str,
    capability: &str,
    target: Option<&SshTarget>,
    args: BTreeMap<String, String>,
    cx: &mut AsyncApp,
) -> Result<(), ToolOutcome> {
    let limits = cx.update(|cx| crate::ai::runtime::limits(cx));
    match limits.permission_mode {
        PermissionMode::Observer => return Err(ToolOutcome::error("Refused by OpsBatch: observer mode is read-only. The mode can be changed in Settings → AI → Safety.")),
        PermissionMode::Auto => return Ok(()),
        PermissionMode::Confirm => {}
    }
    let ctx = MatchContext {
        capability_id: capability.to_string(),
        session_id: target.map(|t| t.session.id.clone()),
        host_id: target.map(|t| t.host_id.clone()),
        args,
    };
    if match_grant(&limits.permission_grants, &ctx).is_some() {
        return Ok(());
    }
    let activity = cx.update(|cx| activity(cx));
    let id_for_request = id.to_string();
    let rx = cx.update(|cx| {
        let rx = activity.update(cx, |a, cx| a.request_approval(&id_for_request, cx));
        let alert = prefs_store::shared(cx)
            .read(cx)
            .prefs()
            .external_mcp
            .approval_alert;
        let focused = cx.active_window().is_some();
        window::show(
            alert == ApprovalAlert::Always || (alert == ApprovalAlert::Unfocused && !focused),
            cx,
        );
        rx
    });
    match rx.await {
        Ok(Decision::Approve) => Ok(()),
        Ok(Decision::AlwaysAllow) => {
            let capability = capability.to_string();
            cx.update(|cx| {
                prefs_store::shared(cx).update(cx, |s, cx| {
                    s.update(cx, |p| {
                        let mut rule = new_grant();
                        rule.capability_id = capability;
                        rule.note = Some("MCP".into());
                        p.permission_grants.push(rule);
                    })
                })
            });
            Ok(())
        }
        Ok(Decision::Reject) | Err(_) => Err(ToolOutcome::error(
            "Rejected: the user did not approve this operation in OpsBatch.",
        )),
    }
}

// ---------------------------------------------------------------------------
// SFTP
// ---------------------------------------------------------------------------

/// The checks of an SFTP call that need no connection: required arguments and the local side of a transfer.
fn validate_sftp(tool: &str, args: &Value) -> Result<(), String> {
    let need = |key: &str| {
        if arg(args, key).trim().is_empty() {
            Err(format!("`{key}` is required."))
        } else {
            Ok(())
        }
    };
    match tool {
        "sftp_rename" => {
            need("from")?;
            need("to")
        }
        "sftp_write" => {
            need("path")?;
            args.get("content")
                .and_then(Value::as_str)
                .map(|_| ())
                .ok_or_else(|| "`content` is required.".to_string())
        }
        "sftp_chmod" => {
            need("path")?;
            parse_mode(&arg(args, "mode")).map(|_| ())
        }
        "sftp_upload" => {
            need("remote_path")?;
            let local = arg(args, "local_path");
            if !std::path::Path::new(&local).is_absolute() {
                return Err("`local_path` must be an absolute path.".into());
            }
            match std::fs::metadata(&local) {
                Ok(m) if m.is_file() => Ok(()),
                Ok(_) => {
                    Err("`local_path` is not a regular file (folders cannot be uploaded).".into())
                }
                Err(e) => Err(format!("Cannot read `local_path`: {e}")),
            }
        }
        "sftp_download" => {
            need("remote_path")?;
            let local = arg(args, "local_path");
            let target = std::path::Path::new(&local);
            if !target.is_absolute() {
                return Err("`local_path` must be an absolute path.".into());
            }
            if target.exists()
                && !args
                    .get("overwrite")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
            {
                return Err(
                    "`local_path` already exists; pass overwrite=true to replace it.".into(),
                );
            }
            if target.is_dir() || !target.parent().is_some_and(|p| p.is_dir()) {
                return Err("`local_path` must be a file path inside an existing folder.".into());
            }
            Ok(())
        }
        // list, stat, read, mkdir, delete
        _ => need("path"),
    }
}

async fn sftp(
    ctx: &std::sync::Arc<ops_ssh::Ctx>,
    tool: &str,
    host_id: &str,
    args: &Value,
) -> ToolOutcome {
    if let Err(e) = validate_sftp(tool, args) {
        return ToolOutcome::error(e);
    }
    // opens the SFTP channel of the session's host (reuses the one the SFTP panel may already hold)
    {
        let (c, h) = (ctx.clone(), host_id.to_string());
        if let Err(e) = ops_core::rt::run(async move { ops_ssh::sftp::sftp_open(c, h).await })
            .await
            .and_then(|r| r)
        {
            return ToolOutcome::error(format!("Could not open SFTP: {e}"));
        }
    }
    let host = host_id.to_string();
    let path = arg(args, "path");
    let c = ctx.clone();
    let result: Result<ToolOutcome, String> = match tool {
        "sftp_list" => {
            let (c2, h, p) = (c.clone(), host, path);
            ops_core::rt::run(async move { ops_ssh::sftp::sftp_list_dir(c2, h, p).await })
                .await
                .and_then(|r| r)
                .map(|entries| {
                    let rows: Vec<Value> = entries.iter().map(entry_json).collect();
                    ToolOutcome::ok(serde_json::to_string_pretty(&rows).unwrap_or_default())
                })
        }
        "sftp_stat" => blocking(move || ops_ssh::sftp::sftp_stat(&c.sftp, host, path))
            .await
            .map(|e| {
                ToolOutcome::ok(serde_json::to_string_pretty(&entry_json(&e)).unwrap_or_default())
            }),
        "sftp_read" => {
            let limit = args
                .get("max_bytes")
                .and_then(Value::as_u64)
                .unwrap_or(DEFAULT_READ_BYTES)
                .clamp(1, MAX_READ_BYTES);
            let (c2, h, p) = (c.clone(), host, path);
            ops_core::rt::run(
                async move { ops_ssh::sftp::sftp_read_file(c2, h, p, Some(limit)).await },
            )
            .await
            .and_then(|r| r)
            .and_then(|b64| decode_text(&b64))
            .map(ToolOutcome::ok)
        }
        "sftp_write" => {
            let content = arg(args, "content");
            let n = content.len();
            blocking(move || {
                ops_ssh::sftp::sftp_write_file(
                    &c.db,
                    &c.sftp,
                    &c.registry,
                    host,
                    path.clone(),
                    content,
                )
                .map(|_| format!("Wrote {n} bytes to {path}"))
            })
            .await
            .map(ToolOutcome::ok)
        }
        "sftp_mkdir" => blocking(move || {
            ops_ssh::sftp::sftp_mkdir(&c.db, &c.sftp, &c.registry, host, path.clone())
                .map(|_| format!("Created directory {path}"))
        })
        .await
        .map(ToolOutcome::ok),
        "sftp_rename" => {
            let (from, to) = (arg(args, "from"), arg(args, "to"));
            blocking(move || {
                ops_ssh::sftp::sftp_rename(&c.sftp, host, from.clone(), to.clone())
                    .map(|_| format!("Renamed {from} to {to}"))
            })
            .await
            .map(ToolOutcome::ok)
        }
        "sftp_upload" => {
            let (local, remote) = (arg(args, "local_path"), arg(args, "remote_path"));
            let size = std::fs::metadata(&local).map(|m| m.len()).unwrap_or(0);
            let (c2, h, l, r) = (c.clone(), host, local, remote);
            ops_core::rt::run(async move {
                ops_ssh::sftp::sftp_upload(c2, h, l, r.clone(), uuid::Uuid::new_v4().to_string())
                    .await
                    .map(|_| format!("Uploaded {size} bytes to {r}"))
            })
            .await
            .and_then(|r| r)
            .map(ToolOutcome::ok)
        }
        "sftp_download" => {
            let (remote, local) = (arg(args, "remote_path"), arg(args, "local_path"));
            let (c2, h, rm, l) = (c.clone(), host, remote, local);
            ops_core::rt::run(async move {
                ops_ssh::sftp::sftp_download(c2, h, rm, l.clone(), uuid::Uuid::new_v4().to_string())
                    .await
                    .map(|_| format!("Downloaded to {l}"))
            })
            .await
            .and_then(|r| r)
            .map(ToolOutcome::ok)
        }
        "sftp_chmod" => match parse_mode(&arg(args, "mode")) {
            Ok(mode) => blocking(move || {
                ops_ssh::sftp::sftp_chmod(&c.sftp, host, path.clone(), mode)
                    .map(|_| format!("Set {path} to {mode:04o}"))
            })
            .await
            .map(ToolOutcome::ok),
            Err(e) => Err(e),
        },
        "sftp_delete" => blocking(move || {
            let entry = ops_ssh::sftp::sftp_stat(&c.sftp, host.clone(), path.clone())?;
            if entry.is_dir && !entry.is_symlink {
                ops_ssh::sftp::sftp_rmdir(&c.sftp, host, path.clone())
                    .map_err(|e| format!("{e} (only empty directories can be deleted)"))?;
            } else {
                ops_ssh::sftp::sftp_remove(&c.sftp, host, path.clone())?;
            }
            Ok(format!("Deleted {path}"))
        })
        .await
        .map(ToolOutcome::ok),
        other => Err(format!("Unknown tool: {other}")),
    };
    result.unwrap_or_else(ToolOutcome::error)
}

fn entry_json(e: &ops_ssh::sftp::FileEntry) -> Value {
    json!({
        "name": e.name,
        "path": e.path,
        "type": if e.is_symlink { "symlink" } else if e.is_dir { "directory" } else { "file" },
        "size": e.size,
        "modified": e.modified,
        "permissions": e.permissions.map(|p| format!("{:04o}", p & 0o7777)),
    })
}

/// `"644"`, `"0755"`, `"2750"`: octal permission bits.
fn parse_mode(text: &str) -> Result<u32, String> {
    let t = text.trim();
    let valid = (3..=4).contains(&t.len()) && t.chars().all(|c| ('0'..='7').contains(&c));
    if !valid {
        return Err("`mode` must be 3 or 4 octal digits such as 644 or 0755.".into());
    }
    u32::from_str_radix(t, 8).map_err(|e| e.to_string())
}

/// The file content the backend returns as base64, as text; binary content is refused.
fn decode_text(b64: &str) -> Result<String, String> {
    use base64::Engine as _;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .map_err(|e| e.to_string())?;
    String::from_utf8(bytes).map_err(|_| {
        "The file is not valid UTF-8 text (binary files cannot be returned).".to_string()
    })
}

// ---------------------------------------------------------------------------
// Port forwards
// ---------------------------------------------------------------------------

fn forward_json(e: &ops_ssh::forward::ForwardEntry, host_name: &str) -> Value {
    json!({
        "forward_id": e.id,
        "host": host_name,
        "type": match e.config.forward_type { ForwardType::Local => "local", ForwardType::Remote => "remote", ForwardType::Dynamic => "dynamic" },
        "local_address": e.config.local_addr,
        "remote_address": e.config.remote_addr,
        "label": e.config.label,
        "status": match e.status { ops_ssh::forward::ForwardStatus::Active => "active", ops_ssh::forward::ForwardStatus::Suspended => "suspended", ops_ssh::forward::ForwardStatus::Error => "error" },
        "error": e.error,
        "bytes_sent": e.bytes_sent,
        "bytes_received": e.bytes_received,
    })
}

/// Forwards of every host that has an open SSH session.
async fn list_forwards(cx: &mut AsyncApp) -> ToolOutcome {
    let (ctx, hosts) = cx.update(|cx| {
        let mut hosts: Vec<(String, String)> = Vec::new();
        for s in super::sessions(cx).into_iter().filter(|s| !s.local) {
            if let Some(h) = s
                .host_id
                .clone()
                .filter(|h| !hosts.iter().any(|(id, _)| id == h))
            {
                hosts.push((h, s.name.clone()));
            }
        }
        (crate::services::ctx(cx), hosts)
    });
    let mut rows: Vec<Value> = Vec::new();
    for (host_id, name) in hosts {
        let c = ctx.clone();
        if let Ok(list) =
            blocking(move || ops_ssh::forward::forward_list(&c.forwards, host_id)).await
        {
            rows.extend(list.iter().map(|e| forward_json(e, &name)));
        }
    }
    ToolOutcome::ok(serde_json::to_string_pretty(&rows).unwrap_or_default())
}

fn valid_address(addr: &str) -> bool {
    match addr.rsplit_once(':') {
        Some((host, port)) => !host.trim().is_empty() && port.parse::<u16>().is_ok_and(|p| p > 0),
        None => false,
    }
}

/// The settings of a forward from the call's arguments; `base` is the forward being changed or copied (its settings
/// stay where the call does not name them).
fn forward_config(args: &Value, base: Option<&ForwardConfig>) -> Result<ForwardConfig, String> {
    let mut config = base.cloned().unwrap_or(ForwardConfig {
        forward_type: ForwardType::Local,
        local_addr: String::new(),
        remote_addr: String::new(),
        label: None,
    });
    match args.get("type").and_then(Value::as_str) {
        Some("local") => config.forward_type = ForwardType::Local,
        Some("remote") => config.forward_type = ForwardType::Remote,
        Some("dynamic") => config.forward_type = ForwardType::Dynamic,
        Some(other) => {
            return Err(format!(
                "`type` must be local, remote or dynamic (got \"{other}\")."
            ))
        }
        None if base.is_none() => return Err("`type` must be local, remote or dynamic.".into()),
        None => {}
    }
    if let Some(v) = args.get("local_address").and_then(Value::as_str) {
        config.local_addr = v.to_string();
    }
    if let Some(v) = args.get("remote_address").and_then(Value::as_str) {
        config.remote_addr = v.to_string();
    }
    if let Some(v) = args.get("label").and_then(Value::as_str) {
        config.label = Some(v.to_string()).filter(|l| !l.is_empty());
    }
    if !valid_address(&config.local_addr) {
        return Err("`local_address` must be host:port.".into());
    }
    if !matches!(config.forward_type, ForwardType::Dynamic) && !valid_address(&config.remote_addr) {
        return Err("`remote_address` must be host:port for local and remote forwards.".into());
    }
    Ok(config)
}

async fn start_forward(
    ctx: &std::sync::Arc<ops_ssh::Ctx>,
    target: &SshTarget,
    args: &Value,
) -> ToolOutcome {
    let config = match forward_config(args, None) {
        Ok(c) => c,
        Err(e) => return ToolOutcome::error(e),
    };
    let (c, host) = (ctx.clone(), target.host_id.clone());
    match blocking(move || {
        ops_ssh::forward::forward_add(c.clone(), &c.db, &c.registry, &c.forwards, host, config)
    })
    .await
    {
        Ok(entry) => ToolOutcome::ok(
            serde_json::to_string_pretty(&forward_json(&entry, &target.session.name))
                .unwrap_or_default(),
        ),
        Err(e) => ToolOutcome::error(format!("Could not start the forward: {e}")),
    }
}

/// The forward with this id, with the host it belongs to (looked up among the hosts that have an open session).
async fn find_forward(
    ctx: &std::sync::Arc<ops_ssh::Ctx>,
    forward_id: &str,
    cx: &mut AsyncApp,
) -> Option<(String, String, ops_ssh::forward::ForwardEntry)> {
    let hosts: Vec<(String, String)> = cx.update(|cx| {
        super::sessions(cx)
            .into_iter()
            .filter(|s| !s.local)
            .filter_map(|s| s.host_id.map(|h| (h, s.name)))
            .collect()
    });
    for (host_id, name) in hosts {
        let (c, h) = (ctx.clone(), host_id.clone());
        if let Ok(list) = blocking(move || ops_ssh::forward::forward_list(&c.forwards, h)).await {
            if let Some(entry) = list.into_iter().find(|e| e.id == forward_id) {
                return Some((host_id, name, entry));
            }
        }
    }
    None
}

fn unknown_forward(forward_id: &str) -> ToolOutcome {
    ToolOutcome::error(if forward_id.is_empty() {
        "`forward_id` is required.".to_string()
    } else {
        format!("Unknown forward_id: {forward_id}. Use portforward_list to see the forwards.")
    })
}

/// `portforward_stop` keeps the forward in the list (suspended); `portforward_delete` removes it.
async fn end_forward(
    ctx: &std::sync::Arc<ops_ssh::Ctx>,
    forward_id: &str,
    remove: bool,
    cx: &mut AsyncApp,
) -> ToolOutcome {
    let Some((host_id, name, entry)) = find_forward(ctx, forward_id, cx).await else {
        return unknown_forward(forward_id);
    };
    let (c, h, f) = (ctx.clone(), host_id.clone(), forward_id.to_string());
    let result = blocking(move || {
        if remove {
            ops_ssh::forward::forward_remove(&c.forwards, h, f)
        } else {
            ops_ssh::forward::forward_stop(&c.forwards, h, f)
        }
    })
    .await;
    match result {
        Ok(()) if remove => ToolOutcome::ok(format!("Removed forward {forward_id}")),
        Ok(()) => {
            // the forward task reports its new state asynchronously
            std::thread::sleep(std::time::Duration::from_millis(150));
            let (c, h) = (ctx.clone(), host_id);
            let now = blocking(move || ops_ssh::forward::forward_list(&c.forwards, h))
                .await
                .ok()
                .and_then(|l| l.into_iter().find(|e| e.id == forward_id))
                .unwrap_or(entry);
            ToolOutcome::ok(
                serde_json::to_string_pretty(&forward_json(&now, &name)).unwrap_or_default(),
            )
        }
        Err(e) => ToolOutcome::error(format!("Could not change the forward: {e}")),
    }
}

/// `portforward_update` (the old forward is replaced) and `portforward_duplicate` (the old one keeps running).
async fn change_forward(
    ctx: &std::sync::Arc<ops_ssh::Ctx>,
    args: &Value,
    duplicate: bool,
    cx: &mut AsyncApp,
) -> ToolOutcome {
    let forward_id = arg(args, "forward_id");
    let Some((host_id, name, old)) = find_forward(ctx, &forward_id, cx).await else {
        return unknown_forward(&forward_id);
    };
    let config = match forward_config(args, Some(&old.config)) {
        Ok(c) => c,
        Err(e) => return ToolOutcome::error(e),
    };
    if !duplicate {
        let (c, h, f) = (ctx.clone(), host_id.clone(), forward_id.clone());
        if let Err(e) = blocking(move || ops_ssh::forward::forward_remove(&c.forwards, h, f)).await
        {
            return ToolOutcome::error(format!("Could not stop the forward: {e}"));
        }
    }
    let add = |config: ForwardConfig| {
        let (c, h) = (ctx.clone(), host_id.clone());
        blocking(move || {
            ops_ssh::forward::forward_add(c.clone(), &c.db, &c.registry, &c.forwards, h, config)
        })
    };
    match add(config).await {
        Ok(entry) => ToolOutcome::ok(
            serde_json::to_string_pretty(&forward_json(&entry, &name)).unwrap_or_default(),
        ),
        Err(e) if duplicate => ToolOutcome::error(format!("Could not start the copy: {e}")),
        Err(e) => {
            // the change did not take: bring the old forward back so the call leaves things as they were
            let restored = add(old.config).await.is_ok();
            ToolOutcome::error(format!(
                "Could not apply the change: {e}. {}",
                if restored {
                    "The previous forward was started again."
                } else {
                    "The previous forward could not be restored."
                }
            ))
        }
    }
}

/// Names the object of a call in its record: ids mean nothing to the person approving it.
async fn describe(
    tool: &str,
    args: &Value,
    ctx: &std::sync::Arc<ops_ssh::Ctx>,
    cx: &mut AsyncApp,
) -> Option<String> {
    if vault::handles(tool) {
        return vault::describe(tool, args, ctx).await;
    }
    if matches!(
        tool,
        "portforward_stop" | "portforward_delete" | "portforward_update" | "portforward_duplicate"
    ) {
        let (_, name, e) = find_forward(ctx, &arg(args, "forward_id"), cx).await?;
        return Some(
            format!(
                "{name}: {} {}{}",
                e.config.label.clone().unwrap_or_default(),
                e.config.local_addr,
                if e.config.remote_addr.is_empty() {
                    String::new()
                } else {
                    format!(" ↔ {}", e.config.remote_addr)
                }
            )
            .trim()
            .to_string(),
        );
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{arg, capability_of, decode_text, handles, parse_mode, summary_of, valid_address};
    use serde_json::json;

    #[test]
    fn only_the_changing_tools_need_a_capability() {
        for tool in [
            "sftp_list",
            "sftp_stat",
            "sftp_read",
            "portforward_list",
            "vault_list_hosts",
            "vault_list_scripts",
            "vault_list_groups",
            "vault_get_script",
            "vault_get_script_run",
        ] {
            assert!(capability_of(tool).is_none(), "{tool}");
        }
        for tool in [
            "sftp_write",
            "sftp_mkdir",
            "sftp_rename",
            "sftp_delete",
            "sftp_upload",
            "sftp_download",
            "sftp_chmod",
            "portforward_start",
            "portforward_stop",
            "portforward_delete",
            "portforward_update",
            "portforward_duplicate",
        ] {
            let cap = capability_of(tool).unwrap();
            assert!(
                ops_ai::prefs::safety::GRANTABLE_CAPABILITIES.contains(&cap),
                "{cap} must be grantable"
            );
        }
        // every tool of the protocol layer beyond the terminal ones is handled here
        for def in ops_ai::mcp_bridge::tool_definitions() {
            let name = def["name"].as_str().unwrap();
            assert_eq!(
                handles(name),
                !matches!(
                    name,
                    "get_environment" | "list_sessions" | "terminal_read" | "terminal_execute"
                ),
                "{name}"
            );
        }
    }

    #[test]
    fn every_tool_has_a_name_in_both_languages() {
        for (lang, text) in [
            ("en-US", include_str!("../../../locales/extra/en-US.json")),
            ("zh-CN", include_str!("../../../locales/extra/zh-CN.json")),
        ] {
            let keys: serde_json::Map<String, serde_json::Value> =
                serde_json::from_str(text).unwrap();
            for def in ops_ai::mcp_bridge::tool_definitions() {
                let key = format!("mcpwin.tool.{}", def["name"].as_str().unwrap());
                assert!(keys.contains_key(&key), "{lang} lacks {key}");
            }
        }
    }

    #[test]
    fn addresses_need_a_host_and_a_port() {
        assert!(valid_address("127.0.0.1:8080"));
        assert!(valid_address("db.internal:5432"));
        assert!(!valid_address("8080"));
        assert!(!valid_address(":8080"));
        assert!(!valid_address("host:0"));
        assert!(!valid_address("host:99999"));
    }

    #[test]
    fn file_text_is_decoded_and_binary_refused() {
        use base64::Engine as _;
        let enc = |b: &[u8]| base64::engine::general_purpose::STANDARD.encode(b);
        assert_eq!(decode_text(&enc("héllo\n".as_bytes())).unwrap(), "héllo\n");
        assert!(decode_text(&enc(&[0xff, 0xfe, 0x00]))
            .unwrap_err()
            .contains("UTF-8"));
    }

    #[test]
    fn records_never_show_file_content() {
        let s = summary_of(
            "sftp_write",
            &json!({"path": "/etc/app.conf", "content": "secret=1"}),
        );
        assert_eq!(s, "/etc/app.conf (8 bytes)");
        assert_eq!(
            summary_of("sftp_rename", &json!({"from": "/a", "to": "/b"})),
            "/a → /b"
        );
    }

    #[test]
    fn permission_modes_are_octal() {
        assert_eq!(parse_mode("644"), Ok(0o644));
        assert_eq!(parse_mode(" 0755 "), Ok(0o755));
        assert_eq!(parse_mode("2750"), Ok(0o2750));
        for bad in ["", "64", "888", "rwx", "07777", "-644"] {
            assert!(parse_mode(bad).is_err(), "{bad}");
        }
    }
}
