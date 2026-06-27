use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::Connection;

use crate::security::SECRET_PLACEHOLDER;

const SCHEMA_MIGRATION_ID: &str = "opsbatch_schema";
const CURRENT_SCHEMA_VERSION: i64 = 1;

/// 内置默认仓库：首次启动（github_repos 为空）时自动注入，无需用户手动添加。
pub const DEFAULT_LIBRARY_REPO_URL: &str = "https://github.com/Mio888888/OpsBatch-Library";
pub const DEFAULT_LIBRARY_REPO_BRANCH: &str = "main";
const DEFAULT_LIBRARY_REPO_ID: &str = "opsbatch-default-library-repo";

pub struct Database {
    pub pool: Pool<SqliteConnectionManager>,
}

#[cfg(test)]
mod tests {
    use super::{
        Database, CURRENT_SCHEMA_VERSION, DEFAULT_LIBRARY_REPO_BRANCH,
        DEFAULT_LIBRARY_REPO_ID, DEFAULT_LIBRARY_REPO_URL,
    };

    fn temp_db_path(name: &str) -> std::path::PathBuf {
        let unique = format!(
            "opsbatch-{}-{}.db",
            name,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        );
        std::env::temp_dir().join(unique)
    }

    #[test]
    fn init_tables_migrates_scheduled_pull_strategy_to_startup_update() {
        let db_path = temp_db_path("startup-update-migration");
        {
            let conn = rusqlite::Connection::open(&db_path).expect("open fixture db");
            conn.execute_batch(
                "
                CREATE TABLE github_repos (
                    id TEXT PRIMARY KEY,
                    url TEXT NOT NULL,
                    branch TEXT DEFAULT 'main',
                    token TEXT,
                    pull_strategy TEXT DEFAULT 'manual',
                    last_pulled_at TEXT,
                    enabled INTEGER DEFAULT 1
                );
                INSERT INTO github_repos (id, url, pull_strategy) VALUES
                    ('manual-repo', 'https://example.com/manual.git', 'manual'),
                    ('daily-repo', 'https://example.com/daily.git', 'daily'),
                    ('weekly-repo', 'https://example.com/weekly.git', 'weekly');
                ",
            )
            .expect("seed fixture db");
        }

        let database = Database::new(&db_path).expect("open db");
        database.init_tables().expect("init tables");

        let conn = database.pool.get().expect("get db conn");
        let manual: i32 = conn
            .query_row(
                "SELECT update_on_startup FROM github_repos WHERE id='manual-repo'",
                [],
                |row| row.get(0),
            )
            .expect("manual repo");
        let daily: i32 = conn
            .query_row(
                "SELECT update_on_startup FROM github_repos WHERE id='daily-repo'",
                [],
                |row| row.get(0),
            )
            .expect("daily repo");
        let weekly: i32 = conn
            .query_row(
                "SELECT update_on_startup FROM github_repos WHERE id='weekly-repo'",
                [],
                |row| row.get(0),
            )
            .expect("weekly repo");

        assert_eq!(0, manual);
        assert_eq!(1, daily);
        assert_eq!(1, weekly);

        drop(conn);
        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn init_tables_records_current_schema_version() {
        let db_path = temp_db_path("schema-version");
        let database = Database::new(&db_path).expect("open db");

        database.init_tables().expect("init tables");

        let conn = database.pool.get().expect("get db conn");
        let version: i64 = conn
            .query_row(
                "SELECT version FROM schema_migrations WHERE id='opsbatch_schema'",
                [],
                |row| row.get(0),
            )
            .expect("schema version");

        assert_eq!(CURRENT_SCHEMA_VERSION, version);

        drop(conn);
        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn init_tables_skips_legacy_migrations_after_current_version_is_recorded() {
        let db_path = temp_db_path("schema-version-fast-path");
        {
            let conn = rusqlite::Connection::open(&db_path).expect("open fixture db");
            conn.execute_batch(
                "
                CREATE TABLE schema_migrations (
                    id TEXT PRIMARY KEY,
                    version INTEGER NOT NULL,
                    applied_at TEXT DEFAULT (datetime('now', 'localtime'))
                );
                CREATE TABLE github_repos (
                    id TEXT PRIMARY KEY,
                    url TEXT NOT NULL,
                    branch TEXT DEFAULT 'main',
                    token TEXT,
                    pull_strategy TEXT DEFAULT 'manual',
                    last_pulled_at TEXT,
                    enabled INTEGER DEFAULT 1
                );
                INSERT INTO github_repos (id, url, pull_strategy) VALUES
                    ('daily-repo', 'https://example.com/daily.git', 'daily');
                ",
            )
            .expect("seed fixture db");
            conn.execute(
                "INSERT INTO schema_migrations (id, version) VALUES ('opsbatch_schema', ?1)",
                rusqlite::params![CURRENT_SCHEMA_VERSION],
            )
            .expect("seed current schema version");
        }

        let database = Database::new(&db_path).expect("open db");
        database.init_tables().expect("init tables");

        let conn = database.pool.get().expect("get db conn");
        let version: i64 = conn
            .query_row(
                "SELECT version FROM schema_migrations WHERE id='opsbatch_schema'",
                [],
                |row| row.get(0),
            )
            .expect("schema version");
        assert_eq!(CURRENT_SCHEMA_VERSION, version);

        let has_update_on_startup: i32 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('github_repos') WHERE name='update_on_startup'",
                [],
                |row| row.get(0),
            )
            .expect("column check");
        assert_eq!(0, has_update_on_startup);

        drop(conn);
        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn init_tables_fast_path_stays_lightweight_after_schema_is_current() {
        let db_path = temp_db_path("schema-version-timing");
        let database = Database::new(&db_path).expect("open db");

        database.init_tables().expect("first init");

        let started_at = std::time::Instant::now();
        database.init_tables().expect("second init");
        let elapsed = started_at.elapsed();

        assert!(
            elapsed < std::time::Duration::from_millis(50),
            "current schema init should only check migration version, elapsed: {:?}",
            elapsed
        );

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn init_tables_seeds_default_library_repo_on_empty_database() {
        let db_path = temp_db_path("default-repo-seed");
        let database = Database::new(&db_path).expect("open db");

        database.init_tables().expect("init tables");

        let conn = database.pool.get().expect("get db conn");
        let (url, branch, enabled, update_on_startup): (String, String, i32, i32) = conn
            .query_row(
                "SELECT url, branch, enabled, update_on_startup FROM github_repos WHERE id=?1",
                rusqlite::params![DEFAULT_LIBRARY_REPO_ID],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .expect("default library repo");

        assert_eq!(url, DEFAULT_LIBRARY_REPO_URL);
        assert_eq!(branch, DEFAULT_LIBRARY_REPO_BRANCH);
        assert_eq!(enabled, 1, "default repo should be enabled");
        assert_eq!(
            update_on_startup, 1,
            "default repo should update on startup"
        );

        drop(conn);
        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn init_tables_does_not_seed_default_repo_when_user_has_repos() {
        let db_path = temp_db_path("default-repo-skip");
        {
            let conn = rusqlite::Connection::open(&db_path).expect("open fixture db");
            conn.execute_batch(
                "
                CREATE TABLE github_repos (
                    id TEXT PRIMARY KEY,
                    url TEXT NOT NULL,
                    branch TEXT DEFAULT 'main',
                    token TEXT,
                    last_pulled_at TEXT,
                    update_on_startup INTEGER DEFAULT 0,
                    enabled INTEGER DEFAULT 1
                );
                INSERT INTO github_repos (id, url) VALUES ('user-repo', 'https://example.com/lib.git');
                ",
            )
            .expect("seed fixture db");
        }

        let database = Database::new(&db_path).expect("open db");
        database.init_tables().expect("init tables");

        let conn = database.pool.get().expect("get db conn");
        let count: i32 = conn
            .query_row(
                "SELECT COUNT(*) FROM github_repos WHERE id=?1",
                rusqlite::params![DEFAULT_LIBRARY_REPO_ID],
                |row| row.get(0),
            )
            .unwrap_or(0);
        assert_eq!(
            0, count,
            "default repo must not be injected when the user already has repos"
        );

        drop(conn);
        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn init_tables_re_seed_default_repo_idempotent() {
        let db_path = temp_db_path("default-repo-idempotent");
        let database = Database::new(&db_path).expect("open db");

        database.init_tables().expect("first init");
        database.init_tables().expect("second init");

        let conn = database.pool.get().expect("get db conn");
        let count: i32 = conn
            .query_row(
                "SELECT COUNT(*) FROM github_repos WHERE id=?1",
                rusqlite::params![DEFAULT_LIBRARY_REPO_ID],
                |row| row.get(0),
            )
            .unwrap_or(0);
        assert_eq!(1, count, "default repo should not be duplicated on re-init");

        drop(conn);
        let _ = std::fs::remove_file(db_path);
    }
}

fn migrate_json_secret_to_keychain<F>(
    id: &str,
    settings: Option<String>,
    field: &str,
    store: F,
) -> Result<Option<String>, String>
where
    F: FnOnce(&str, &str) -> Result<(), String>,
{
    let Some(raw) = settings else {
        return Ok(None);
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }

    let mut value: serde_json::Value =
        serde_json::from_str(trimmed).map_err(|e| format!("settings JSON 格式无效: {}", e))?;
    let Some(object) = value.as_object_mut() else {
        return Ok(None);
    };
    let Some(secret) = object
        .get(field)
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty() && *value != SECRET_PLACEHOLDER)
        .map(str::to_string)
    else {
        return Ok(None);
    };

    store(id, &secret)?;
    object.insert(
        field.to_string(),
        serde_json::Value::String(SECRET_PLACEHOLDER.to_string()),
    );
    serde_json::to_string(&value)
        .map(Some)
        .map_err(|e| e.to_string())
}

#[derive(Debug)]
struct DbConnectionCustomizer;

impl r2d2::CustomizeConnection<Connection, rusqlite::Error> for DbConnectionCustomizer {
    fn on_acquire(&self, conn: &mut Connection) -> Result<(), rusqlite::Error> {
        conn.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000;",
        )
    }
}

impl Database {
    pub fn new(db_path: &std::path::PathBuf) -> Result<Self, String> {
        let manager = SqliteConnectionManager::file(db_path);
        let pool = Pool::builder()
            .max_size(8)
            .connection_customizer(Box::new(DbConnectionCustomizer))
            .build(manager)
            .map_err(|e| e.to_string())?;
        Ok(Self { pool })
    }

    pub fn init_tables(&self) -> Result<(), String> {
        let conn = self.pool.get().map_err(|e| e.to_string())?;
        Self::ensure_schema_migrations_table(&conn)?;
        // 内置默认仓库的注入与 schema 版本无关：只要 github_repos 为空就补齐，
        // 因此放在快速路径之前，确保已初始化的旧库也能拿到默认仓库。
        Self::ensure_default_library_repo(&conn)?;
        if Self::applied_schema_version(&conn)? >= CURRENT_SCHEMA_VERSION {
            return Ok(());
        }

        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS hosts (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                ip TEXT NOT NULL,
                port INTEGER DEFAULT 22,
                auth_type TEXT DEFAULT 'password',
                username TEXT DEFAULT 'root',
                password TEXT,
                private_key TEXT,
                os TEXT DEFAULT 'linux',
                tags TEXT DEFAULT '[]',
                group_id TEXT,
                remark TEXT DEFAULT '',
                status TEXT DEFAULT 'unknown',
                rdp_settings TEXT DEFAULT '{}',
                proxy_settings TEXT DEFAULT '{}',
                created_at TEXT DEFAULT (datetime('now', 'localtime')),
                updated_at TEXT DEFAULT (datetime('now', 'localtime'))
            );

            CREATE TABLE IF NOT EXISTS host_groups (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                parent_id TEXT,
                sort_order INTEGER DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS tags (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL UNIQUE,
                color TEXT DEFAULT '#1677ff',
                icon TEXT DEFAULT ''
            );

            CREATE TABLE IF NOT EXISTS execution_history (
                id TEXT PRIMARY KEY,
                command TEXT NOT NULL,
                host_ids TEXT NOT NULL,
                host_count INTEGER,
                success_count INTEGER DEFAULT 0,
                fail_count INTEGER DEFAULT 0,
                started_at TEXT,
                completed_at TEXT,
                duration INTEGER DEFAULT 0,
                quick_action_id TEXT
            );

            CREATE TABLE IF NOT EXISTS commands (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                command TEXT NOT NULL,
                category TEXT DEFAULT '',
                tags TEXT DEFAULT '[]',
                risk TEXT DEFAULT 'low',
                description TEXT DEFAULT '',
                platform TEXT DEFAULT 'linux',
                starred INTEGER DEFAULT 0,
                is_builtin INTEGER DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS scripts (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                language TEXT DEFAULT 'shell',
                category TEXT DEFAULT '',
                tags TEXT DEFAULT '[]',
                risk TEXT DEFAULT 'low',
                description TEXT DEFAULT '',
                content TEXT DEFAULT '',
                parameters TEXT DEFAULT '[]',
                platform TEXT DEFAULT 'linux',
                starred INTEGER DEFAULT 0,
                is_builtin INTEGER DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS quick_actions (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                command TEXT NOT NULL,
                category TEXT DEFAULT '',
                shortcut TEXT,
                parameters TEXT DEFAULT '[]',
                sort_order INTEGER DEFAULT 0,
                starred INTEGER DEFAULT 0,
                description TEXT DEFAULT '',
                tags TEXT DEFAULT '[]',
                language TEXT DEFAULT 'shell',
                last_run_at TEXT DEFAULT '',
                last_status TEXT DEFAULT ''
            );

            CREATE TABLE IF NOT EXISTS github_repos (
                id TEXT PRIMARY KEY,
                url TEXT NOT NULL,
                branch TEXT DEFAULT 'main',
                token TEXT,
                last_pulled_at TEXT,
                update_on_startup INTEGER DEFAULT 0,
                enabled INTEGER DEFAULT 1
            );

            CREATE TABLE IF NOT EXISTS danger_rules (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                pattern TEXT NOT NULL,
                enabled INTEGER DEFAULT 1,
                is_builtin INTEGER DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS execution_details (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                history_id TEXT NOT NULL,
                host_id TEXT NOT NULL,
                host_name TEXT DEFAULT '',
                status TEXT DEFAULT 'pending',
                output TEXT DEFAULT '',
                exit_code INTEGER DEFAULT 0,
                duration INTEGER DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS execution_cancellations (
                task_id TEXT PRIMARY KEY,
                cancelled_at TEXT DEFAULT (datetime('now', 'localtime'))
            );

            CREATE TABLE IF NOT EXISTS script_versions (
                id TEXT PRIMARY KEY,
                script_id TEXT NOT NULL,
                content TEXT NOT NULL,
                label TEXT NOT NULL,
                created_at TEXT DEFAULT (datetime('now', 'localtime'))
            );

            CREATE TABLE IF NOT EXISTS workflows (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                description TEXT DEFAULT '',
                nodes TEXT DEFAULT '[]',
                connections TEXT DEFAULT '[]',
                status TEXT DEFAULT 'draft',
                created_at TEXT DEFAULT (datetime('now', 'localtime')),
                updated_at TEXT DEFAULT (datetime('now', 'localtime'))
            );

            CREATE TABLE IF NOT EXISTS general_settings (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS settings (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS workflow_templates (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                description TEXT DEFAULT '',
                nodes TEXT DEFAULT '[]',
                connections TEXT DEFAULT '[]',
                created_at TEXT DEFAULT (datetime('now', 'localtime'))
            );

            CREATE TABLE IF NOT EXISTS scheduled_tasks (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                cron TEXT NOT NULL,
                workflow_id TEXT NOT NULL,
                enabled INTEGER DEFAULT 1,
                last_run_at TEXT,
                next_run_at TEXT,
                created_at TEXT DEFAULT (datetime('now', 'localtime'))
            );

            -- Migration: drop credential_id from hosts table (if exists)
            -- SQLite doesn't support DROP COLUMN before 3.35.0, so use table rebuild

            CREATE TABLE IF NOT EXISTS ai_conversations (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL DEFAULT '',
                scope TEXT NOT NULL DEFAULT 'global',
                scope_id TEXT DEFAULT '',
                model TEXT DEFAULT '',
                created_at TEXT DEFAULT (datetime('now', 'localtime')),
                updated_at TEXT DEFAULT (datetime('now', 'localtime'))
            );

            CREATE TABLE IF NOT EXISTS ai_messages (
                id TEXT PRIMARY KEY,
                conversation_id TEXT NOT NULL,
                role TEXT NOT NULL,
                content TEXT NOT NULL DEFAULT '',
                model TEXT DEFAULT '',
                tokens_used INTEGER DEFAULT 0,
                created_at TEXT DEFAULT (datetime('now', 'localtime')),
                FOREIGN KEY (conversation_id) REFERENCES ai_conversations(id) ON DELETE CASCADE
            );

            CREATE INDEX IF NOT EXISTS idx_ai_messages_conversation ON ai_messages(conversation_id, created_at);

            CREATE TABLE IF NOT EXISTS ai_action_audit (
                id TEXT PRIMARY KEY,
                conversation_id TEXT DEFAULT '',
                session_id TEXT DEFAULT '',
                action_id TEXT NOT NULL,
                event TEXT NOT NULL,
                command TEXT NOT NULL,
                decision TEXT NOT NULL,
                risk_level TEXT NOT NULL,
                risk_score INTEGER NOT NULL,
                matched_rule TEXT DEFAULT '',
                reason TEXT DEFAULT '',
                host TEXT DEFAULT '',
                created_at TEXT DEFAULT (datetime('now', 'localtime'))
            );

            CREATE INDEX IF NOT EXISTS idx_ai_action_audit_action ON ai_action_audit(action_id, created_at);
            CREATE INDEX IF NOT EXISTS idx_execution_history_started ON execution_history(started_at DESC);
            CREATE INDEX IF NOT EXISTS idx_execution_details_history ON execution_details(history_id, host_name);
            ",
        )
        .map_err(|e| e.to_string())?;

        // Migrate: remove credential_id column from hosts if it still exists
        let has_credential_id: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('hosts') WHERE name='credential_id'",
                [],
                |row| row.get::<_, i32>(0),
            )
            .unwrap_or(0)
            > 0;

        if has_credential_id {
            conn.execute_batch(
                "
                CREATE TABLE hosts_new (
                    id TEXT PRIMARY KEY,
                    name TEXT NOT NULL,
                    ip TEXT NOT NULL,
                    port INTEGER DEFAULT 22,
                    auth_type TEXT DEFAULT 'password',
                    username TEXT DEFAULT 'root',
                    password TEXT,
                    private_key TEXT,
                    os TEXT DEFAULT 'linux',
                    tags TEXT DEFAULT '[]',
                    group_id TEXT,
                    remark TEXT DEFAULT '',
                    status TEXT DEFAULT 'unknown',
                    rdp_settings TEXT DEFAULT '{}',
                    proxy_settings TEXT DEFAULT '{}',
                    created_at TEXT DEFAULT (datetime('now', 'localtime')),
                    updated_at TEXT DEFAULT (datetime('now', 'localtime'))
                );
                INSERT INTO hosts_new SELECT id, name, ip, port, auth_type, 'root' as username, NULL as password, NULL as private_key, os, tags, group_id, remark, status, '{}', '{}', created_at, updated_at FROM hosts;
                DROP TABLE hosts;
                ALTER TABLE hosts_new RENAME TO hosts;
                "
            ).map_err(|e| format!("migration failed: {}", e))?;
        }

        // Drop credentials table if it exists
        conn.execute_batch("DROP TABLE IF EXISTS credentials;")
            .map_err(|e| e.to_string())?;

        // Add icon column to tags if missing
        let has_icon: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('tags') WHERE name='icon'",
                [],
                |row| row.get::<_, i32>(0),
            )
            .unwrap_or(0)
            > 0;
        if !has_icon {
            conn.execute_batch("ALTER TABLE tags ADD COLUMN icon TEXT DEFAULT '';")
                .map_err(|e| e.to_string())?;
        }

        // Migrate: add username/password/private_key columns to hosts if missing
        let has_username: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('hosts') WHERE name='username'",
                [],
                |row| row.get::<_, i32>(0),
            )
            .unwrap_or(0)
            > 0;
        if !has_username {
            conn.execute_batch("ALTER TABLE hosts ADD COLUMN username TEXT DEFAULT 'root';")
                .map_err(|e| e.to_string())?;
        }

        let has_password: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('hosts') WHERE name='password'",
                [],
                |row| row.get::<_, i32>(0),
            )
            .unwrap_or(0)
            > 0;
        if !has_password {
            conn.execute_batch("ALTER TABLE hosts ADD COLUMN password TEXT;")
                .map_err(|e| e.to_string())?;
        }

        let has_private_key: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('hosts') WHERE name='private_key'",
                [],
                |row| row.get::<_, i32>(0),
            )
            .unwrap_or(0)
            > 0;
        if !has_private_key {
            conn.execute_batch("ALTER TABLE hosts ADD COLUMN private_key TEXT;")
                .map_err(|e| e.to_string())?;
        }

        // Add jump_chain column to hosts if missing
        let has_jump_chain: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('hosts') WHERE name='jump_chain'",
                [],
                |row| row.get::<_, i32>(0),
            )
            .unwrap_or(0)
            > 0;
        if !has_jump_chain {
            conn.execute_batch("ALTER TABLE hosts ADD COLUMN jump_chain TEXT DEFAULT '[]';")
                .map_err(|e| e.to_string())?;
        }

        let has_rdp_settings: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('hosts') WHERE name='rdp_settings'",
                [],
                |row| row.get::<_, i32>(0),
            )
            .unwrap_or(0)
            > 0;
        if !has_rdp_settings {
            conn.execute_batch("ALTER TABLE hosts ADD COLUMN rdp_settings TEXT DEFAULT '{}';")
                .map_err(|e| e.to_string())?;
        }

        let has_proxy_settings: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('hosts') WHERE name='proxy_settings'",
                [],
                |row| row.get::<_, i32>(0),
            )
            .unwrap_or(0)
            > 0;
        if !has_proxy_settings {
            conn.execute_batch("ALTER TABLE hosts ADD COLUMN proxy_settings TEXT DEFAULT '{}';")
                .map_err(|e| e.to_string())?;
        }

        self.migrate_host_secrets_to_keychain(&conn)?;

        // Migrate: add new columns to quick_actions if missing
        let has_qa_starred: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('quick_actions') WHERE name='starred'",
                [],
                |row| row.get::<_, i32>(0),
            )
            .unwrap_or(0)
            > 0;
        if !has_qa_starred {
            conn.execute_batch("ALTER TABLE quick_actions ADD COLUMN starred INTEGER DEFAULT 0;")
                .map_err(|e| e.to_string())?;
        }

        let has_qa_description: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('quick_actions') WHERE name='description'",
                [],
                |row| row.get::<_, i32>(0),
            )
            .unwrap_or(0)
            > 0;
        if !has_qa_description {
            conn.execute_batch("ALTER TABLE quick_actions ADD COLUMN description TEXT DEFAULT '';")
                .map_err(|e| e.to_string())?;
        }

        let has_qa_tags: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('quick_actions') WHERE name='tags'",
                [],
                |row| row.get::<_, i32>(0),
            )
            .unwrap_or(0)
            > 0;
        if !has_qa_tags {
            conn.execute_batch("ALTER TABLE quick_actions ADD COLUMN tags TEXT DEFAULT '[]';")
                .map_err(|e| e.to_string())?;
        }

