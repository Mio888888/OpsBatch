//! Workflows, templates and scheduled tasks in SQLite (port of `commands/workflow.rs`).

use chrono::NaiveDateTime;
use ops_core::db::Database;
use rusqlite::params;

use crate::model::{
    parse_nodes_payload, stringify_nodes_payload, Connection, WorkflowRecord, WorkflowSettings,
    WorkflowStatus,
};

const WORKFLOW_COLUMNS: &str =
    "id, name, description, nodes, connections, status, created_at, updated_at";

fn record_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<WorkflowRecord> {
    let nodes_raw: String = row.get(3)?;
    let connections_raw: String = row.get(4)?;
    // A damaged row still shows up (empty) instead of hiding every workflow.
    let payload = parse_nodes_payload(&nodes_raw).unwrap_or(crate::model::WorkflowPayload {
        nodes: Vec::new(),
        settings: WorkflowSettings::default(),
    });
    let connections: Vec<Connection> = serde_json::from_str(if connections_raw.is_empty() {
        "[]"
    } else {
        &connections_raw
    })
    .unwrap_or_default();
    let status: String = row.get(5)?;
    Ok(WorkflowRecord {
        id: row.get(0)?,
        name: row.get(1)?,
        description: row.get(2)?,
        nodes: payload.nodes,
        connections,
        settings: payload.settings,
        status: WorkflowStatus::parse(&status),
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
    })
}

pub fn list_workflows(db: &Database) -> Result<Vec<WorkflowRecord>, String> {
    let conn = db.pool.get().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(&format!(
            "SELECT {WORKFLOW_COLUMNS} FROM workflows ORDER BY updated_at DESC"
        ))
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], record_from_row)
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(rows)
}

pub fn get_workflow(db: &Database, id: &str) -> Result<WorkflowRecord, String> {
    let conn = db.pool.get().map_err(|e| e.to_string())?;
    conn.query_row(
        &format!("SELECT {WORKFLOW_COLUMNS} FROM workflows WHERE id=?1"),
        params![id],
        record_from_row,
    )
    .map_err(|e| e.to_string())
}

pub fn create_workflow(
    db: &Database,
    name: &str,
    description: &str,
) -> Result<WorkflowRecord, String> {
    let id = uuid::Uuid::new_v4().to_string();
    let conn = db.pool.get().map_err(|e| e.to_string())?;
    conn.execute("INSERT INTO workflows (id, name, description, nodes, connections, status) VALUES (?1, ?2, ?3, '[]', '[]', 'draft')", params![id, name, description]).map_err(|e| e.to_string())?;
    drop(conn);
    get_workflow(db, &id)
}

