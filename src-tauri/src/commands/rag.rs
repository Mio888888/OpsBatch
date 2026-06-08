use crate::db::Database;
use regex::Regex;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

// ---------------------------------------------------------------------------
// CJK Bigram Tokenizer
// ---------------------------------------------------------------------------

fn is_cjk(ch: char) -> bool {
    matches!(ch, '\u{4E00}'..='\u{9FFF}' | '\u{3400}'..='\u{4DBF}' | '\u{3000}'..='\u{303F}' | '\u{3040}'..='\u{309F}' | '\u{30A0}'..='\u{30FF}' | '\u{AC00}'..='\u{D7AF}')
}

fn tokenize(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;

    let word_re = Regex::new(r"[a-zA-Z0-9]+").unwrap();

    // Extract latin/digit words first
    for mat in word_re.find_iter(text) {
        tokens.push(mat.as_str().to_lowercase());
    }

    // CJK bigrams
    while i < chars.len() {
        if is_cjk(chars[i]) {
            if i + 1 < chars.len() && is_cjk(chars[i + 1]) {
                tokens.push(format!("{}{}", chars[i], chars[i + 1]));
            }
            tokens.push(chars[i].to_string());
        }
        i += 1;
    }

    tokens
}

// ---------------------------------------------------------------------------
// Markdown-aware Chunking
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentChunk {
    pub id: String,
    pub collection_id: String,
    pub content: String,
    pub heading: String,
    pub heading_level: u8,
    pub position: i64,
}

fn chunk_markdown(text: &str, collection_id: &str) -> Vec<DocumentChunk> {
    let mut chunks = Vec::new();
    let mut current_heading = String::new();
    let mut current_level: u8 = 0;
    let mut current_content = String::new();
    let mut position = 0;

    let heading_re = Regex::new(r"^(#{1,6})\s+(.+)$").unwrap();
    let chunk_size = 800;

    for line in text.lines() {
        if let Some(caps) = heading_re.captures(line) {
            // Flush current chunk if large enough
            if current_content.len() >= chunk_size / 2 {
                chunks.push(DocumentChunk {
                    id: uuid::Uuid::new_v4().to_string(),
                    collection_id: collection_id.to_string(),
                    content: current_content.trim().to_string(),
                    heading: current_heading.clone(),
                    heading_level: current_level,
                    position,
                });
                position += 1;
                current_content.clear();
            }

            current_level = caps[1].len() as u8;
            current_heading = caps[2].trim().to_string();
            current_content.push_str(line);
            current_content.push('\n');
        } else {
            current_content.push_str(line);
            current_content.push('\n');

            if current_content.len() >= chunk_size {
                chunks.push(DocumentChunk {
                    id: uuid::Uuid::new_v4().to_string(),
                    collection_id: collection_id.to_string(),
                    content: current_content.trim().to_string(),
                    heading: current_heading.clone(),
                    heading_level: current_level,
                    position,
                });
                position += 1;
                current_content.clear();
            }
        }
    }

    if !current_content.trim().is_empty() {
        chunks.push(DocumentChunk {
            id: uuid::Uuid::new_v4().to_string(),
            collection_id: collection_id.to_string(),
            content: current_content.trim().to_string(),
            heading: current_heading,
            heading_level: current_level,
            position,
        });
    }

    chunks
}

// ---------------------------------------------------------------------------
// BM25 Scoring
// ---------------------------------------------------------------------------

const K1: f64 = 1.2;
const B: f64 = 0.75;

fn bm25_score(
    query_tokens: &[String],
    doc_tokens: &[String],
    avg_dl: f64,
    doc_count: f64,
    df: &HashMap<String, f64>,
) -> f64 {
    let dl = doc_tokens.len() as f64;
    let tf_map = term_frequency(doc_tokens);
    let mut score = 0.0;

    for qt in query_tokens {
        let tf = *tf_map.get(qt).unwrap_or(&0.0);
        let df_val = *df.get(qt).unwrap_or(&0.0);
        let idf = ((doc_count - df_val + 0.5) / (df_val + 0.5) + 1.0).ln();
        let tf_norm = (tf * (K1 + 1.0)) / (tf + K1 * (1.0 - B + B * (dl / avg_dl)));
        score += idf * tf_norm;
    }

    score
}

fn term_frequency(tokens: &[String]) -> HashMap<String, f64> {
    let mut map = HashMap::new();
    for t in tokens {
        *map.entry(t.clone()).or_insert(0.0) += 1.0;
    }
    map
}

fn document_frequency(all_tokens: &[Vec<String>]) -> HashMap<String, f64> {
    let mut df: HashMap<String, f64> = HashMap::new();
    for doc_tokens in all_tokens {
        let unique: HashSet<&String> = doc_tokens.iter().collect();
        for t in unique {
            *df.entry((*t).clone()).or_insert(0.0) += 1.0;
        }
    }
    df
}

// ---------------------------------------------------------------------------
// Vector Similarity (simplified TF-IDF vectors)
// ---------------------------------------------------------------------------

