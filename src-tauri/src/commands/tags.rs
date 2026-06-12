use crate::db::Database;
use rusqlite::params;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tag {
    pub id: String,
    pub name: String,
    pub color: String,
}

fn map_tag_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Tag> {
    Ok(Tag {
        id: row.get(0)?,
        name: row.get(1)?,
        color: row.get(2)?,
    })
}

fn get_tag_by_id(conn: &rusqlite::Connection, id: &str) -> Result<Tag, String> {
    conn.query_row(
        "SELECT id, name, color FROM tags WHERE id=?1",
        params![id],
        map_tag_row,
    )
    .map_err(|e| format!("tag {} not found: {}", id, e))
}

#[tauri::command]
pub async fn list_tags(db: tauri::State<'_, Database>) -> Result<Vec<Tag>, String> {
    let conn = db.pool.clone();
    tokio::task::spawn_blocking(move || {
        let conn = conn.get().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare("SELECT id, name, color FROM tags ORDER BY name")
            .map_err(|e| e.to_string())?;
        let tags = stmt
            .query_map([], map_tag_row)
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        Ok(tags)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn add_tag(
    db: tauri::State<'_, Database>,
    name: String,
    color: String,
) -> Result<Tag, String> {
    let conn = db.pool.clone();
    tokio::task::spawn_blocking(move || {
        let id = uuid::Uuid::new_v4().to_string();
        let conn = conn.get().map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO tags (id, name, color) VALUES (?1, ?2, ?3)",
            params![id, name, color],
        )
        .map_err(|e| e.to_string())?;
        get_tag_by_id(&conn, &id)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn update_tag(db: tauri::State<'_, Database>, tag: Tag) -> Result<Tag, String> {
    let conn = db.pool.clone();
    tokio::task::spawn_blocking(move || {
        let conn = conn.get().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE tags SET name=?1, color=?2 WHERE id=?3",
            params![tag.name, tag.color, tag.id],
        )
        .map_err(|e| e.to_string())?;
        get_tag_by_id(&conn, &tag.id)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn delete_tag(db: tauri::State<'_, Database>, id: String) -> Result<(), String> {
    let conn = db.pool.clone();
    tokio::task::spawn_blocking(move || {
        let conn = conn.get().map_err(|e| e.to_string())?;
        conn.execute("DELETE FROM tags WHERE id=?1", params![id])
            .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}
