use crate::db::Database;
use rusqlite::params;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostGroup {
    pub id: String,
    pub name: String,
    pub parent_id: Option<String>,
    pub sort_order: i32,
}

fn map_group_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<HostGroup> {
    Ok(HostGroup {
        id: row.get(0)?,
        name: row.get(1)?,
        parent_id: row.get(2)?,
        sort_order: row.get(3)?,
    })
}

fn get_group_by_id(conn: &rusqlite::Connection, id: &str) -> Result<HostGroup, String> {
    conn.query_row(
        "SELECT id, name, parent_id, sort_order FROM host_groups WHERE id=?1",
        params![id],
        map_group_row,
    )
    .map_err(|e| format!("group {} not found: {}", id, e))
}

#[tauri::command]
pub async fn list_groups(db: tauri::State<'_, Database>) -> Result<Vec<HostGroup>, String> {
    let conn = db.pool.clone();
    tokio::task::spawn_blocking(move || {
        let conn = conn.get().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT id, name, parent_id, sort_order FROM host_groups ORDER BY sort_order, name",
            )
            .map_err(|e| e.to_string())?;
        let groups = stmt
            .query_map([], map_group_row)
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        Ok(groups)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn add_group(
    db: tauri::State<'_, Database>,
    name: String,
    parent_id: Option<String>,
) -> Result<HostGroup, String> {
    let conn = db.pool.clone();
    tokio::task::spawn_blocking(move || {
        let id = uuid::Uuid::new_v4().to_string();
        let conn = conn.get().map_err(|e| e.to_string())?;
        let max_order: i32 = conn
            .query_row(
                "SELECT COALESCE(MAX(sort_order), 0) FROM host_groups",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);
        conn.execute(
            "INSERT INTO host_groups (id, name, parent_id, sort_order) VALUES (?1, ?2, ?3, ?4)",
            params![id, name, parent_id, max_order + 1],
        )
        .map_err(|e| e.to_string())?;
        get_group_by_id(&conn, &id)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn update_group(
    db: tauri::State<'_, Database>,
    group: HostGroup,
) -> Result<HostGroup, String> {
    let conn = db.pool.clone();
    tokio::task::spawn_blocking(move || {
        let conn = conn.get().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE host_groups SET name=?1, parent_id=?2, sort_order=?3 WHERE id=?4",
            params![group.name, group.parent_id, group.sort_order, group.id],
        )
        .map_err(|e| e.to_string())?;
        get_group_by_id(&conn, &group.id)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn delete_group(db: tauri::State<'_, Database>, id: String) -> Result<(), String> {
    let conn = db.pool.clone();
    tokio::task::spawn_blocking(move || {
        let conn = conn.get().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE hosts SET group_id=NULL WHERE group_id=?1",
            params![id],
        )
        .map_err(|e| e.to_string())?;
        conn.execute("DELETE FROM host_groups WHERE id=?1", params![id])
            .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}