fn cosine_similarity(a: &[f64], b: &[f64]) -> f64 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let dot: f64 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f64 = a.iter().map(|x| x * x).sum::<f64>().sqrt();
    let norm_b: f64 = b.iter().map(|x| x * x).sum::<f64>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }
    dot / (norm_a * norm_b)
}

fn build_tfidf_vector(tokens: &[String], vocab: &[String], idf: &HashMap<String, f64>) -> Vec<f64> {
    let tf = term_frequency(tokens);
    vocab
        .iter()
        .map(|word| {
            let tf_val = *tf.get(word).unwrap_or(&0.0);
            let idf_val = *idf.get(word).unwrap_or(&0.0);
            tf_val * idf_val
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Reciprocal Rank Fusion
// ---------------------------------------------------------------------------

fn reciprocal_rank_fusion(rankings: &[Vec<(String, f64)>], k: f64) -> Vec<(String, f64)> {
    let mut scores: HashMap<String, f64> = HashMap::new();

    for ranking in rankings {
        for (rank, (id, _score)) in ranking.iter().enumerate() {
            *scores.entry(id.clone()).or_insert(0.0) += 1.0 / (k + (rank + 1) as f64);
        }
    }

    let mut result: Vec<(String, f64)> = scores.into_iter().collect();
    result.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    result
}

// ---------------------------------------------------------------------------
// Collection & Document DB Operations
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeCollection {
    pub id: String,
    pub name: String,
    pub scope: String,
    pub scope_id: String,
    pub document_count: i64,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub chunk_id: String,
    pub collection_id: String,
    pub content: String,
    pub heading: String,
    pub score: f64,
}

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

// ---------------------------------------------------------------------------
// RAG Commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn rag_create_collection(
    db: tauri::State<'_, Database>,
    name: String,
    scope: Option<String>,
    scope_id: Option<String>,
) -> Result<KnowledgeCollection, String> {
    let id = uuid::Uuid::new_v4().to_string();
    let scope = scope.unwrap_or_else(|| "global".into());
    let scope_id = scope_id.unwrap_or_default();

    {
        let conn = db.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO rag_collections (id, name, scope, scope_id) VALUES (?1, ?2, ?3, ?4)",
            params![id, name, scope, scope_id],
        )
        .map_err(|e| e.to_string())?;
    }

    Ok(KnowledgeCollection {
        id,
        name,
        scope,
        scope_id,
        document_count: 0,
        created_at: chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string(),
    })
}

#[tauri::command]
pub fn rag_list_collections(
    db: tauri::State<'_, Database>,
    scope: Option<String>,
    scope_id: Option<String>,
) -> Result<Vec<KnowledgeCollection>, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let mut sql =
        "SELECT id, name, scope, scope_id, document_count, created_at FROM rag_collections"
            .to_string();
    let mut conditions = Vec::new();
    let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

    if let Some(ref s) = scope {
        conditions.push(format!("scope = ?{}", param_values.len() + 1));
        param_values.push(Box::new(s.clone()));
    }
    if let Some(ref sid) = scope_id {
        conditions.push(format!("scope_id = ?{}", param_values.len() + 1));
        param_values.push(Box::new(sid.clone()));
    }

    if !conditions.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&conditions.join(" AND "));
    }
    sql.push_str(" ORDER BY created_at DESC");

    let params: Vec<&dyn rusqlite::types::ToSql> =
        param_values.iter().map(|p| p.as_ref()).collect();
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params.as_slice(), |row| {
            Ok(KnowledgeCollection {
                id: row.get(0)?,
                name: row.get(1)?,
                scope: row.get(2)?,
                scope_id: row.get(3)?,
                document_count: row.get(4)?,
                created_at: row.get(5)?,
            })
        })
        .map_err(|e| e.to_string())?;

    let mut result = Vec::new();
    for row in rows {
        result.push(row.map_err(|e| e.to_string())?);
    }
    Ok(result)
}

#[tauri::command]
pub fn rag_import_document(
    db: tauri::State<'_, Database>,
    collection_id: String,
    content: String,
) -> Result<i64, String> {
    let chunks = chunk_markdown(&content, &collection_id);
    let chunk_count = chunks.len() as i64;

    let conn = db.conn.lock().map_err(|e| e.to_string())?;

    for chunk in &chunks {
        let tokens = tokenize(&chunk.content);
        let tokens_json = serde_json::to_string(&tokens).map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO rag_chunks (id, collection_id, content, heading, heading_level, position, tokens) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![chunk.id, chunk.collection_id, chunk.content, chunk.heading, chunk.heading_level, chunk.position, tokens_json],
        )
        .map_err(|e| e.to_string())?;
    }

    conn.execute(
        "UPDATE rag_collections SET document_count = document_count + 1 WHERE id = ?1",
        params![collection_id],
    )
    .map_err(|e| e.to_string())?;

    Ok(chunk_count)
}