pub fn update_workflow(db: &Database, workflow: &WorkflowRecord) -> Result<(), String> {
    let conn = db.pool.get().map_err(|e| e.to_string())?;
    conn.execute(
        "UPDATE workflows SET name=?1, description=?2, nodes=?3, connections=?4, status=?5, updated_at=datetime('now','localtime') WHERE id=?6",
        params![
            workflow.name,
            workflow.description,
            stringify_nodes_payload(&workflow.nodes, workflow.settings),
            serde_json::to_string(&workflow.connections).map_err(|e| e.to_string())?,
            workflow.status.as_str(),
            workflow.id
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn delete_workflow(db: &Database, id: &str) -> Result<(), String> {
    let conn = db.pool.get().map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM workflows WHERE id=?1", params![id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

// ---- templates ----

#[derive(Debug, Clone, PartialEq)]
pub struct WorkflowTemplate {
    pub id: String,
    pub name: String,
    pub description: String,
    /// Raw `nodes` column (payload JSON), kept as text so a template round-trips untouched.
    pub nodes: String,
    pub connections: String,
    pub created_at: String,
}

pub fn list_templates(db: &Database) -> Result<Vec<WorkflowTemplate>, String> {
    let conn = db.pool.get().map_err(|e| e.to_string())?;
    let mut stmt = conn.prepare("SELECT id, name, description, nodes, connections, created_at FROM workflow_templates ORDER BY name").map_err(|e| e.to_string())?;
    let rows = stmt
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
    Ok(rows)
}

pub fn save_template(
    db: &Database,
    name: &str,
    description: &str,
    nodes: &str,
    connections: &str,
) -> Result<String, String> {
    let id = uuid::Uuid::new_v4().to_string();
    let conn = db.pool.get().map_err(|e| e.to_string())?;
    conn.execute("INSERT INTO workflow_templates (id, name, description, nodes, connections) VALUES (?1, ?2, ?3, ?4, ?5)", params![id, name, description, nodes, connections]).map_err(|e| e.to_string())?;
    Ok(id)
}

pub fn delete_template(db: &Database, id: &str) -> Result<(), String> {
    let conn = db.pool.get().map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM workflow_templates WHERE id=?1", params![id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

// ---- scheduled tasks ----

#[derive(Debug, Clone, PartialEq)]
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

pub fn list_scheduled_tasks(db: &Database) -> Result<Vec<ScheduledTask>, String> {
    let conn = db.pool.get().map_err(|e| e.to_string())?;
    let mut stmt = conn.prepare("SELECT id, name, cron, workflow_id, enabled, last_run_at, next_run_at, created_at FROM scheduled_tasks ORDER BY name").map_err(|e| e.to_string())?;
    let rows = stmt
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
    Ok(rows)
}

pub fn add_scheduled_task(
    db: &Database,
    name: &str,
    cron: &str,
    workflow_id: &str,
) -> Result<String, String> {
    let id = uuid::Uuid::new_v4().to_string();
    let conn = db.pool.get().map_err(|e| e.to_string())?;
    conn.execute("INSERT INTO scheduled_tasks (id, name, cron, workflow_id, enabled) VALUES (?1, ?2, ?3, ?4, 1)", params![id, name, cron, workflow_id]).map_err(|e| e.to_string())?;
    Ok(id)
}

pub fn update_scheduled_task(
    db: &Database,
    id: &str,
    name: &str,
    cron: &str,
    workflow_id: &str,
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

pub fn delete_scheduled_task(db: &Database, id: &str) -> Result<(), String> {
    let conn = db.pool.get().map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM scheduled_tasks WHERE id=?1", params![id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Seconds between runs for a rule: `every:N`; anything else is treated as hourly, a broken `N` as "never".
pub fn interval_seconds(rule: &str) -> i64 {
    match rule.strip_prefix("every:") {
        Some(n) => n.trim().parse::<i64>().unwrap_or(0),
        None => 3600,
    }
}

/// Enabled tasks that are due at `now`. (The Tauri build compared against a time-zone-less string with a
/// zone-aware parser, which always failed and so reported every task as due on every check.)
pub fn due_tasks(tasks: &[ScheduledTask], now: NaiveDateTime) -> Vec<String> {
    tasks
        .iter()
        .filter(|t| t.enabled)
        .filter_map(|t| {
            let interval = interval_seconds(&t.cron);
            if interval <= 0 {
                return None;
            }
            let due = match t
                .last_run_at
                .as_deref()
                .map(|lr| NaiveDateTime::parse_from_str(lr, "%Y-%m-%d %H:%M:%S"))
            {
                None => true,
                Some(Ok(last)) => now.signed_duration_since(last).num_seconds() >= interval,
                Some(Err(_)) => true,
            };
            due.then(|| format!("{}: {} (到期)", t.id, t.name))
        })
        .collect()
}

pub fn check_scheduled_tasks(db: &Database) -> Result<Vec<String>, String> {
    Ok(due_tasks(
        &list_scheduled_tasks(db)?,
        chrono::Local::now().naive_local(),
    ))
}

/// Stamp `last_run_at` after a scheduled run started.
pub fn mark_task_ran(db: &Database, id: &str) -> Result<(), String> {
    let conn = db.pool.get().map_err(|e| e.to_string())?;
    conn.execute(
        "UPDATE scheduled_tasks SET last_run_at=datetime('now','localtime') WHERE id=?1",
        params![id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::CanvasNode;

    fn core() -> ops_core::Core {
        ops_core::Core::open_temp("wf").unwrap()
    }

    #[test]
    fn workflows_round_trip_through_sqlite() {
        let core = core();
        let db = &core.db;
        let created = create_workflow(db, "发布", "上线流程").unwrap();
        assert_eq!(
            (created.status, created.nodes.len()),
            (WorkflowStatus::Draft, 0)
        );
        let mut wf = created.clone();
        wf.nodes = vec![
            CanvasNode::new("n1", "selectHost", "主机", r#"{"hostIds":["h1"]}"#),
            CanvasNode::new("n2", "command", "重启", "systemctl restart app"),
        ];
        wf.connections = vec![Connection {
            id: "e1".into(),
            from_id: "n1".into(),
            to_id: "n2".into(),
            source_handle: None,
        }];
        wf.settings = WorkflowSettings {
            default_node_interval_seconds: 2.0,
        };
        wf.status = WorkflowStatus::Ready;
        update_workflow(db, &wf).unwrap();
        let back = get_workflow(db, &wf.id).unwrap();
        assert_eq!(
            (back.nodes, back.connections, back.settings, back.status),
            (
                wf.nodes.clone(),
                wf.connections.clone(),
                wf.settings,
                WorkflowStatus::Ready
            )
        );
        assert_eq!(list_workflows(db).unwrap().len(), 1);
        delete_workflow(db, &wf.id).unwrap();
        assert!(list_workflows(db).unwrap().is_empty());
    }

    #[test]
    fn damaged_rows_stay_visible_and_empty() {
        let core = core();
        let wf = create_workflow(&core.db, "坏数据", "").unwrap();
        core.db
            .pool
            .get()
            .unwrap()
            .execute(
                "UPDATE workflows SET nodes='oops', connections='{' WHERE id=?1",
                params![wf.id],
            )
            .unwrap();
        let got = list_workflows(&core.db).unwrap();
        assert_eq!(
            (got.len(), got[0].nodes.len(), got[0].connections.len()),
            (1, 0, 0)
        );
    }

    #[test]
    fn templates_keep_their_text_and_sort_by_name() {
        let core = core();
        let payload = stringify_nodes_payload(
            &[CanvasNode::new("a", "start", "s", "")],
            WorkflowSettings::default(),
        );
        let b = save_template(&core.db, "B 模板", "d", &payload, "[]").unwrap();
        save_template(&core.db, "A 模板", "", &payload, "[]").unwrap();
        let list = list_templates(&core.db).unwrap();
        assert_eq!(
            list.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
            ["A 模板", "B 模板"]
        );
        assert_eq!(list[1].nodes, payload);
        delete_template(&core.db, &b).unwrap();
        assert_eq!(list_templates(&core.db).unwrap().len(), 1);
    }

    #[test]
    fn scheduled_tasks_crud_and_due_rules() {
        let core = core();
        let wf = create_workflow(&core.db, "w", "").unwrap();
        let id = add_scheduled_task(&core.db, "每分钟", "every:60", &wf.id).unwrap();
        let t = &list_scheduled_tasks(&core.db).unwrap()[0];
        assert_eq!((t.enabled, t.last_run_at.as_deref()), (true, None));
        update_scheduled_task(&core.db, &id, "每分钟", "every:60", &wf.id, false).unwrap();
        assert!(!list_scheduled_tasks(&core.db).unwrap()[0].enabled);

        let now =
            NaiveDateTime::parse_from_str("2026-01-01 12:00:00", "%Y-%m-%d %H:%M:%S").unwrap();
        let task = |cron: &str, last: Option<&str>, enabled: bool| ScheduledTask {
            id: "i".into(),
            name: "n".into(),
            cron: cron.into(),
            workflow_id: "w".into(),
            enabled,
            last_run_at: last.map(str::to_string),
            next_run_at: None,
            created_at: String::new(),
        };
        let due = |t: ScheduledTask| !due_tasks(&[t], now).is_empty();
        assert!(due(task("every:60", None, true)), "never ran");
        assert!(due(task("every:60", Some("2026-01-01 11:58:59"), true)));
        assert!(
            !due(task("every:60", Some("2026-01-01 11:59:30"), true)),
            "ran 30s ago"
        );
        assert!(!due(task("every:60", None, false)), "disabled");
        assert!(!due(task("every:0", None, true)) && !due(task("every:x", None, true)));
        assert!(
            due(task("every:60", Some("garbage"), true)),
            "unreadable timestamps count as due"
        );
        assert!(
            !due(task("0 * * * *", Some("2026-01-01 11:30:00"), true)),
            "unknown rules are hourly"
        );
        assert_eq!(
            due_tasks(&[task("every:1", None, true)], now),
            vec!["i: n (到期)"]
        );

        mark_task_ran(&core.db, &id).unwrap();
        assert!(list_scheduled_tasks(&core.db).unwrap()[0]
            .last_run_at
            .is_some());
        delete_scheduled_task(&core.db, &id).unwrap();
        assert!(list_scheduled_tasks(&core.db).unwrap().is_empty());
        assert_eq!(check_scheduled_tasks(&core.db).unwrap().len(), 0);
    }

    #[test]
    fn interval_rules() {
        assert_eq!(interval_seconds("every:60"), 60);
        assert_eq!(interval_seconds("every: 90 "), 90);
        assert_eq!(interval_seconds("every:abc"), 0);
        assert_eq!(interval_seconds("0 * * * *"), 3600);
    }
}
