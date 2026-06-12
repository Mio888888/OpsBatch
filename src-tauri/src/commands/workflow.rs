use crate::db::Database;
use rusqlite::params;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workflow {
    pub id: String,
    pub name: String,
    pub description: String,
    pub nodes: String,
    pub connections: String,
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
}

// ===== Workflow CRUD =====

#[tauri::command]
pub async fn list_workflows(db: tauri::State<'_, Database>) -> Result<Vec<Workflow>, String> {
    let conn = db.pool.get().map_err(|e| e.to_string())?;
    let mut stmt = conn.prepare(
        "SELECT id, name, description, nodes, connections, status, created_at, updated_at FROM workflows ORDER BY updated_at DESC"
    ).map_err(|e| e.to_string())?;
    let workflows = stmt
        .query_map([], |row| {
            Ok(Workflow {
                id: row.get(0)?,
                name: row.get(1)?,
                description: row.get(2)?,
                nodes: row.get(3)?,
                connections: row.get(4)?,
                status: row.get(5)?,
                created_at: row.get(6)?,
                updated_at: row.get(7)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(workflows)
}

#[tauri::command]
pub async fn create_workflow(
    db: tauri::State<'_, Database>,
    name: String,
    description: String,
) -> Result<Workflow, String> {
    let id = uuid::Uuid::new_v4().to_string();
    let conn = db.pool.get().map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT INTO workflows (id, name, description, nodes, connections, status) VALUES (?1, ?2, ?3, '[]', '[]', 'draft')",
        params![id, name, description],
    ).map_err(|e| e.to_string())?;

    let workflow = conn.query_row(
        "SELECT id, name, description, nodes, connections, status, created_at, updated_at FROM workflows WHERE id=?1",
        params![id],
        |row| {
            Ok(Workflow {
                id: row.get(0)?,
                name: row.get(1)?,
                description: row.get(2)?,
                nodes: row.get(3)?,
                connections: row.get(4)?,
                status: row.get(5)?,
                created_at: row.get(6)?,
                updated_at: row.get(7)?,
            })
        },
    ).map_err(|e| e.to_string())?;

    Ok(workflow)
}

#[tauri::command]
pub async fn update_workflow(
    db: tauri::State<'_, Database>,
    workflow: Workflow,
) -> Result<(), String> {
    let conn = db.pool.get().map_err(|e| e.to_string())?;
    conn.execute(
        "UPDATE workflows SET name=?1, description=?2, nodes=?3, connections=?4, status=?5, updated_at=datetime('now','localtime') WHERE id=?6",
        params![workflow.name, workflow.description, workflow.nodes, workflow.connections, workflow.status, workflow.id],
    ).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn delete_workflow(db: tauri::State<'_, Database>, id: String) -> Result<(), String> {
    let conn = db.pool.get().map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM workflows WHERE id=?1", params![id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

// ===== Workflow Templates =====

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowTemplate {
    pub id: String,
    pub name: String,
    pub description: String,
    pub nodes: String,
    pub connections: String,
    pub created_at: String,
}

#[tauri::command]
pub async fn list_workflow_templates(
    db: tauri::State<'_, Database>,
) -> Result<Vec<WorkflowTemplate>, String> {
    let conn = db.pool.get().map_err(|e| e.to_string())?;
    let mut stmt = conn.prepare(
        "SELECT id, name, description, nodes, connections, created_at FROM workflow_templates ORDER BY name"
    ).map_err(|e| e.to_string())?;
    let templates = stmt
        .query_map([], |row| {
            Ok(WorkflowTemplate {
                id: row.get(0)?,
                name: row.get(1)?,
                description: row.get(2)?,
                nodes: row.get(3)?,
                connections: row.get(4)?,
                created_at: row.get(5)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(templates)
}

#[tauri::command]
pub async fn save_workflow_template(
    db: tauri::State<'_, Database>,
    name: String,
    description: String,
    nodes: String,
    connections: String,
) -> Result<String, String> {
    let id = uuid::Uuid::new_v4().to_string();
    let conn = db.pool.get().map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT INTO workflow_templates (id, name, description, nodes, connections) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![id, name, description, nodes, connections],
    ).map_err(|e| e.to_string())?;
    Ok(id)
}

#[tauri::command]
pub async fn delete_workflow_template(
    db: tauri::State<'_, Database>,
    id: String,
) -> Result<(), String> {
    let conn = db.pool.get().map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM workflow_templates WHERE id=?1", params![id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

// ===== Scheduled Tasks =====

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduledTask {
    pub id: String,
    pub name: String,
    pub cron: String,
    pub workflow_id: String,
    pub enabled: bool,
    pub last_run_at: Option<String>,
    pub next_run_at: Option<String>,
    pub created_at: String,
}

#[tauri::command]
pub async fn list_scheduled_tasks(
    db: tauri::State<'_, Database>,
) -> Result<Vec<ScheduledTask>, String> {
    let conn = db.pool.get().map_err(|e| e.to_string())?;
    let mut stmt = conn.prepare(
        "SELECT id, name, cron, workflow_id, enabled, last_run_at, next_run_at, created_at FROM scheduled_tasks ORDER BY name"
    ).map_err(|e| e.to_string())?;
    let tasks = stmt
        .query_map([], |row| {
            Ok(ScheduledTask {
                id: row.get(0)?,
                name: row.get(1)?,
                cron: row.get(2)?,
                workflow_id: row.get(3)?,
                enabled: row.get::<_, i32>(4)? == 1,
                last_run_at: row.get(5)?,
                next_run_at: row.get(6)?,
                created_at: row.get(7)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(tasks)
}

#[tauri::command]
pub async fn add_scheduled_task(
    db: tauri::State<'_, Database>,
    name: String,
    cron: String,
    workflow_id: String,
) -> Result<String, String> {
    let id = uuid::Uuid::new_v4().to_string();
    let conn = db.pool.get().map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT INTO scheduled_tasks (id, name, cron, workflow_id, enabled) VALUES (?1, ?2, ?3, ?4, 1)",
        params![id, name, cron, workflow_id],
    ).map_err(|e| e.to_string())?;
    Ok(id)
}

#[tauri::command]
pub async fn update_scheduled_task(
    db: tauri::State<'_, Database>,
    id: String,
    name: String,
    cron: String,
    workflow_id: String,
    enabled: bool,
) -> Result<(), String> {
    let conn = db.pool.get().map_err(|e| e.to_string())?;
    conn.execute(
        "UPDATE scheduled_tasks SET name=?1, cron=?2, workflow_id=?3, enabled=?4 WHERE id=?5",
        params![name, cron, workflow_id, enabled as i32, id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn delete_scheduled_task(
    db: tauri::State<'_, Database>,
    id: String,
) -> Result<(), String> {
    let conn = db.pool.get().map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM scheduled_tasks WHERE id=?1", params![id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Check scheduled tasks that are due based on simple interval parsing.
/// Supports formats like: "every:60" (every 60 seconds), "every:3600" (every hour)
/// For full cron support, a cron library would be needed.
#[tauri::command]
pub async fn check_scheduled_tasks(db: tauri::State<'_, Database>) -> Result<Vec<String>, String> {
    let tasks: Vec<(String, String, String, String, Option<String>)> = {
        let conn = db.pool.get().map_err(|e| e.to_string())?;
        let mut stmt = conn.prepare(
            "SELECT id, name, cron, workflow_id, last_run_at FROM scheduled_tasks WHERE enabled=1"
        ).map_err(|e| e.to_string())?;

        let rows: Vec<(String, String, String, String, Option<String>)> = stmt
            .query_map([], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        rows
    };
    // conn is dropped here

    let now = chrono::Local::now();
    let mut due_tasks = Vec::new();

    for (id, name, cron, _workflow_id, last_run) in &tasks {
        // Parse interval from cron: expect format "every:N" where N is seconds
        let interval_secs = if let Some(secs) = cron.strip_prefix("every:") {
            secs.parse::<i64>().unwrap_or(0)
        } else {
            // Default to 1 hour for unrecognized formats
            3600
        };

        if interval_secs <= 0 {
            continue;
        }

        let should_run = match last_run {
            None => true,
            Some(lr) => {
                if let Ok(last_time) = chrono::DateTime::parse_from_str(lr, "%Y-%m-%d %H:%M:%S") {
                    let elapsed =
                        now.signed_duration_since(last_time.with_timezone(&chrono::Local));
                    elapsed.num_seconds() >= interval_secs
                } else {
                    true
                }
            }
        };

        if should_run {
            due_tasks.push(format!("{}: {} (到期)", id, name));
        }
    }

    Ok(due_tasks)
}
