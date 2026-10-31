//! The external MCP bridge: a small Streamable-HTTP MCP server (JSON-RPC over `POST /mcp`, bearer token) that lets an
//! outside agent (Codex, Claude Code, Cursor, ...) use the app's terminal sessions. This module speaks the protocol
//! only; every `tools/call` is handed to the app through a channel and answered once the app has decided (run,
//! refuse, or wait for the user's approval).

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{oneshot, watch};

pub const SERVER_NAME: &str = "opsbatch-external";
pub const PROTOCOL_VERSION: &str = "2025-03-26";
/// A tool call waits this long for the app (an approval can take a while).
pub const CALL_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const MAX_BODY: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolOutcome {
    pub text: String,
    pub is_error: bool,
}

impl ToolOutcome {
    pub fn ok(text: impl Into<String>) -> Self {
        Self { text: text.into(), is_error: false }
    }
    pub fn error(text: impl Into<String>) -> Self {
        Self { text: text.into(), is_error: true }
    }
}

/// One `tools/call` waiting for the app.
pub struct BridgeCall {
    pub tool: String,
    pub arguments: Value,
    pub reply: oneshot::Sender<ToolOutcome>,
}

/// The tools the bridge offers.
pub fn tool_definitions() -> Vec<Value> {
    vec![
        json!({
            "name": "get_environment",
            "description": "Describe the OpsBatch environment: app version, the permission mode that applies to write operations and the open terminal sessions.",
            "inputSchema": {"type": "object", "properties": {}, "additionalProperties": false}
        }),
        json!({
            "name": "list_sessions",
            "description": "List the terminal sessions (local shells and SSH hosts) that can be used with the other tools.",
            "inputSchema": {"type": "object", "properties": {}, "additionalProperties": false}
        }),
        json!({
            "name": "terminal_read",
            "description": "Read the last lines currently shown in a terminal session. Read only.",
            "inputSchema": {"type": "object", "properties": {
                "session_id": {"type": "string", "description": "A session_id from list_sessions. Defaults to the active session."},
                "lines": {"type": "integer", "minimum": 1, "maximum": 500, "description": "How many trailing lines to return (default 50)."}
            }, "additionalProperties": false}
        }),
        json!({
            "name": "terminal_execute",
            "description": "Run one shell command in a terminal session and return its output and exit code. Subject to the permission mode (observer refuses, confirm asks the user in OpsBatch, auto runs) and the command blocklist.",
            "inputSchema": {"type": "object", "properties": {
                "command": {"type": "string", "description": "The command line to run."},
                "session_id": {"type": "string", "description": "A session_id from list_sessions. Defaults to the active session."},
                "timeout_seconds": {"type": "integer", "minimum": 1, "maximum": 3600, "description": "Give up waiting after this many seconds (default: the command timeout of the safety settings)."}
            }, "required": ["command"], "additionalProperties": false}
        }),
        json!({
            "name": "sftp_list",
            "description": "List a directory on the server of an SSH session over SFTP. Read only.",
            "inputSchema": {"type": "object", "properties": {
                "session_id": session_id_property(),
                "path": {"type": "string", "description": "Absolute remote directory path."}
            }, "required": ["path"], "additionalProperties": false}
        }),
        json!({
            "name": "sftp_stat",
            "description": "Size, type, permissions and modification time of a remote file or directory. Read only.",
            "inputSchema": {"type": "object", "properties": {
                "session_id": session_id_property(),
                "path": {"type": "string", "description": "Absolute remote path."}
            }, "required": ["path"], "additionalProperties": false}
        }),
        json!({
            "name": "sftp_read",
            "description": "Read a remote text file (UTF-8) over SFTP. Binary files and files over the size limit are refused. Read only.",
            "inputSchema": {"type": "object", "properties": {
                "session_id": session_id_property(),
                "path": {"type": "string", "description": "Absolute remote file path."},
                "max_bytes": {"type": "integer", "minimum": 1, "maximum": 1048576, "description": "Refuse files larger than this (default 262144)."}
            }, "required": ["path"], "additionalProperties": false}
        }),
        json!({
            "name": "sftp_write",
            "description": "Create or overwrite a remote text file over SFTP. Subject to the permission mode (observer refuses, confirm asks the user in OpsBatch, auto writes).",
            "inputSchema": {"type": "object", "properties": {
                "session_id": session_id_property(),
                "path": {"type": "string", "description": "Absolute remote file path."},
                "content": {"type": "string", "description": "The complete new file content."}
            }, "required": ["path", "content"], "additionalProperties": false}
        }),
        json!({
            "name": "sftp_mkdir",
            "description": "Create a remote directory over SFTP (the parent directory must exist). Subject to the permission mode.",
            "inputSchema": {"type": "object", "properties": {
                "session_id": session_id_property(),
                "path": {"type": "string", "description": "Absolute remote directory path."}
            }, "required": ["path"], "additionalProperties": false}
        }),
        json!({
            "name": "sftp_rename",
            "description": "Rename or move a remote file or directory over SFTP. Subject to the permission mode.",
            "inputSchema": {"type": "object", "properties": {
                "session_id": session_id_property(),
                "from": {"type": "string", "description": "Existing absolute remote path."},
                "to": {"type": "string", "description": "New absolute remote path."}
            }, "required": ["from", "to"], "additionalProperties": false}
        }),
        json!({
            "name": "sftp_delete",
            "description": "Delete a remote file or an empty directory over SFTP. Subject to the permission mode.",
            "inputSchema": {"type": "object", "properties": {
                "session_id": session_id_property(),
                "path": {"type": "string", "description": "Absolute remote path of a file or an empty directory."}
            }, "required": ["path"], "additionalProperties": false}
        }),
        json!({
            "name": "sftp_upload",
            "description": "Upload a file from this computer to the server of an SSH session over SFTP (an existing remote file is overwritten). Subject to the permission mode; the approval shows both paths.",
            "inputSchema": {"type": "object", "properties": {
                "session_id": session_id_property(),
                "local_path": {"type": "string", "description": "Absolute path of an existing file on this computer."},
                "remote_path": {"type": "string", "description": "Absolute remote file path to write."}
            }, "required": ["local_path", "remote_path"], "additionalProperties": false}
        }),
        json!({
            "name": "sftp_download",
            "description": "Download a remote file to this computer over SFTP. An existing local file is only replaced with overwrite=true. Subject to the permission mode; the approval shows both paths.",
            "inputSchema": {"type": "object", "properties": {
                "session_id": session_id_property(),
                "remote_path": {"type": "string", "description": "Absolute remote file path."},
                "local_path": {"type": "string", "description": "Absolute path on this computer; its folder must exist."},
                "overwrite": {"type": "boolean", "description": "Replace the local file when it exists (default false)."}
            }, "required": ["remote_path", "local_path"], "additionalProperties": false}
        }),
        json!({
            "name": "sftp_chmod",
            "description": "Change the permission bits of a remote file or directory over SFTP. Subject to the permission mode.",
            "inputSchema": {"type": "object", "properties": {
                "session_id": session_id_property(),
                "path": {"type": "string", "description": "Absolute remote path."},
                "mode": {"type": "string", "description": "Octal permission bits such as \"644\", \"0755\" or \"2750\"."}
            }, "required": ["path", "mode"], "additionalProperties": false}
        }),
        json!({
            "name": "portforward_list",
            "description": "List the port forwards that exist for the open SSH sessions, with their status and traffic. Read only.",
            "inputSchema": {"type": "object", "properties": {}, "additionalProperties": false}
        }),
        json!({
            "name": "portforward_start",
            "description": "Start a port forward through an SSH session: local (listen here, connect from the server), remote (listen on the server, connect from here) or dynamic (SOCKS5 proxy). Subject to the permission mode.",
            "inputSchema": {"type": "object", "properties": {
                "session_id": session_id_property(),
                "type": {"type": "string", "enum": ["local", "remote", "dynamic"], "description": "Forward type."},
                "local_address": {"type": "string", "description": "host:port on this machine (the listen side of a local or dynamic forward, the target of a remote one)."},
                "remote_address": {"type": "string", "description": "host:port on the server side (the target of a local forward, the listen side of a remote one). Not used by dynamic forwards."},
                "label": {"type": "string", "description": "Optional name shown in OpsBatch."}
            }, "required": ["type", "local_address"], "additionalProperties": false}
        }),
        json!({
            "name": "portforward_stop",
            "description": "Stop a running port forward. It stays in portforward_list as suspended; use portforward_delete to remove it. Subject to the permission mode.",
            "inputSchema": {"type": "object", "properties": {
                "forward_id": {"type": "string", "description": "A forward id from portforward_list."}
            }, "required": ["forward_id"], "additionalProperties": false}
        }),
        json!({
            "name": "portforward_delete",
            "description": "Stop a port forward and remove it from the list. Subject to the permission mode.",
            "inputSchema": {"type": "object", "properties": {
                "forward_id": {"type": "string", "description": "A forward id from portforward_list."}
            }, "required": ["forward_id"], "additionalProperties": false}
        }),
        json!({
            "name": "portforward_update",
            "description": "Change a port forward: it is stopped and started again with the changed settings (the forward gets a new forward_id, which is returned). Only the fields you pass change. Subject to the permission mode.",
            "inputSchema": {"type": "object", "properties": {
                "forward_id": {"type": "string", "description": "A forward id from portforward_list."},
                "type": {"type": "string", "enum": ["local", "remote", "dynamic"]},
                "local_address": {"type": "string", "description": "host:port"},
                "remote_address": {"type": "string", "description": "host:port"},
                "label": {"type": "string"}
            }, "required": ["forward_id"], "additionalProperties": false}
        }),
        json!({
            "name": "portforward_duplicate",
            "description": "Start another forward with the same settings as an existing one, optionally with changes (a local forward needs a different local_address, since the port is taken). Subject to the permission mode.",
            "inputSchema": {"type": "object", "properties": {
                "forward_id": {"type": "string", "description": "The forward to copy, from portforward_list."},
                "local_address": {"type": "string", "description": "host:port"},
                "remote_address": {"type": "string", "description": "host:port"},
                "label": {"type": "string"}
            }, "required": ["forward_id"], "additionalProperties": false}
        }),
        json!({
            "name": "vault_list_hosts",
            "description": "List the hosts saved in OpsBatch (name, address, port, user, system, group, tags). Passwords and keys are never included. Read only.",
            "inputSchema": {"type": "object", "properties": {
                "query": {"type": "string", "description": "Only hosts whose name, address, group or tags contain this text (case-insensitive)."}
            }, "additionalProperties": false}
        }),
        json!({
            "name": "vault_list_scripts",
            "description": "List the scripts in the OpsBatch library (name, language, category, risk, description; not the script text). Read only.",
            "inputSchema": {"type": "object", "properties": {
                "query": {"type": "string", "description": "Only scripts whose name, category, tags or description contain this text (case-insensitive)."}
            }, "additionalProperties": false}
        }),
        json!({
            "name": "vault_list_groups",
            "description": "List the host groups (id, name, parent). Read only.",
            "inputSchema": {"type": "object", "properties": {}, "additionalProperties": false}
        }),
        json!({
            "name": "vault_get_script",
            "description": "Read one library script including its text. Read only.",
            "inputSchema": {"type": "object", "properties": {
                "script_id": {"type": "string", "description": "A script_id from vault_list_scripts."}
            }, "required": ["script_id"], "additionalProperties": false}
        }),
        json!({
            "name": "vault_create_host",
            "description": "Save a new host. Passwords and keys cannot be set through MCP: add them in OpsBatch afterwards. Subject to the permission mode.",
            "inputSchema": {"type": "object", "properties": {
                "name": {"type": "string"},
                "address": {"type": "string", "description": "IP address or host name."},
                "port": {"type": "integer", "minimum": 1, "maximum": 65535, "description": "Default 22."},
                "username": {"type": "string", "description": "Default root."},
                "auth_type": {"type": "string", "enum": ["password", "key"], "description": "How the host logs in (default password)."},
                "system": {"type": "string", "description": "linux, windows, ... (default linux)."},
                "tags": {"type": "array", "items": {"type": "string"}},
                "group_id": {"type": "string", "description": "A group_id from vault_list_groups."},
                "remark": {"type": "string"}
            }, "required": ["name", "address"], "additionalProperties": false}
        }),
        json!({
            "name": "vault_update_host",
            "description": "Change the settings of a saved host; only the fields you pass change and its credentials are kept. Subject to the permission mode.",
            "inputSchema": {"type": "object", "properties": {
                "host_id": {"type": "string", "description": "A host_id from vault_list_hosts."},
                "name": {"type": "string"},
                "address": {"type": "string"},
                "port": {"type": "integer", "minimum": 1, "maximum": 65535},
                "username": {"type": "string"},
                "auth_type": {"type": "string", "enum": ["password", "key"]},
                "system": {"type": "string"},
                "tags": {"type": "array", "items": {"type": "string"}},
                "group_id": {"type": "string", "description": "A group_id, or an empty string for no group."},
                "remark": {"type": "string"}
            }, "required": ["host_id"], "additionalProperties": false}
        }),
        json!({
            "name": "vault_set_host_notes",
            "description": "Set the notes (remark) of a saved host. Subject to the permission mode.",
            "inputSchema": {"type": "object", "properties": {
                "host_id": {"type": "string", "description": "A host_id from vault_list_hosts."},
                "notes": {"type": "string"}
            }, "required": ["host_id", "notes"], "additionalProperties": false}
        }),
        json!({
            "name": "vault_delete_host",
            "description": "Delete a saved host together with its stored credentials. Subject to the permission mode.",
            "inputSchema": {"type": "object", "properties": {
                "host_id": {"type": "string", "description": "A host_id from vault_list_hosts."}
            }, "required": ["host_id"], "additionalProperties": false}
        }),
        json!({
            "name": "vault_create_group",
            "description": "Create a host group. Subject to the permission mode.",
            "inputSchema": {"type": "object", "properties": {
                "name": {"type": "string"},
                "parent_id": {"type": "string", "description": "A group_id to nest it in."}
            }, "required": ["name"], "additionalProperties": false}
        }),
        json!({
            "name": "vault_update_group",
            "description": "Rename a host group or move it under another one. Subject to the permission mode.",
            "inputSchema": {"type": "object", "properties": {
                "group_id": {"type": "string", "description": "A group_id from vault_list_groups."},
                "name": {"type": "string"},
                "parent_id": {"type": "string", "description": "A group_id, or an empty string for the top level."}
            }, "required": ["group_id"], "additionalProperties": false}
        }),
        json!({
            "name": "vault_delete_group",
            "description": "Delete a host group; its hosts stay and become ungrouped. Subject to the permission mode.",
            "inputSchema": {"type": "object", "properties": {
                "group_id": {"type": "string", "description": "A group_id from vault_list_groups."}
            }, "required": ["group_id"], "additionalProperties": false}
        }),
        json!({
            "name": "vault_create_script",
            "description": "Add a script to the library. Subject to the permission mode.",
            "inputSchema": {"type": "object", "properties": {
                "name": {"type": "string"},
                "content": {"type": "string", "description": "The script text."},
                "language": {"type": "string", "description": "shell, powershell, python, ... (default shell)."},
                "category": {"type": "string"},
                "tags": {"type": "array", "items": {"type": "string"}},
                "risk": {"type": "string", "enum": ["low", "medium", "high"], "description": "Default low."},
                "description": {"type": "string"},
                "platform": {"type": "string", "description": "linux, windows, ... (default linux)."}
            }, "required": ["name", "content"], "additionalProperties": false}
        }),
        json!({
            "name": "vault_update_script",
            "description": "Change a library script; only the fields you pass change. Built-in scripts cannot be changed. Subject to the permission mode.",
            "inputSchema": {"type": "object", "properties": {
                "script_id": {"type": "string", "description": "A script_id from vault_list_scripts."},
                "name": {"type": "string"},
                "content": {"type": "string"},
                "language": {"type": "string"},
                "category": {"type": "string"},
                "tags": {"type": "array", "items": {"type": "string"}},
                "risk": {"type": "string", "enum": ["low", "medium", "high"]},
                "description": {"type": "string"},
                "platform": {"type": "string"}
            }, "required": ["script_id"], "additionalProperties": false}
        }),
        json!({
            "name": "vault_delete_script",
            "description": "Delete a library script. Built-in scripts cannot be deleted. Subject to the permission mode.",
            "inputSchema": {"type": "object", "properties": {
                "script_id": {"type": "string", "description": "A script_id from vault_list_scripts."}
            }, "required": ["script_id"], "additionalProperties": false}
        }),
        json!({
            "name": "vault_run_script",
            "description": "Run a shell library script on one or more saved hosts over SSH (several hosts run side by side, up to 5 at a time) and return each host's output and exit status. If the run takes longer than wait_seconds it keeps running: use vault_get_script_run to follow it. A script that is already running on a host cannot be interrupted; vault_stop_script_run only skips the hosts that have not started yet. Subject to the permission mode and the command blocklist; the approval names the script, its risk and the hosts.",
            "inputSchema": {"type": "object", "properties": {
                "script_id": {"type": "string", "description": "A script_id from vault_list_scripts (language shell; scripts that download from a URL are not supported)."},
                "host_ids": {"type": "array", "items": {"type": "string"}, "minItems": 1, "maxItems": 20, "description": "host_id values from vault_list_hosts."},
                "timeout_seconds": {"type": "integer", "minimum": 1, "maximum": 600, "description": "How long to wait for the SSH connection to a host (default 30). It does not limit how long the script runs."},
                "wait_seconds": {"type": "integer", "minimum": 0, "maximum": 600, "description": "How long this call waits for the run to finish (default 60). 0 returns the run_id at once."}
            }, "required": ["script_id", "host_ids"], "additionalProperties": false}
        }),
        json!({
            "name": "vault_get_script_run",
            "description": "The state of a script run started with vault_run_script: running or finished, and the output and exit status of every host that has finished. Read only.",
            "inputSchema": {"type": "object", "properties": {
                "run_id": {"type": "string", "description": "The run_id returned by vault_run_script."}
            }, "required": ["run_id"], "additionalProperties": false}
        }),
        json!({
            "name": "vault_stop_script_run",
            "description": "Stop a running script run: hosts that have not started yet are skipped; a script already running on a host cannot be interrupted and finishes on its own. Subject to the permission mode.",
            "inputSchema": {"type": "object", "properties": {
                "run_id": {"type": "string", "description": "The run_id returned by vault_run_script."}
            }, "required": ["run_id"], "additionalProperties": false}
        }),
    ]
}