#[tauri::command]
pub fn rag_delete_collection(
    db: tauri::State<'_, Database>,
    collection_id: String,
) -> Result<(), String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    conn.execute(
        "DELETE FROM rag_chunks WHERE collection_id = ?1",
        params![collection_id],
    )
    .map_err(|e| e.to_string())?;
    conn.execute(
        "DELETE FROM rag_collections WHERE id = ?1",
        params![collection_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

// ---------------------------------------------------------------------------
// RAG Search: BM25 + Vector + RRF
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn rag_search(
    db: tauri::State<'_, Database>,
    query: String,
    collection_ids: Option<Vec<String>>,
    limit: Option<i64>,
) -> Result<Vec<SearchResult>, String> {
    let limit = limit.unwrap_or(10);
    let query_tokens = tokenize(&query);

    let conn = db.conn.lock().map_err(|e| e.to_string())?;

    let mut sql = "SELECT id, collection_id, content, heading, tokens FROM rag_chunks".to_string();
    let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    if let Some(ref cids) = collection_ids {
        let placeholders: Vec<String> = cids
            .iter()
            .enumerate()
            .map(|(i, _)| format!("?{}", i + 1))
            .collect();
        sql.push_str(&format!(
            " WHERE collection_id IN ({})",
            placeholders.join(",")
        ));
        for cid in cids {
            param_values.push(Box::new(cid.clone()));
        }
    }

    // Read raw strings from DB
    struct RawChunk {
        id: String,
        collection_id: String,
        content: String,
        heading: String,
        tokens_json: String,
    }

    let params: Vec<&dyn rusqlite::types::ToSql> =
        param_values.iter().map(|p| p.as_ref()).collect();
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params.as_slice(), |row| {
            Ok(RawChunk {
                id: row.get(0)?,
                collection_id: row.get(1)?,
                content: row.get(2)?,
                heading: row.get(3)?,
                tokens_json: row.get(4)?,
            })
        })
        .map_err(|e| e.to_string())?;

    let mut raw_chunks: Vec<RawChunk> = Vec::new();
    for row in rows {
        raw_chunks.push(row.map_err(|e| e.to_string())?);
    }
    drop(stmt);

    if raw_chunks.is_empty() {
        return Ok(Vec::new());
    }

    // Parse tokens
    let parsed: Vec<(String, String, String, String, Vec<String>)> = raw_chunks
        .into_iter()
        .map(|rc| {
            let tokens: Vec<String> = serde_json::from_str(&rc.tokens_json).unwrap_or_default();
            (rc.id, rc.collection_id, rc.content, rc.heading, tokens)
        })
        .collect();

    let doc_count = parsed.len() as f64;
    let all_token_lists: Vec<Vec<String>> = parsed.iter().map(|p| p.4.clone()).collect();
    let avg_dl = all_token_lists.iter().map(|t| t.len() as f64).sum::<f64>() / doc_count;
    let df = document_frequency(&all_token_lists);

    // BM25 ranking
    let mut bm25_ranking: Vec<(String, f64)> = parsed
        .iter()
        .map(|(id, _, _, _, tokens)| {
            let score = bm25_score(&query_tokens, tokens, avg_dl, doc_count, &df);
            (id.clone(), score)
        })
        .filter(|(_, score)| *score > 0.0)
        .collect();
    bm25_ranking.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    // Vector ranking
    let vocab_set: HashSet<String> = parsed
        .iter()
        .flat_map(|(_, _, _, _, tokens)| tokens.iter().cloned())
        .collect();
    let vocab: Vec<String> = vocab_set.into_iter().collect();
    let idf_map: HashMap<String, f64> = vocab
        .iter()
        .map(|word| {
            let df_val = *df.get(word).unwrap_or(&0.0);
            let idf = ((doc_count - df_val + 0.5) / (df_val + 0.5) + 1.0).ln();
            (word.clone(), idf)
        })
        .collect();

    let query_vec = build_tfidf_vector(&query_tokens, &vocab, &idf_map);
    let mut vector_ranking: Vec<(String, f64)> = parsed
        .iter()
        .map(|(id, _, _, _, tokens)| {
            let doc_vec = build_tfidf_vector(tokens, &vocab, &idf_map);
            let sim = cosine_similarity(&query_vec, &doc_vec);
            (id.clone(), sim)
        })
        .filter(|(_, sim)| *sim > 0.0)
        .collect();
    vector_ranking.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    // RRF fusion
    let rankings = vec![bm25_ranking, vector_ranking];
    let fused = reciprocal_rank_fusion(&rankings, 60.0);

    // Map back to results
    let chunk_map: HashMap<String, (String, String, String)> = parsed
        .iter()
        .map(|(id, cid, content, heading, _)| {
            (id.clone(), (cid.clone(), content.clone(), heading.clone()))
        })
        .collect();

    let mut results = Vec::new();
    for (id, score) in fused.into_iter().take(limit as usize) {
        if let Some((cid, content, heading)) = chunk_map.get(&id) {
            results.push(SearchResult {
                chunk_id: id,
                collection_id: cid.clone(),
                content: content.clone(),
                heading: heading.clone(),
                score,
            });
        }
    }

    Ok(results)
}
