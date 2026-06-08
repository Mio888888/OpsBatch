use crate::db::Database;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;

// ---------------------------------------------------------------------------
// MCP Types (JSON-RPC 2.0 based)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
struct JsonRpcResponse {
    jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct JsonRpcError {
    code: i64,
    message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpTool {
    pub name: String,
    pub description: Option<String>,
    #[serde(rename = "inputSchema")]
    pub input_schema: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    pub id: String,
    pub name: String,
    pub transport: String, // "stdio" or "sse"
    pub command: Option<String>,
    pub args: Option<Vec<String>>,
    pub env: Option<HashMap<String, String>>,
    pub url: Option<String>,
    pub enabled: bool,
}

// ---------------------------------------------------------------------------
// MCP Server State
// ---------------------------------------------------------------------------

pub struct McpManager {
    processes: Mutex<HashMap<String, Child>>,
    request_id: Mutex<u64>,
}

impl McpManager {
    pub fn new() -> Self {
        Self {
            processes: Mutex::new(HashMap::new()),
            request_id: Mutex::new(0),
        }
    }
}

// ---------------------------------------------------------------------------
// MCP DB Tables
// ---------------------------------------------------------------------------

pub fn init_mcp_tables(conn: &rusqlite::Connection) -> Result<(), String> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS mcp_servers (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            transport TEXT NOT NULL DEFAULT 'stdio',
            command TEXT,
            args TEXT DEFAULT '[]',
            env TEXT DEFAULT '{}',
            url TEXT,
            enabled INTEGER DEFAULT 1,
            created_at TEXT DEFAULT (datetime('now', 'localtime'))
        );
        ",
    )
    .map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// MCP Commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn mcp_list_servers(db: tauri::State<'_, Database>) -> Result<Vec<McpServerConfig>, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare("SELECT id, name, transport, command, args, env, url, enabled FROM mcp_servers ORDER BY name")
        .map_err(|e| e.to_string())?;

    let rows = stmt
        .query_map([], |row| {
            let args_str: String = row.get(4).unwrap_or_else(|_| "[]".into());
            let env_str: String = row.get(5).unwrap_or_else(|_| "{}".into());
            Ok(McpServerConfig {
                id: row.get(0)?,
                name: row.get(1)?,
                transport: row.get(2)?,
                command: row.get(3)?,
                args: serde_json::from_str(&args_str).unwrap_or_default(),
                env: serde_json::from_str(&env_str).unwrap_or_default(),
                url: row.get(6)?,
                enabled: row.get::<_, i32>(7)? == 1,
            })
        })
        .map_err(|e| e.to_string())?;

    let mut result = Vec::new();
    for row in rows {
        result.push(row.map_err(|e| e.to_string())?);
    }
    Ok(result)
}

#[tauri::command]
pub fn mcp_add_server(
    db: tauri::State<'_, Database>,
    config: McpServerConfig,
) -> Result<(), String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let args_json = serde_json::to_string(&config.args).map_err(|e| e.to_string())?;
    let env_json = serde_json::to_string(&config.env).map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT INTO mcp_servers (id, name, transport, command, args, env, url, enabled) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![config.id, config.name, config.transport, config.command, args_json, env_json, config.url, config.enabled as i32],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn mcp_remove_server(db: tauri::State<'_, Database>, server_id: String) -> Result<(), String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM mcp_servers WHERE id = ?1", params![server_id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn mcp_connect_stdio(
    manager: tauri::State<'_, Arc<McpManager>>,
    server_id: String,
    command: String,
    args: Vec<String>,
    env: HashMap<String, String>,
) -> Result<(), String> {
    let mut child = Command::new(&command)
        .args(&args)
        .envs(&env)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("启动 MCP 服务器失败: {}", e))?;

    // Send initialize request
    let stdin = child.stdin.as_mut().ok_or("无法获取 stdin")?;
    let init_request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {"name": "OpsBatch", "version": "1.0.0"}
        }
    });
    let init_str = format!(
        "{}\n",
        serde_json::to_string(&init_request).map_err(|e| e.to_string())?
    );
    stdin
        .write_all(init_str.as_bytes())
        .await
        .map_err(|e| format!("写入失败: {}", e))?;
    stdin
        .flush()
        .await
        .map_err(|e| format!("flush 失败: {}", e))?;

    let mut processes = manager.processes.lock().await;
    processes.insert(server_id, child);
    Ok(())
}

