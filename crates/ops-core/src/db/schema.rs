//! Tables owned by crates built on top of ops-core (RAG, MCP). They live here so a single `init_tables` can create
//! the whole schema without ops-core depending on those crates.

pub fn init_rag_tables(conn: &rusqlite::Connection) -> Result<(), String> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS rag_collections (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            scope TEXT NOT NULL DEFAULT 'global',
            scope_id TEXT DEFAULT '',
            document_count INTEGER DEFAULT 0,
            created_at TEXT DEFAULT (datetime('now', 'localtime'))
        );

        CREATE TABLE IF NOT EXISTS rag_chunks (
            id TEXT PRIMARY KEY,
            collection_id TEXT NOT NULL,
            content TEXT NOT NULL,
            heading TEXT DEFAULT '',
            heading_level INTEGER DEFAULT 0,
            position INTEGER DEFAULT 0,
            tokens TEXT DEFAULT '[]',
            FOREIGN KEY (collection_id) REFERENCES rag_collections(id) ON DELETE CASCADE
        );

        CREATE INDEX IF NOT EXISTS idx_rag_chunks_collection ON rag_chunks(collection_id);
        ",
    )
    .map_err(|e| e.to_string())
}

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