        let has_qa_language: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('quick_actions') WHERE name='language'",
                [],
                |row| row.get::<_, i32>(0),
            )
            .unwrap_or(0)
            > 0;
        if !has_qa_language {
            conn.execute_batch(
                "ALTER TABLE quick_actions ADD COLUMN language TEXT DEFAULT 'shell';",
            )
            .map_err(|e| e.to_string())?;
        }

        let has_qa_last_run_at: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('quick_actions') WHERE name='last_run_at'",
                [],
                |row| row.get::<_, i32>(0),
            )
            .unwrap_or(0)
            > 0;
        if !has_qa_last_run_at {
            conn.execute_batch("ALTER TABLE quick_actions ADD COLUMN last_run_at TEXT DEFAULT '';")
                .map_err(|e| e.to_string())?;
        }

        let has_qa_last_status: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('quick_actions') WHERE name='last_status'",
                [],
                |row| row.get::<_, i32>(0),
            )
            .unwrap_or(0)
            > 0;
        if !has_qa_last_status {
            conn.execute_batch("ALTER TABLE quick_actions ADD COLUMN last_status TEXT DEFAULT '';")
                .map_err(|e| e.to_string())?;
        }

        // Migrate: add quick_action_id column to execution_history if missing
        let has_eh_qa_id: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('execution_history') WHERE name='quick_action_id'",
                [],
                |row| row.get::<_, i32>(0),
            )
            .unwrap_or(0)
            > 0;
        if !has_eh_qa_id {
            conn.execute_batch("ALTER TABLE execution_history ADD COLUMN quick_action_id TEXT;")
                .map_err(|e| e.to_string())?;
        }

        // Migrate repository sync startup-update flag from the legacy pull_strategy column.
        let has_repo_update_on_startup: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('github_repos') WHERE name='update_on_startup'",
                [],
                |row| row.get::<_, i32>(0),
            )
            .unwrap_or(0)
            > 0;
        if !has_repo_update_on_startup {
            conn.execute_batch(
                "ALTER TABLE github_repos ADD COLUMN update_on_startup INTEGER DEFAULT 0;",
            )
            .map_err(|e| e.to_string())?;

            let has_legacy_pull_strategy: bool = conn
                .query_row(
                    "SELECT COUNT(*) FROM pragma_table_info('github_repos') WHERE name='pull_strategy'",
                    [],
                    |row| row.get::<_, i32>(0),
                )
                .unwrap_or(0)
                > 0;
            if has_legacy_pull_strategy {
                conn.execute_batch(
                    "UPDATE github_repos SET update_on_startup = CASE
                        WHEN pull_strategy IN ('daily', 'weekly') THEN 1
                        ELSE 0
                    END;",
                )
                .map_err(|e| e.to_string())?;
            }
        }
        self.migrate_repo_tokens_to_keychain(&conn)?;

        // RAG tables
        crate::commands::rag::init_rag_tables(&conn)?;
        conn.execute_batch(
            "
            CREATE INDEX IF NOT EXISTS idx_rag_chunks_collection ON rag_chunks(collection_id, position);
            CREATE INDEX IF NOT EXISTS idx_rag_collections_scope ON rag_collections(scope, scope_id, created_at DESC);
            CREATE INDEX IF NOT EXISTS idx_github_repos_startup ON github_repos(enabled, update_on_startup);
            ",
        )
        .map_err(|e| e.to_string())?;

        // MCP tables
        crate::commands::mcp::init_mcp_tables(&conn)?;

        // source_repo_id column for repo sync tracking
        let has_cmd_source: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('commands') WHERE name='source_repo_id'",
                [],
                |row| row.get::<_, i32>(0),
            )
            .unwrap_or(0)
            > 0;
        if !has_cmd_source {
            conn.execute_batch("ALTER TABLE commands ADD COLUMN source_repo_id TEXT;")
                .map_err(|e| e.to_string())?;
        }

        // Add parameters column to commands if missing
        let has_cmd_params: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('commands') WHERE name='parameters'",
                [],
                |row| row.get::<_, i32>(0),
            )
            .unwrap_or(0)
            > 0;
        if !has_cmd_params {
            conn.execute_batch("ALTER TABLE commands ADD COLUMN parameters TEXT DEFAULT '[]';")
                .map_err(|e| e.to_string())?;
        }

        // Add url column to commands for remote script execution
        let has_cmd_url: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('commands') WHERE name='url'",
                [],
                |row| row.get::<_, i32>(0),
            )
            .unwrap_or(0)
            > 0;
        if !has_cmd_url {
            conn.execute_batch("ALTER TABLE commands ADD COLUMN url TEXT DEFAULT '';")
                .map_err(|e| e.to_string())?;
        }

        let has_script_source: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('scripts') WHERE name='source_repo_id'",
                [],
                |row| row.get::<_, i32>(0),
            )
            .unwrap_or(0)
            > 0;
        if !has_script_source {
            conn.execute_batch("ALTER TABLE scripts ADD COLUMN source_repo_id TEXT;")
                .map_err(|e| e.to_string())?;
        }

        // Add url column to scripts for remote script execution
        let has_script_url: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('scripts') WHERE name='url'",
                [],
                |row| row.get::<_, i32>(0),
            )
            .unwrap_or(0)
            > 0;
        if !has_script_url {
            conn.execute_batch("ALTER TABLE scripts ADD COLUMN url TEXT DEFAULT '';")
                .map_err(|e| e.to_string())?;
        }

        let has_qa_source: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('quick_actions') WHERE name='source_repo_id'",
                [],
                |row| row.get::<_, i32>(0),
            )
            .unwrap_or(0)
            > 0;
        if !has_qa_source {
            conn.execute_batch("ALTER TABLE quick_actions ADD COLUMN source_repo_id TEXT;")
                .map_err(|e| e.to_string())?;
        }

        conn.execute_batch(
            "
            CREATE INDEX IF NOT EXISTS idx_commands_source_repo ON commands(source_repo_id);
            CREATE INDEX IF NOT EXISTS idx_scripts_source_repo ON scripts(source_repo_id);
            CREATE INDEX IF NOT EXISTS idx_quick_actions_source_repo ON quick_actions(source_repo_id);
            ",
        )
        .map_err(|e| e.to_string())?;

        // App logs table
        crate::commands::app_log::init_app_logs_table(&conn)?;

        // 建表完成后再次确认默认仓库（覆盖全新建库的情况）
        Self::ensure_default_library_repo(&conn)?;

        Self::record_schema_version(&conn)?;

        Ok(())
    }

    fn ensure_schema_migrations_table(conn: &Connection) -> Result<(), String> {
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS schema_migrations (
                id TEXT PRIMARY KEY,
                version INTEGER NOT NULL,
                applied_at TEXT DEFAULT (datetime('now', 'localtime'))
            );
            ",
        )
        .map_err(|e| e.to_string())
    }

    /// 若 github_repos 表存在且为空，注入内置默认仓库。
    /// 表不存在（尚未建表）时跳过，留待后续建表完成后的下一次初始化补齐。
    fn ensure_default_library_repo(conn: &Connection) -> Result<(), String> {
        let exists: i32 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='github_repos'",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);
        if exists == 0 {
            return Ok(());
        }
        let repo_count: i32 = conn
            .query_row("SELECT COUNT(*) FROM github_repos", [], |row| row.get(0))
            .unwrap_or(0);
        if repo_count > 0 {
            return Ok(());
        }
        conn.execute(
            "INSERT INTO github_repos (id, url, branch, token, last_pulled_at, update_on_startup, enabled)
             VALUES (?1, ?2, ?3, NULL, NULL, 1, 1)",
            rusqlite::params![
                DEFAULT_LIBRARY_REPO_ID,
                DEFAULT_LIBRARY_REPO_URL,
                DEFAULT_LIBRARY_REPO_BRANCH,
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn applied_schema_version(conn: &Connection) -> Result<i64, String> {
        match conn.query_row(
            "SELECT version FROM schema_migrations WHERE id=?1",
            rusqlite::params![SCHEMA_MIGRATION_ID],
            |row| row.get(0),
        ) {
            Ok(version) => Ok(version),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(0),
            Err(err) => Err(err.to_string()),
        }
    }

    fn record_schema_version(conn: &Connection) -> Result<(), String> {
        conn.execute(
            "
            INSERT INTO schema_migrations (id, version, applied_at)
            VALUES (?1, ?2, datetime('now', 'localtime'))
            ON CONFLICT(id) DO UPDATE SET
                version=excluded.version,
                applied_at=excluded.applied_at
            ",
            rusqlite::params![SCHEMA_MIGRATION_ID, CURRENT_SCHEMA_VERSION],
        )
        .map(|_| ())
        .map_err(|e| e.to_string())
    }

    fn migrate_host_secrets_to_keychain(&self, conn: &Connection) -> Result<(), String> {
        self.migrate_host_secrets_to_keychain_with_mode(conn, false)
    }

    fn migrate_repo_tokens_to_keychain(&self, conn: &Connection) -> Result<(), String> {
        self.migrate_repo_tokens_to_keychain_with_mode(conn, false)
    }

    pub fn migrate_plaintext_secrets_to_vault(&self) -> Result<(), String> {
        let conn = self.pool.get().map_err(|e| e.to_string())?;
        self.migrate_host_secrets_to_keychain_with_mode(&conn, true)?;
        self.migrate_repo_tokens_to_keychain_with_mode(&conn, true)
    }

    fn migrate_host_secrets_to_keychain_with_mode(
        &self,
        conn: &Connection,
        require_unlocked: bool,
    ) -> Result<(), String> {
        let mut stmt = conn
            .prepare("SELECT id, password, private_key, rdp_settings, proxy_settings FROM hosts")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                ))
            })
            .map_err(|e| e.to_string())?;
        let mut updates = Vec::new();
        let mut settings_updates = Vec::new();
        for row in rows {
            let (id, password, private_key, rdp_settings, proxy_settings) =
                row.map_err(|e| e.to_string())?;
            if let Some(secret) =
                password.filter(|value| !value.is_empty() && value != SECRET_PLACEHOLDER)
            {
                if let Err(error) = crate::keychain::store_host_password(&id, &secret) {
                    if !require_unlocked && crate::keychain::is_locked_error_message(&error) {
                        return Ok(());
                    }
                    return Err(error);
                }
                updates.push(("password", id.clone()));
            }
            if let Some(secret) =
                private_key.filter(|value| !value.is_empty() && value != SECRET_PLACEHOLDER)
            {
                if let Err(error) = crate::keychain::store_host_private_key(&id, &secret) {
                    if !require_unlocked && crate::keychain::is_locked_error_message(&error) {
                        return Ok(());
                    }
                    return Err(error);
                }
                updates.push(("private_key", id.clone()));
            }
            match migrate_json_secret_to_keychain(
                &id,
                rdp_settings,
                "vncPassword",
                crate::keychain::store_host_vnc_password,
            ) {
                Ok(Some(updated)) => settings_updates.push(("rdp_settings", id.clone(), updated)),
                Ok(None) => {}
                Err(error) => {
                    if !require_unlocked && crate::keychain::is_locked_error_message(&error) {
                        return Ok(());
                    }
                    return Err(error);
                }
            }
            match migrate_json_secret_to_keychain(
                &id,
                proxy_settings,
                "password",
                crate::keychain::store_host_proxy_password,
            ) {
                Ok(Some(updated)) => {
                    settings_updates.push(("proxy_settings", id.clone(), updated));
                }
                Ok(None) => {}
                Err(error) => {
                    if !require_unlocked && crate::keychain::is_locked_error_message(&error) {
                        return Ok(());
                    }
                    return Err(error);
                }
            }
        }
        for (column, id) in updates {
            let sql = format!("UPDATE hosts SET {}=?1 WHERE id=?2", column);
            conn.execute(&sql, rusqlite::params![SECRET_PLACEHOLDER, id])
                .map_err(|e| e.to_string())?;
        }
        for (column, id, value) in settings_updates {
            let sql = format!("UPDATE hosts SET {}=?1 WHERE id=?2", column);
            conn.execute(&sql, rusqlite::params![value, id])
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    fn migrate_repo_tokens_to_keychain_with_mode(
        &self,
        conn: &Connection,
        require_unlocked: bool,
    ) -> Result<(), String> {
        let mut stmt = conn
            .prepare("SELECT id, token FROM github_repos")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
            })
            .map_err(|e| e.to_string())?;
        let mut ids = Vec::new();
        for row in rows {
            let (id, token) = row.map_err(|e| e.to_string())?;
            if let Some(secret) =
                token.filter(|value| !value.is_empty() && value != SECRET_PLACEHOLDER)
            {
                if let Err(error) = crate::keychain::store_github_token(&id, &secret) {
                    if !require_unlocked && crate::keychain::is_locked_error_message(&error) {
                        return Ok(());
                    }
                    return Err(error);
                }
                ids.push(id);
            }
        }
        for id in ids {
            conn.execute(
                "UPDATE github_repos SET token=?1 WHERE id=?2",
                rusqlite::params![SECRET_PLACEHOLDER, id],
            )
            .map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}
