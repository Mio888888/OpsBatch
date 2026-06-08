use crate::db::Database;
use rusqlite::params;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuickAction {
    pub id: String,
    pub name: String,
    pub command: String,
    pub category: String,
    pub parameters: String,
    pub sort_order: i32,
    pub starred: i32,
    pub description: String,
    pub tags: String,
    pub language: String,
    pub last_run_at: String,
    pub last_status: String,
}

#[tauri::command]
pub async fn list_quick_actions(
    db: tauri::State<'_, Database>,
) -> Result<Vec<QuickAction>, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn.prepare(
        "SELECT id, name, command, category, parameters, sort_order, starred, description, tags, language, last_run_at, last_status FROM quick_actions ORDER BY sort_order, id"
    ).map_err(|e| e.to_string())?;
    let actions = stmt
        .query_map([], |row| {
            Ok(QuickAction {
                id: row.get(0)?,
                name: row.get(1)?,
                command: row.get(2)?,
                category: row.get(3)?,
                parameters: row.get(4)?,
                sort_order: row.get(5)?,
                starred: row.get(6)?,
                description: row.get(7)?,
                tags: row.get(8)?,
                language: row.get(9)?,
                last_run_at: row.get(10)?,
                last_status: row.get(11)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(actions)
}

#[derive(Deserialize)]
pub struct NewQuickAction {
    pub name: String,
    pub command: String,
    pub category: Option<String>,
    pub parameters: Option<String>,
    pub sort_order: Option<i32>,
    pub description: Option<String>,
    pub tags: Option<String>,
    pub language: Option<String>,
}

#[tauri::command]
pub async fn add_quick_action(
    db: tauri::State<'_, Database>,
    action: NewQuickAction,
) -> Result<String, String> {
    let id = uuid::Uuid::new_v4().to_string();
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT INTO quick_actions (id, name, command, category, parameters, sort_order, starred, description, tags, language, last_run_at, last_status) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, ?7, ?8, ?9, '', '')",
        params![
            id,
            action.name,
            action.command,
            action.category.unwrap_or_default(),
            action.parameters.unwrap_or_else(|| "[]".to_string()),
            action.sort_order.unwrap_or(0),
            action.description.unwrap_or_default(),
            action.tags.unwrap_or_else(|| "[]".to_string()),
            action.language.unwrap_or_else(|| "shell".to_string()),
        ],
    ).map_err(|e| e.to_string())?;
    Ok(id)
}

#[tauri::command]
pub async fn update_quick_action(
    db: tauri::State<'_, Database>,
    action: QuickAction,
) -> Result<(), String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    conn.execute(
        "UPDATE quick_actions SET name=?1, command=?2, category=?3, parameters=?4, sort_order=?5, description=?6, tags=?7, language=?8 WHERE id=?9",
        params![action.name, action.command, action.category, action.parameters, action.sort_order, action.description, action.tags, action.language, action.id],
    ).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn delete_quick_action(db: tauri::State<'_, Database>, id: String) -> Result<(), String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM quick_actions WHERE id=?1", params![id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn reorder_quick_actions(
    db: tauri::State<'_, Database>,
    ids: Vec<String>,
) -> Result<(), String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    for (i, id) in ids.iter().enumerate() {
        conn.execute(
            "UPDATE quick_actions SET sort_order=?1 WHERE id=?2",
            params![i as i32, id],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub async fn toggle_star_quick_action(
    db: tauri::State<'_, Database>,
    id: String,
) -> Result<(), String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    conn.execute(
        "UPDATE quick_actions SET starred = CASE WHEN starred=1 THEN 0 ELSE 1 END WHERE id=?1",
        params![id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}