#[tauri::command]
pub async fn mcp_disconnect(
    manager: tauri::State<'_, Arc<McpManager>>,
    server_id: String,
) -> Result<(), String> {
    let mut processes = manager.processes.lock().await;
    if let Some(mut child) = processes.remove(&server_id) {
        let _ = child.kill().await;
    }
    Ok(())
}

#[tauri::command]
pub async fn mcp_call_tool(
    manager: tauri::State<'_, Arc<McpManager>>,
    server_id: String,
    tool_name: String,
    arguments: Value,
) -> Result<Value, String> {
    let mut processes = manager.processes.lock().await;
    let child = processes.get_mut(&server_id).ok_or("MCP 服务器未连接")?;

    let id = {
        let mut id = manager.request_id.lock().await;
        *id += 1;
        *id
    };

    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "tools/call",
        "params": {
            "name": tool_name,
            "arguments": arguments
        }
    });

    let stdin = child.stdin.as_mut().ok_or("无法获取 stdin")?;
    let request_str = format!(
        "{}\n",
        serde_json::to_string(&request).map_err(|e| e.to_string())?
    );
    stdin
        .write_all(request_str.as_bytes())
        .await
        .map_err(|e| format!("写入失败: {}", e))?;
    stdin
        .flush()
        .await
        .map_err(|e| format!("flush 失败: {}", e))?;

    let stdout = child.stdout.as_mut().ok_or("无法获取 stdout")?;
    let mut reader = BufReader::new(stdout);
    let mut response_line = String::new();
    reader
        .read_line(&mut response_line)
        .await
        .map_err(|e| format!("读取失败: {}", e))?;

    let response: JsonRpcResponse =
        serde_json::from_str(response_line.trim()).map_err(|e| format!("解析响应失败: {}", e))?;

    if let Some(error) = response.error {
        return Err(format!("MCP 错误: {} - {}", error.code, error.message));
    }

    Ok(response.result.unwrap_or(Value::Null))
}

#[tauri::command]
pub async fn mcp_list_tools(
    manager: tauri::State<'_, Arc<McpManager>>,
    server_id: String,
) -> Result<Vec<McpTool>, String> {
    let mut processes = manager.processes.lock().await;
    let child = processes.get_mut(&server_id).ok_or("MCP 服务器未连接")?;

    let id = {
        let mut id = manager.request_id.lock().await;
        *id += 1;
        *id
    };

    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "tools/list",
        "params": {}
    });

    let stdin = child.stdin.as_mut().ok_or("无法获取 stdin")?;
    let request_str = format!(
        "{}\n",
        serde_json::to_string(&request).map_err(|e| e.to_string())?
    );
    stdin
        .write_all(request_str.as_bytes())
        .await
        .map_err(|e| format!("写入失败: {}", e))?;
    stdin
        .flush()
        .await
        .map_err(|e| format!("flush 失败: {}", e))?;

    let stdout = child.stdout.as_mut().ok_or("无法获取 stdout")?;
    let mut reader = BufReader::new(stdout);
    let mut response_line = String::new();
    reader
        .read_line(&mut response_line)
        .await
        .map_err(|e| format!("读取失败: {}", e))?;

    let response: JsonRpcResponse =
        serde_json::from_str(response_line.trim()).map_err(|e| format!("解析响应失败: {}", e))?;

    let tools_value = response.result.unwrap_or(Value::Null);
    let tools: Vec<McpTool> = serde_json::from_value(
        tools_value
            .get("tools")
            .cloned()
            .unwrap_or(Value::Array(vec![])),
    )
    .map_err(|e| format!("解析工具列表失败: {}", e))?;

    Ok(tools)
}
