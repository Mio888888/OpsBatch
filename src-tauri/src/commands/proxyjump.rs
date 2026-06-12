use rusqlite::params;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs as std_fs;
use std::path::PathBuf;

use crate::db::Database;
use crate::ssh::SshConnectionRegistry;

// ---------------------------------------------------------------------------
// SSH config parsing
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SshConfigEntry {
    pub host_alias: String,
    pub hostname: Option<String>,
    pub port: u16,
    pub user: String,
    pub identity_file: Option<String>,
    pub proxy_jump: Option<String>,
}

#[tauri::command]
pub fn parse_ssh_config() -> Result<Vec<SshConfigEntry>, String> {
    let home = dirs_home_dir()?;
    let config_path = home.join(".ssh").join("config");

    if !config_path.exists() {
        return Ok(vec![]);
    }

    let content =
        std_fs::read_to_string(&config_path).map_err(|e| format!("读取 SSH config 失败: {}", e))?;

    Ok(parse_ssh_config_content(&content))
}

fn dirs_home_dir() -> Result<PathBuf, String> {
    // Try $HOME first, then fallback
    if let Ok(home) = std::env::var("HOME") {
        if !home.is_empty() {
            return Ok(PathBuf::from(home));
        }
    }
    // Fallback: use the home directory from the `dirs` crate logic
    Ok(PathBuf::from("/").join(
        std::env::var("USER")
            .map(|u| format!("home/{}", u))
            .unwrap_or_else(|_| "root".to_string()),
    ))
}

fn parse_ssh_config_content(content: &str) -> Vec<SshConfigEntry> {
    let mut entries = Vec::new();
    let mut current: Option<SshConfigEntry> = None;

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let (key, value) = if let Some(pos) = line.find(char::is_whitespace) {
            (&line[..pos], line[pos..].trim())
        } else if let Some(pos) = line.find('=') {
            (&line[..pos], line[pos + 1..].trim())
        } else {
            continue;
        };

        let key_lower = key.to_lowercase();

        match key_lower.as_str() {
            "host" => {
                if let Some(entry) = current.take() {
                    entries.push(entry);
                }
                // Skip wildcard/pattern hosts
                if value.contains('*') || value.contains('?') {
                    continue;
                }
                current = Some(SshConfigEntry {
                    host_alias: value.to_string(),
                    hostname: None,
                    port: 22,
                    user: String::new(),
                    identity_file: None,
                    proxy_jump: None,
                });
            }
            "hostname" => {
                if let Some(ref mut entry) = current {
                    entry.hostname = Some(value.to_string());
                }
            }
            "port" => {
                if let Some(ref mut entry) = current {
                    entry.port = value.parse().unwrap_or(22);
                }
            }
            "user" => {
                if let Some(ref mut entry) = current {
                    entry.user = value.to_string();
                }
            }
            "identityfile" => {
                if let Some(ref mut entry) = current {
                    // Expand ~ to home dir
                    let path = if value.starts_with("~/") {
                        let home = std::env::var("HOME").unwrap_or_else(|_| "/".to_string());
                        format!("{}{}", home, &value[1..])
                    } else {
                        value.to_string()
                    };
                    entry.identity_file = Some(path);
                }
            }
            "proxyjump" => {
                if let Some(ref mut entry) = current {
                    entry.proxy_jump = Some(value.to_string());
                }
            }
            _ => {}
        }
    }

    if let Some(entry) = current.take() {
        entries.push(entry);
    }

    entries
}

// ---------------------------------------------------------------------------
// Jump chain resolution (Dijkstra shortest path)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JumpTopologyNode {
    pub id: String,
    pub name: String,
    pub ip: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JumpTopologyEdge {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JumpTopology {
    pub nodes: Vec<JumpTopologyNode>,
    pub edges: Vec<JumpTopologyEdge>,
}

/// Build a jump topology graph from all hosts' jump_chain fields.
fn build_topology(
    db: &Database,
) -> Result<(Vec<JumpTopologyNode>, HashMap<String, Vec<String>>), String> {
    let rows = {
        let conn = db.pool.get().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare("SELECT id, name, ip, jump_chain FROM hosts")
            .map_err(|e| e.to_string())?;

        let rows: Vec<(String, String, String, String)> = stmt
            .query_map([], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get::<_, Option<String>>(3)?
                        .unwrap_or_else(|| "[]".to_string()),
                ))
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        drop(stmt);
        drop(conn);
        rows
    };

    let mut nodes = Vec::new();
    let mut adjacency: HashMap<String, Vec<String>> = HashMap::new();

    for (id, name, ip, jump_chain_json) in &rows {
        nodes.push(JumpTopologyNode {
            id: id.clone(),
            name: name.clone(),
            ip: ip.clone(),
        });

        let chain: Vec<String> = serde_json::from_str(jump_chain_json).unwrap_or_default();
        let mut path_segments = chain.clone();
        path_segments.push(id.clone());

        for i in 0..path_segments.len().saturating_sub(1) {
            adjacency
                .entry(path_segments[i].clone())
                .or_default()
                .push(path_segments[i + 1].clone());
            adjacency
                .entry(path_segments[i + 1].clone())
                .or_default()
                .push(path_segments[i].clone());
        }
    }

    Ok((nodes, adjacency))
}

