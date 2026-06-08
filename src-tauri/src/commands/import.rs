use crate::db::Database;
use rusqlite::params;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ImportResult {
    pub total: u32,
    pub imported: u32,
    pub skipped: u32,
    pub errors: Vec<String>,
}

#[tauri::command]
pub fn import_hosts_csv(
    db: tauri::State<'_, Database>,
    csv_content: String,
    mode: String,
) -> Result<ImportResult, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;

    if mode == "overwrite" {
        conn.execute("DELETE FROM hosts", [])
            .map_err(|e| e.to_string())?;
    }

    let mut result = ImportResult {
        total: 0,
        imported: 0,
        skipped: 0,
        errors: Vec::new(),
    };

    let lines: Vec<&str> = csv_content.lines().collect();
    if lines.is_empty() {
        return Err("CSV file is empty".to_string());
    }

    // Skip header line
    for (i, line) in lines.iter().enumerate().skip(1) {
        result.total += 1;
        let fields: Vec<&str> = line.split(',').collect();

        let name = fields.get(0).unwrap_or(&"").trim();
        let ip = fields.get(1).unwrap_or(&"").trim();

        if name.is_empty() || ip.is_empty() {
            result
                .errors
                .push(format!("line {}: name or ip is empty", i + 1));
            result.skipped += 1;
            continue;
        }

        // Check duplicate
        let exists: bool = match mode.as_str() {
            "incremental" => {
                conn.query_row(
                    "SELECT COUNT(*) FROM hosts WHERE ip=?1",
                    params![ip],
                    |row| row.get::<_, i32>(0),
                )
                .unwrap_or(0)
                    > 0
            }
            _ => false,
        };

        if exists {
            result.skipped += 1;
            continue;
        }

        let port: i32 = fields
            .get(2)
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(22);
        let auth_type = fields
            .get(3)
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| "password".into());
        let os = fields
            .get(4)
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| "linux".into());
        let tags = fields
            .get(5)
            .map(|s| format!("[{}]", s.trim()))
            .unwrap_or_else(|| "[]".into());
        let group = fields.get(6).map(|s| s.trim().to_string());
        let remark = fields
            .get(7)
            .map(|s| s.trim().to_string())
            .unwrap_or_default();

        let id = uuid::Uuid::new_v4().to_string();
        match conn.execute(
            "INSERT INTO hosts (id, name, ip, port, auth_type, os, tags, group_id, remark) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![id, name, ip, port, auth_type, os, tags, group, remark],
        ) {
            Ok(_) => result.imported += 1,
            Err(e) => {
                result.errors.push(format!("line {}: {}", i + 1, e));
                result.skipped += 1;
            }
        }
    }

    Ok(result)
}

#[tauri::command]
pub fn export_hosts_csv(db: tauri::State<'_, Database>) -> Result<String, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT name, ip, port, auth_type, os, tags, group_id, remark FROM hosts ORDER BY name",
        )
        .map_err(|e| e.to_string())?;

    let mut csv = String::from("name,ip,port,auth_type,os,tags,group_id,remark\n");
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i32>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, String>(7)?,
            ))
        })
        .map_err(|e| e.to_string())?;

    for row in rows {
        let (name, ip, port, auth_type, os, tags, group_id, remark) =
            row.map_err(|e| e.to_string())?;
        csv.push_str(&format!(
            "{},{},{},{},{},{},{},{}\n",
            name,
            ip,
            port,
            auth_type,
            os,
            tags,
            group_id.unwrap_or_default(),
            remark,
        ));
    }

    Ok(csv)
}