fn session_id_property() -> Value {
    json!({"type": "string", "description": "A session_id of an SSH session from list_sessions. Defaults to the active SSH session."})
}

/// Answers one JSON-RPC message. `None` is for notifications (no response body).
pub async fn handle_rpc(message: &Value, calls: &flume::Sender<BridgeCall>) -> Option<Value> {
    let id = message.get("id").cloned();
    let method = message.get("method").and_then(|m| m.as_str()).unwrap_or("");
    let respond = |result: Value| id.clone().map(|id| json!({"jsonrpc": "2.0", "id": id, "result": result}));
    let fail = |code: i64, text: &str| id.clone().map(|id| json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": text}}));
    if id.is_none() {
        // notifications (`notifications/initialized`, ...) are accepted silently
        return None;
    }
    match method {
        "initialize" => {
            let requested = message.pointer("/params/protocolVersion").and_then(|v| v.as_str()).unwrap_or(PROTOCOL_VERSION);
            respond(json!({
                "protocolVersion": requested,
                "capabilities": {"tools": {"listChanged": false}},
                "serverInfo": {"name": SERVER_NAME, "version": env!("CARGO_PKG_VERSION")},
                "instructions": "Tools operate on the terminal sessions open in the OpsBatch desktop app. Write operations follow the permission mode set in OpsBatch."
            }))
        }
        "ping" => respond(json!({})),
        "tools/list" => respond(json!({"tools": tool_definitions()})),
        "tools/call" => {
            let name = message.pointer("/params/name").and_then(|v| v.as_str()).unwrap_or("").to_string();
            if !tool_definitions().iter().any(|t| t["name"] == name.as_str()) {
                return fail(-32602, &format!("Unknown tool: {name}"));
            }
            let arguments = message.pointer("/params/arguments").cloned().unwrap_or_else(|| json!({}));
            let (tx, rx) = oneshot::channel();
            if calls.send_async(BridgeCall { tool: name, arguments, reply: tx }).await.is_err() {
                return respond(json!({"content": [{"type": "text", "text": "OpsBatch is not accepting tool calls right now."}], "isError": true}));
            }
            let outcome = match tokio::time::timeout(CALL_TIMEOUT, rx).await {
                Ok(Ok(o)) => o,
                Ok(Err(_)) => ToolOutcome::error("The call was cancelled."),
                Err(_) => ToolOutcome::error("Timed out waiting for OpsBatch."),
            };
            respond(json!({"content": [{"type": "text", "text": outcome.text}], "isError": outcome.is_error}))
        }
        other => fail(-32601, &format!("Method not found: {other}")),
    }
}

// ---------------------------------------------------------------------------
// HTTP
// ---------------------------------------------------------------------------

struct HttpRequest {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl HttpRequest {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

async fn read_request(stream: &mut TcpStream) -> Result<HttpRequest, String> {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 8192];
    let header_end = loop {
        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break pos + 4;
        }
        if buf.len() > 64 * 1024 {
            return Err("headers too large".into());
        }
        let n = stream.read(&mut tmp).await.map_err(|e| e.to_string())?;
        if n == 0 {
            return Err("closed".into());
        }
        buf.extend_from_slice(&tmp[..n]);
    };
    let head = String::from_utf8_lossy(&buf[..header_end]).to_string();
    let mut lines = head.lines();
    let first = lines.next().unwrap_or("");
    let mut parts = first.split_whitespace();
    let (method, path) = (parts.next().unwrap_or("").to_string(), parts.next().unwrap_or("").to_string());
    let headers: Vec<(String, String)> = lines.filter_map(|l| l.split_once(':')).map(|(k, v)| (k.trim().to_string(), v.trim().to_string())).collect();
    let length = headers.iter().find(|(k, _)| k.eq_ignore_ascii_case("content-length")).and_then(|(_, v)| v.parse::<usize>().ok()).unwrap_or(0);
    if length > MAX_BODY {
        return Err("body too large".into());
    }
    let mut body = buf[header_end..].to_vec();
    while body.len() < length {
        let n = stream.read(&mut tmp).await.map_err(|e| e.to_string())?;
        if n == 0 {
            return Err("closed".into());
        }
        body.extend_from_slice(&tmp[..n]);
    }
    body.truncate(length);
    Ok(HttpRequest { method, path, headers, body })
}

async fn write_response(stream: &mut TcpStream, status: u16, reason: &str, body: &str) {
    let reply = format!("HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{body}", body.len());
    let _ = stream.write_all(reply.as_bytes()).await;
    let _ = stream.shutdown().await;
}

/// Compares in constant time so a wrong token leaks nothing through timing.
fn token_matches(given: &str, expected: &str) -> bool {
    let (a, b) = (given.as_bytes(), expected.as_bytes());
    let mut diff = (a.len() != b.len()) as u8;
    for i in 0..a.len().max(b.len()) {
        diff |= a.get(i).copied().unwrap_or(0) ^ b.get(i).copied().unwrap_or(0);
    }
    diff == 0
}

/// A browser page must not be able to drive the bridge (DNS rebinding): an `Origin` has to be this machine.
fn origin_allowed(origin: Option<&str>) -> bool {
    match origin {
        None => true,
        Some(o) => {
            let host = o.split("://").nth(1).unwrap_or(o).split('/').next().unwrap_or("");
            let host = host.rsplit_once(':').map(|(h, p)| if p.chars().all(|c| c.is_ascii_digit()) { h } else { host }).unwrap_or(host);
            matches!(host.trim_matches(|c| c == '[' || c == ']'), "localhost" | "127.0.0.1" | "::1")
        }
    }
}

async fn serve_connection(mut stream: TcpStream, token: Arc<String>, calls: flume::Sender<BridgeCall>) {
    let request = match tokio::time::timeout(Duration::from_secs(30), read_request(&mut stream)).await {
        Ok(Ok(r)) => r,
        _ => return,
    };
    if request.path.split('?').next() != Some("/mcp") {
        return write_response(&mut stream, 404, "Not Found", r#"{"error":"not found"}"#).await;
    }
    if !origin_allowed(request.header("origin")) {
        return write_response(&mut stream, 403, "Forbidden", r#"{"error":"origin not allowed"}"#).await;
    }
    let bearer = request.header("authorization").and_then(|h| h.strip_prefix("Bearer ").or_else(|| h.strip_prefix("bearer "))).unwrap_or("");
    if !token_matches(bearer, &token) {
        return write_response(&mut stream, 401, "Unauthorized", r#"{"error":"invalid bearer token"}"#).await;
    }
    match request.method.as_str() {
        "POST" => {}
        // the legacy HTTP+SSE transport is not offered
        "GET" => return write_response(&mut stream, 405, "Method Not Allowed", r#"{"error":"use POST"}"#).await,
        "DELETE" => return write_response(&mut stream, 200, "OK", "{}").await,
        _ => return write_response(&mut stream, 405, "Method Not Allowed", r#"{"error":"use POST"}"#).await,
    }
    let Ok(message) = serde_json::from_slice::<Value>(&request.body) else {
        return write_response(&mut stream, 400, "Bad Request", r#"{"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"Parse error"}}"#).await;
    };
    let answer = match &message {
        Value::Array(items) => {
            let mut out = Vec::new();
            for m in items {
                if let Some(r) = handle_rpc(m, &calls).await {
                    out.push(r);
                }
            }
            (!out.is_empty()).then(|| Value::Array(out))
        }
        m => handle_rpc(m, &calls).await,
    };
    match answer {
        Some(body) => write_response(&mut stream, 200, "OK", &body.to_string()).await,
        None => write_response(&mut stream, 202, "Accepted", "").await,
    }
}

/// A running bridge; dropping it (or `stop`) closes the listener.
pub struct BridgeHandle {
    pub addr: SocketAddr,
    shutdown: watch::Sender<bool>,
}

impl BridgeHandle {
    pub fn stop(&self) {
        let _ = self.shutdown.send(true);
    }
}

impl Drop for BridgeHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Listens on `listen_address:port`; every accepted `tools/call` goes to `calls`.
pub async fn start(listen_address: &str, port: u16, token: String, calls: flume::Sender<BridgeCall>) -> Result<BridgeHandle, String> {
    let ip: std::net::IpAddr = listen_address.trim().parse().map_err(|_| "invalid_address".to_string())?;
    let listener = TcpListener::bind(SocketAddr::new(ip, port)).await.map_err(|e| if e.kind() == std::io::ErrorKind::AddrInUse { "port_in_use".to_string() } else { e.to_string() })?;
    let addr = listener.local_addr().map_err(|e| e.to_string())?;
    let (shutdown, mut stop) = watch::channel(false);
    let token = Arc::new(token);
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = stop.changed() => break,
                accepted = listener.accept() => {
                    let Ok((stream, _)) = accepted else { continue };
                    tokio::spawn(serve_connection(stream, token.clone(), calls.clone()));
                }
            }
        }
    });
    Ok(BridgeHandle { addr, shutdown })
}

// ---------------------------------------------------------------------------
// Discovery file and the stdio launcher
// ---------------------------------------------------------------------------

/// Env var a launcher reads its discovery file path from (the MCP client config sets it).
pub const DISCOVERY_ENV: &str = "OPSBATCH_EXTERNAL_MCP_DISCOVERY_FILE";

/// `<data dir>/mcp/discovery.json`
pub fn default_discovery_path(data_dir: &std::path::Path) -> std::path::PathBuf {
    data_dir.join("mcp").join("discovery.json")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Discovery {
    pub url: String,
    pub token: String,
}

/// Written while the bridge runs so a stdio launcher can find the port and token (private to the user, like the
/// original's discovery file); removed when the bridge stops.
pub fn write_discovery(path: &std::path::Path, addr: SocketAddr, token: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let host = if addr.ip().is_unspecified() { "127.0.0.1".to_string() } else if addr.is_ipv6() { format!("[{}]", addr.ip()) } else { addr.ip().to_string() };
    let body = json!({"url": format!("http://{host}:{}/mcp", addr.port()), "token": token, "pid": std::process::id(), "version": env!("CARGO_PKG_VERSION")});
    let tmp = path.with_extension("json.tmp");
    {
        use std::io::Write as _;
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&tmp)?;
        file.write_all(body.to_string().as_bytes())?;
    }
    std::fs::rename(&tmp, path)
}

pub fn remove_discovery(path: &std::path::Path) {
    let _ = std::fs::remove_file(path);
}

pub fn read_discovery(path: &std::path::Path) -> Result<Discovery, String> {
    let text = std::fs::read_to_string(path).map_err(|_| "OpsBatch is not running, or External MCP is switched off in OpsBatch (Settings → AI → Tool access).".to_string())?;
    let v: Value = serde_json::from_str(&text).map_err(|e| format!("The discovery file is damaged: {e}"))?;
    let url = v["url"].as_str().filter(|u| !u.is_empty()).ok_or("The discovery file has no url")?.to_string();
    let token = v["token"].as_str().unwrap_or("").to_string();
    Ok(Discovery { url, token })
}

/// The stdio MCP transport: newline-delimited JSON-RPC on `input` / `output`, each message forwarded to the running
/// app over HTTP (the discovery file is read for every message, so an app restart is picked up). Failures are answered
/// as JSON-RPC errors so the client sees why instead of hanging.
pub fn run_stdio_proxy<R: std::io::BufRead, W: std::io::Write>(discovery: &std::path::Path, input: R, output: &mut W) -> std::io::Result<()> {
    let client = reqwest::blocking::Client::builder().timeout(CALL_TIMEOUT + Duration::from_secs(30)).build().map_err(std::io::Error::other)?;
    for line in input.lines() {
        let line = line?;
        let text = line.trim();
        if text.is_empty() {
            continue;
        }
        let parsed: Option<Value> = serde_json::from_str(text).ok();
        let id = parsed.as_ref().and_then(|v| v.get("id")).cloned();
        let fail = |output: &mut W, message: String| -> std::io::Result<()> {
            // notifications have no id and get no answer
            match &id {
                Some(id) => writeln!(output, "{}", json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32000, "message": message}})),
                None => Ok(()),
            }
        };
        let found = match read_discovery(discovery) {
            Ok(d) => d,
            Err(e) => {
                fail(output, e)?;
                output.flush()?;
                continue;
            }
        };
        let response = client.post(&found.url).header("Content-Type", "application/json").header("Authorization", format!("Bearer {}", found.token)).body(text.to_string()).send();
        match response {
            Ok(r) if r.status().as_u16() == 202 => {}
            Ok(r) if r.status().is_success() => {
                let body = r.text().unwrap_or_default();
                let body = body.trim();
                if !body.is_empty() {
                    // one message per line
                    writeln!(output, "{}", body.replace('\n', " "))?;
                }
            }
            Ok(r) if r.status().as_u16() == 401 => fail(output, "OpsBatch refused the token. Restart your MCP client so it reads the new discovery file.".into())?,
            Ok(r) => fail(output, format!("OpsBatch answered HTTP {}.", r.status()))?,
            Err(_) => fail(output, "Could not reach OpsBatch. Is it running with External MCP switched on?".into())?,
        }
        output.flush()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn post(addr: SocketAddr, token: Option<&str>, origin: Option<&str>, method: &str, path: &str, body: &str) -> (u16, String) {
        let mut s = TcpStream::connect(addr).await.unwrap();
        let mut req = format!("{method} {path} HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n", body.len());
        if let Some(t) = token {
            req.push_str(&format!("Authorization: Bearer {t}\r\n"));
        }
        if let Some(o) = origin {
            req.push_str(&format!("Origin: {o}\r\n"));
        }
        req.push_str("\r\n");
        req.push_str(body);
        s.write_all(req.as_bytes()).await.unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).await.unwrap();
        let status = out.split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
        (status, out.split_once("\r\n\r\n").map(|(_, b)| b.to_string()).unwrap_or_default())
    }

    async fn rpc(addr: SocketAddr, body: Value) -> (u16, Value) {
        let (status, text) = post(addr, Some("tok"), None, "POST", "/mcp", &body.to_string()).await;
        (status, serde_json::from_str(&text).unwrap_or(Value::Null))
    }

    #[test]
    fn tokens_and_origins() {
        assert!(token_matches("abc", "abc"));
        assert!(!token_matches("abd", "abc"));
        assert!(!token_matches("ab", "abc"));
        assert!(!token_matches("", "abc"));
        assert!(origin_allowed(None));
        assert!(origin_allowed(Some("http://localhost:3000")));
        assert!(origin_allowed(Some("http://127.0.0.1")));
        assert!(origin_allowed(Some("http://[::1]:80")));
        assert!(!origin_allowed(Some("https://evil.example")));
        assert!(!origin_allowed(Some("http://localhost.evil.example")));
    }

    #[test]
    fn every_tool_has_a_schema() {
        let tools = tool_definitions();
        let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        // the terminal tools come first; every name is unique and snake_case
        assert_eq!(&names[..4], ["get_environment", "list_sessions", "terminal_read", "terminal_execute"]);
        let mut unique = names.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), names.len());
        assert!(names.iter().all(|n| n.chars().all(|c| c.is_ascii_lowercase() || c == '_')));
        assert_eq!(names.len(), 37);
        assert!(tools.iter().all(|t| t["inputSchema"]["type"] == "object" && t["description"].as_str().is_some_and(|d| !d.is_empty())));
        assert_eq!(tools[3]["inputSchema"]["required"][0], "command");
    }

    #[tokio::test]
    async fn the_protocol_handshake_and_listing() {
        let (tx, _rx) = flume::unbounded();
        let h = start("127.0.0.1", 0, "tok".into(), tx).await.unwrap();
        let (status, init) = rpc(h.addr, json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": {"name": "t", "version": "1"}}})).await;
        assert_eq!(status, 200);
        assert_eq!(init["result"]["protocolVersion"], "2024-11-05");
        assert_eq!(init["result"]["serverInfo"]["name"], SERVER_NAME);
        assert!(init["result"]["capabilities"]["tools"].is_object());
        // a notification gets no body
        let (status, text) = post(h.addr, Some("tok"), None, "POST", "/mcp", r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#).await;
        assert_eq!((status, text.as_str()), (202, ""));
        let (_, list) = rpc(h.addr, json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"})).await;
        assert_eq!(list["result"]["tools"].as_array().unwrap().len(), tool_definitions().len());
        let (_, ping) = rpc(h.addr, json!({"jsonrpc": "2.0", "id": 3, "method": "ping"})).await;
        assert_eq!(ping["result"], json!({}));
        let (_, unknown) = rpc(h.addr, json!({"jsonrpc": "2.0", "id": 4, "method": "nope"})).await;
        assert_eq!(unknown["error"]["code"], -32601);
        let (_, bad_tool) = rpc(h.addr, json!({"jsonrpc": "2.0", "id": 5, "method": "tools/call", "params": {"name": "rm_rf"}})).await;
        assert_eq!(bad_tool["error"]["code"], -32602);
        // batches are answered as a batch
        let (_, batch) = rpc(h.addr, json!([{"jsonrpc": "2.0", "id": 6, "method": "ping"}, {"jsonrpc": "2.0", "method": "notifications/x"}, {"jsonrpc": "2.0", "id": 7, "method": "ping"}])).await;
        assert_eq!(batch.as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn requests_are_authenticated_and_confined() {
        let (tx, _rx) = flume::unbounded();
        let h = start("127.0.0.1", 0, "tok".into(), tx).await.unwrap();
        let body = r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#;
        assert_eq!(post(h.addr, None, None, "POST", "/mcp", body).await.0, 401);
        assert_eq!(post(h.addr, Some("wrong"), None, "POST", "/mcp", body).await.0, 401);
        assert_eq!(post(h.addr, Some("tok"), Some("https://evil.example"), "POST", "/mcp", body).await.0, 403);
        assert_eq!(post(h.addr, Some("tok"), Some("http://localhost:1234"), "POST", "/mcp", body).await.0, 200);
        assert_eq!(post(h.addr, Some("tok"), None, "POST", "/other", body).await.0, 404);
        assert_eq!(post(h.addr, Some("tok"), None, "GET", "/mcp", "").await.0, 405);
        assert_eq!(post(h.addr, Some("tok"), None, "POST", "/mcp", "{not json").await.0, 400);
        h.stop();
    }

    #[tokio::test]
    async fn tool_calls_wait_for_the_app_and_carry_its_answer() {
        let (tx, rx) = flume::unbounded::<BridgeCall>();
        let h = start("127.0.0.1", 0, "tok".into(), tx).await.unwrap();
        tokio::spawn(async move {
            while let Ok(call) = rx.recv_async().await {
                let outcome = match call.tool.as_str() {
                    "terminal_execute" => ToolOutcome::ok(format!("ran: {}", call.arguments["command"].as_str().unwrap_or(""))),
                    _ => ToolOutcome::error("refused"),
                };
                let _ = call.reply.send(outcome);
            }
        });
        let (_, ok) = rpc(h.addr, json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "terminal_execute", "arguments": {"command": "uptime"}}})).await;
        assert_eq!(ok["result"]["content"][0]["text"], "ran: uptime");
        assert_eq!(ok["result"]["isError"], false);
        let (_, denied) = rpc(h.addr, json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": "list_sessions"}})).await;
        assert_eq!((denied["result"]["isError"].as_bool(), denied["result"]["content"][0]["text"].as_str()), (Some(true), Some("refused")));
    }

    #[test]
    fn the_discovery_file_round_trips_privately() {
        let dir = std::env::temp_dir().join(format!("opsbatch-disc-{}", uuid::Uuid::new_v4().simple()));
        let path = default_discovery_path(&dir);
        assert!(read_discovery(&path).unwrap_err().contains("not running"));
        write_discovery(&path, "0.0.0.0:4567".parse().unwrap(), "tok").unwrap();
        assert_eq!(read_discovery(&path).unwrap(), Discovery { url: "http://127.0.0.1:4567/mcp".into(), token: "tok".into() });
        write_discovery(&path, "[::1]:9".parse().unwrap(), "t2").unwrap();
        assert_eq!(read_discovery(&path).unwrap().url, "http://[::1]:9/mcp");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        }
        std::fs::write(&path, "not json").unwrap();
        assert!(read_discovery(&path).unwrap_err().contains("damaged"));
        remove_discovery(&path);
        assert!(!path.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_stdio_proxy_forwards_lines_and_reports_failures() {
        let (tx, rx) = flume::unbounded::<BridgeCall>();
        let h = start("127.0.0.1", 0, "tok".into(), tx).await.unwrap();
        tokio::spawn(async move {
            while let Ok(call) = rx.recv_async().await {
                let _ = call.reply.send(ToolOutcome::ok(format!("echo:{}", call.arguments["x"].as_str().unwrap_or(""))));
            }
        });
        let dir = std::env::temp_dir().join(format!("opsbatch-stdio-{}", uuid::Uuid::new_v4().simple()));
        let path = default_discovery_path(&dir);
        let run = |path: std::path::PathBuf, input: String| {
            tokio::task::spawn_blocking(move || {
                let mut out = Vec::new();
                run_stdio_proxy(&path, std::io::Cursor::new(input), &mut out).unwrap();
                String::from_utf8(out).unwrap().lines().map(|l| serde_json::from_str::<Value>(l).unwrap()).collect::<Vec<_>>()
            })
        };
        let session = [
            json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}).to_string(),
            json!({"jsonrpc": "2.0", "method": "notifications/initialized"}).to_string(),
            String::new(),
            json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}).to_string(),
            json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "get_environment", "arguments": {"x": "hi"}}}).to_string(),
        ]
        .join("\n");

        // no discovery file: requests are answered with an error, notifications are dropped
        let out = run(path.clone(), session.clone()).await.unwrap();
        assert_eq!(out.len(), 3);
        assert!(out.iter().all(|m| m["error"]["message"].as_str().unwrap().contains("not running")));
        assert_eq!(out[0]["id"], 1);

        write_discovery(&path, h.addr, "tok").unwrap();
        let out = run(path.clone(), session.clone()).await.unwrap();
        assert_eq!(out.len(), 3, "the notification is forwarded and answered with 202, which prints nothing");
        assert_eq!(out[0]["result"]["serverInfo"]["name"], SERVER_NAME);
        assert_eq!(out[1]["result"]["tools"].as_array().unwrap().len(), tool_definitions().len());
        assert_eq!(out[2]["result"]["content"][0]["text"], "echo:hi");

        // a wrong token and a stopped server are explained
        write_discovery(&path, h.addr, "wrong").unwrap();
        let out = run(path.clone(), json!({"jsonrpc": "2.0", "id": 9, "method": "ping"}).to_string()).await.unwrap();
        assert!(out[0]["error"]["message"].as_str().unwrap().contains("refused the token"));
        write_discovery(&path, h.addr, "tok").unwrap();
        h.stop();
        tokio::time::sleep(Duration::from_millis(150)).await;
        let out = run(path.clone(), json!({"jsonrpc": "2.0", "id": 10, "method": "ping"}).to_string()).await.unwrap();
        assert!(out[0]["error"]["message"].as_str().unwrap().contains("Could not reach"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn the_listener_reports_a_busy_port_and_stops() {
        let (tx, _rx) = flume::unbounded();
        let a = start("127.0.0.1", 0, "t".into(), tx.clone()).await.unwrap();
        assert_eq!(start("127.0.0.1", a.addr.port(), "t".into(), tx.clone()).await.err().as_deref(), Some("port_in_use"));
        assert_eq!(start("not-an-ip", 0, "t".into(), tx.clone()).await.err().as_deref(), Some("invalid_address"));
        let port = a.addr.port();
        a.stop();
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(TcpStream::connect(("127.0.0.1", port)).await.is_err() || start("127.0.0.1", port, "t".into(), tx).await.is_ok());
    }
}
