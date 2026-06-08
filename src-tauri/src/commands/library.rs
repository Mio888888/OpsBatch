use crate::db::Database;
use rusqlite::params;
use serde::{Deserialize, Serialize};

// ===== Custom Commands =====

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomCommand {
    pub id: String,
    pub name: String,
    pub command: String,
    pub category: String,
    pub tags: String,
    pub risk: String,
    pub description: String,
    pub platform: String,
    pub parameters: String,
    pub url: String,
    pub starred: bool,
    pub is_builtin: bool,
}

#[tauri::command]
pub async fn list_commands(db: tauri::State<'_, Database>) -> Result<Vec<CustomCommand>, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn.prepare(
        "SELECT id, name, command, category, tags, risk, description, platform, parameters, url, starred, is_builtin FROM commands ORDER BY is_builtin DESC, name"
    ).map_err(|e| e.to_string())?;
    let commands = stmt
        .query_map([], |row| {
            Ok(CustomCommand {
                id: row.get(0)?,
                name: row.get(1)?,
                command: row.get(2)?,
                category: row.get(3)?,
                tags: row.get(4)?,
                risk: row.get(5)?,
                description: row.get(6)?,
                platform: row.get(7)?,
                parameters: row.get::<_, Option<String>>(8)?.unwrap_or_else(|| "[]".to_string()),
                url: row.get::<_, Option<String>>(9)?.unwrap_or_default(),
                starred: row.get::<_, i32>(10)? == 1,
                is_builtin: row.get::<_, i32>(11)? == 1,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(commands)
}

#[derive(Deserialize)]
pub struct NewCommand {
    pub name: String,
    pub command: String,
    pub category: Option<String>,
    pub tags: Option<String>,
    pub risk: Option<String>,
    pub description: Option<String>,
    pub platform: Option<String>,
    pub parameters: Option<String>,
    pub url: Option<String>,
}

#[tauri::command]
pub async fn add_command(
    db: tauri::State<'_, Database>,
    command: NewCommand,
) -> Result<String, String> {
    let id = uuid::Uuid::new_v4().to_string();
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT INTO commands (id, name, command, category, tags, risk, description, platform, parameters, url, starred, is_builtin) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 0, 0)",
        params![
            id,
            command.name,
            command.command,
            command.category.unwrap_or_default(),
            command.tags.unwrap_or_else(|| "[]".to_string()),
            command.risk.unwrap_or_else(|| "low".to_string()),
            command.description.unwrap_or_default(),
            command.platform.unwrap_or_else(|| "linux".to_string()),
            command.parameters.unwrap_or_else(|| "[]".to_string()),
            command.url.unwrap_or_default(),
        ],
    ).map_err(|e| e.to_string())?;
    Ok(id)
}

#[tauri::command]
pub async fn update_command(
    db: tauri::State<'_, Database>,
    command: CustomCommand,
) -> Result<(), String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    conn.execute(
        "UPDATE commands SET name=?1, command=?2, category=?3, tags=?4, risk=?5, description=?6, platform=?7, parameters=?8, url=?9, starred=?10 WHERE id=?11",
        params![
            command.name, command.command, command.category, command.tags,
            command.risk, command.description, command.platform, command.parameters,
            command.url, command.starred as i32, command.id
        ],
    ).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn delete_command(db: tauri::State<'_, Database>, id: String) -> Result<(), String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    conn.execute(
        "DELETE FROM commands WHERE id=?1 AND is_builtin=0",
        params![id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn toggle_star_command(db: tauri::State<'_, Database>, id: String) -> Result<(), String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    conn.execute(
        "UPDATE commands SET starred = CASE WHEN starred=1 THEN 0 ELSE 1 END WHERE id=?1",
        params![id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

// ===== Custom Scripts =====

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomScript {
    pub id: String,
    pub name: String,
    pub language: String,
    pub category: String,
    pub tags: String,
    pub risk: String,
    pub description: String,
    pub content: String,
    pub parameters: String,
    pub url: String,
    pub platform: String,
    pub starred: bool,
    pub is_builtin: bool,
}

#[tauri::command]
pub async fn list_scripts(db: tauri::State<'_, Database>) -> Result<Vec<CustomScript>, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn.prepare(
        "SELECT id, name, language, category, tags, risk, description, content, parameters, url, platform, starred, is_builtin FROM scripts ORDER BY is_builtin DESC, name"
    ).map_err(|e| e.to_string())?;
    let scripts = stmt
        .query_map([], |row| {
            Ok(CustomScript {
                id: row.get(0)?,
                name: row.get(1)?,
                language: row.get(2)?,
                category: row.get(3)?,
                tags: row.get(4)?,
                risk: row.get(5)?,
                description: row.get(6)?,
                content: row.get(7)?,
                parameters: row.get(8)?,
                url: row.get::<_, Option<String>>(9)?.unwrap_or_default(),
                platform: row.get(10)?,
                starred: row.get::<_, i32>(11)? == 1,
                is_builtin: row.get::<_, i32>(12)? == 1,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(scripts)
}

#[derive(Deserialize)]
pub struct NewScript {
    pub name: String,
    pub language: Option<String>,
    pub category: Option<String>,
    pub tags: Option<String>,
    pub risk: Option<String>,
    pub description: Option<String>,
    pub content: Option<String>,
    pub parameters: Option<String>,
    pub url: Option<String>,
    pub platform: Option<String>,
}

#[tauri::command]
pub async fn add_script(
    db: tauri::State<'_, Database>,
    script: NewScript,
) -> Result<String, String> {
    let id = uuid::Uuid::new_v4().to_string();
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT INTO scripts (id, name, language, category, tags, risk, description, content, parameters, url, platform, starred, is_builtin) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 0, 0)",
        params![
            id,
            script.name,
            script.language.unwrap_or_else(|| "shell".to_string()),
            script.category.unwrap_or_default(),
            script.tags.unwrap_or_else(|| "[]".to_string()),
            script.risk.unwrap_or_else(|| "low".to_string()),
            script.description.unwrap_or_default(),
            script.content.unwrap_or_default(),
            script.parameters.unwrap_or_else(|| "[]".to_string()),
            script.url.unwrap_or_default(),
            script.platform.unwrap_or_else(|| "linux".to_string()),
        ],
    ).map_err(|e| e.to_string())?;
    Ok(id)
}

#[tauri::command]
pub async fn update_script(
    db: tauri::State<'_, Database>,
    script: CustomScript,
) -> Result<(), String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    conn.execute(
        "UPDATE scripts SET name=?1, language=?2, category=?3, tags=?4, risk=?5, description=?6, content=?7, parameters=?8, url=?9, platform=?10, starred=?11 WHERE id=?12",
        params![
            script.name, script.language, script.category, script.tags,
            script.risk, script.description, script.content, script.parameters,
            script.url, script.platform, script.starred as i32, script.id
        ],
    ).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn delete_script(db: tauri::State<'_, Database>, id: String) -> Result<(), String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    conn.execute(
        "DELETE FROM scripts WHERE id=?1 AND is_builtin=0",
        params![id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn toggle_star_script(db: tauri::State<'_, Database>, id: String) -> Result<(), String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    conn.execute(
        "UPDATE scripts SET starred = CASE WHEN starred=1 THEN 0 ELSE 1 END WHERE id=?1",
        params![id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

// ===== Script Versions =====

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScriptVersion {
    pub id: String,
    pub script_id: String,
    pub content: String,
    pub label: String,
    pub created_at: String,
}

#[tauri::command]
pub async fn list_script_versions(
    db: tauri::State<'_, Database>,
    script_id: String,
) -> Result<Vec<ScriptVersion>, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn.prepare(
        "SELECT id, script_id, content, label, created_at FROM script_versions WHERE script_id=?1 ORDER BY created_at DESC"
    ).map_err(|e| e.to_string())?;
    let versions = stmt
        .query_map(params![script_id], |row| {
            Ok(ScriptVersion {
                id: row.get(0)?,
                script_id: row.get(1)?,
                content: row.get(2)?,
                label: row.get(3)?,
                created_at: row.get(4)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(versions)
}

#[tauri::command]
pub async fn save_script_version(
    db: tauri::State<'_, Database>,
    script_id: String,
    content: String,
    label: String,
) -> Result<String, String> {
    let id = uuid::Uuid::new_v4().to_string();
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT INTO script_versions (id, script_id, content, label) VALUES (?1, ?2, ?3, ?4)",
        params![id, script_id, content, label],
    )
    .map_err(|e| e.to_string())?;
    Ok(id)
}