#[tauri::command]
pub fn get_jump_topology(db: tauri::State<'_, Database>) -> Result<JumpTopology, String> {
    let (nodes, adjacency) = build_topology(&db)?;

    let mut edges = Vec::new();
    let mut seen = HashSet::new();
    for (from, tos) in &adjacency {
        for to in tos {
            let key = if from < to {
                format!("{}:{}", from, to)
            } else {
                format!("{}:{}", to, from)
            };
            if seen.insert(key) {
                edges.push(JumpTopologyEdge {
                    from: from.clone(),
                    to: to.clone(),
                });
            }
        }
    }

    Ok(JumpTopology { nodes, edges })
}

/// Dijkstra shortest path from "client" (no specific node) to target host.
/// Returns the ordered list of jump host IDs.
#[tauri::command]
pub fn resolve_jump_chain(
    db: tauri::State<'_, Database>,
    host_id: String,
) -> Result<Vec<String>, String> {
    // First check if the host already has an explicit jump_chain
    let conn = db.pool.get().map_err(|e| e.to_string())?;
    let existing_chain: Option<String> = conn
        .query_row(
            "SELECT jump_chain FROM hosts WHERE id=?1",
            params![host_id],
            |row| row.get(0),
        )
        .ok();
    drop(conn);

    if let Some(chain_json) = existing_chain {
        let chain: Vec<String> = serde_json::from_str(&chain_json).unwrap_or_default();
        if !chain.is_empty() {
            return Ok(chain);
        }
    }

    // Try to find a path using the topology graph
    let (_, adjacency) = build_topology(&db)?;

    // BFS from all directly reachable hosts (hosts with empty jump_chain, meaning client can reach them directly)
    let client_reachable = {
        let conn = db.pool.get().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare("SELECT id FROM hosts WHERE jump_chain = '[]' OR jump_chain IS NULL")
            .map_err(|e| e.to_string())?;
        let ids: Vec<String> = stmt
            .query_map([], |row| row.get(0))
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();
        drop(stmt);
        drop(conn);
        ids
    };

    // BFS from client-reachable nodes to find shortest path to host_id
    let mut visited: HashSet<String> = HashSet::new();
    let mut queue: VecDeque<(String, Vec<String>)> = VecDeque::new();

    for id in &client_reachable {
        if id == &host_id {
            return Ok(vec![]); // Target is directly reachable
        }
        visited.insert(id.clone());
        queue.push_back((id.clone(), vec![id.clone()]));
    }

    while let Some((current, path)) = queue.pop_front() {
        if let Some(neighbors) = adjacency.get(&current) {
            for neighbor in neighbors {
                if neighbor == &host_id {
                    return Ok(path);
                }
                if !visited.contains(neighbor) {
                    visited.insert(neighbor.clone());
                    let mut new_path = path.clone();
                    new_path.push(neighbor.clone());
                    queue.push_back((neighbor.clone(), new_path));
                }
            }
        }
    }

    Ok(vec![]) // No path found, treat as direct connection
}

// ---------------------------------------------------------------------------
// Cascade disconnect
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn cascade_disconnect(
    pool: tauri::State<'_, SshConnectionRegistry>,
    db: tauri::State<'_, Database>,
    jump_host_id: String,
) -> Result<Vec<String>, String> {
    let downstream = {
        let conn = db.pool.get().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare("SELECT id, jump_chain FROM hosts")
            .map_err(|e| e.to_string())?;
        let rows: Vec<(String, String)> = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?
                        .unwrap_or_else(|| "[]".to_string()),
                ))
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        drop(stmt);
        drop(conn);

        let mut result = Vec::new();
        for (host_id, chain_json) in &rows {
            let chain: Vec<String> = serde_json::from_str(chain_json).unwrap_or_default();
            if chain.contains(&jump_host_id) {
                result.push(host_id.clone());
            }
        }
        result
    };

    // Disconnect all downstream hosts from pool
    let mut disconnected: Vec<String> = Vec::new();
    for host_id in &downstream {
        if pool.remove_connection(host_id) {
            disconnected.push(host_id.clone());
        }
    }

    // Also disconnect the jump host itself
    pool.remove_connection(&jump_host_id);

    Ok(disconnected)
}

// ---------------------------------------------------------------------------
// Import SSH config entries as hosts
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct ImportSshConfigRequest {
    pub entries: Vec<SshConfigEntry>,
    pub group_id: Option<String>,
}

#[tauri::command]
pub fn import_ssh_config_hosts(
    db: tauri::State<'_, Database>,
    request: ImportSshConfigRequest,
) -> Result<Vec<String>, String> {
    let conn = db.pool.get().map_err(|e| e.to_string())?;
    let mut imported_ids = Vec::new();

    for entry in &request.entries {
        let id = uuid::Uuid::new_v4().to_string();
        let hostname = entry.hostname.as_deref().unwrap_or(&entry.host_alias);

        // Read identity file content if provided
        let private_key = entry
            .identity_file
            .as_ref()
            .and_then(|path| std_fs::read_to_string(path).ok());

        let auth_type = if private_key.is_some() {
            "key"
        } else {
            "password"
        };
        let jump_chain = entry.proxy_jump.as_deref().unwrap_or("[]");

        conn.execute(
            "INSERT OR IGNORE INTO hosts (id, name, ip, port, auth_type, username, password, private_key, os, tags, group_id, remark, status, jump_chain) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![
                id,
                entry.host_alias,
                hostname,
                entry.port,
                auth_type,
                entry.user,
                None::<String>, // password
                private_key,
                "linux",
                "[]",
                request.group_id,
                "",
                "unknown",
                jump_chain,
            ],
        )
        .map_err(|e| format!("导入 {} 失败: {}", entry.host_alias, e))?;

        imported_ids.push(id);
    }

    Ok(imported_ids)
}
