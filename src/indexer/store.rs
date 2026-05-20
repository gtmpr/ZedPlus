use crate::indexer::embedder::{cosine_similarity, from_blob, to_blob};
use anyhow::Result;
use rusqlite::{params, Connection};
use std::path::Path;

pub struct IndexStore {
    conn: Connection,
}

#[derive(Debug, Clone)]
pub struct SimilarChunk {
    pub file_path: String,
    pub symbol: Option<String>,
    pub content: String,
    pub score: f32,
}

impl IndexStore {
    pub fn new(conn: Connection) -> Self {
        Self { conn }
    }

    pub fn open(db_path: &Path) -> Result<Self> {
        let conn = crate::db::open(db_path)?;
        Ok(Self { conn })
    }

    /// Returns true if the file is not indexed or its hash changed.
    pub fn needs_reindex(&self, path: &str, hash: &str) -> Result<bool> {
        let result: Option<String> = self.conn.query_row(
            "SELECT hash FROM files WHERE path = ?1",
            params![path],
            |row| row.get(0),
        ).optional()?;

        Ok(result.as_deref() != Some(hash))
    }

    /// Upsert file record (path + hash).
    pub fn upsert_file(&self, path: &str, hash: &str) -> Result<()> {
        let now = unix_now();
        self.conn.execute(
            "INSERT INTO files (path, hash, indexed_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(path) DO UPDATE SET hash = ?2, indexed_at = ?3",
            params![path, hash, now],
        )?;
        Ok(())
    }

    /// Delete all chunks for a file (call before re-indexing).
    pub fn delete_chunks(&self, path: &str) -> Result<()> {
        // Remove from FTS index before removing from chunks (needs the ids).
        let _ = self.conn.execute(
            "DELETE FROM chunks_fts WHERE rowid IN (SELECT id FROM chunks WHERE file_path = ?1)",
            params![path],
        );
        self.conn.execute("DELETE FROM chunks WHERE file_path = ?1", params![path])?;
        Ok(())
    }

    /// Insert a chunk with its embedding.
    pub fn insert_chunk(
        &self,
        file_path: &str,
        symbol: Option<&str>,
        content: &str,
        embedding: &[f32],
    ) -> Result<()> {
        let blob = to_blob(embedding);
        self.conn.execute(
            "INSERT INTO chunks (file_path, symbol, content, embedding) VALUES (?1, ?2, ?3, ?4)",
            params![file_path, symbol, content, blob],
        )?;
        let rowid = self.conn.last_insert_rowid();
        let _ = self.conn.execute(
            "INSERT INTO chunks_fts(rowid, file_path, symbol, content) VALUES (?1, ?2, ?3, ?4)",
            params![rowid, file_path, symbol, content],
        );
        Ok(())
    }

    /// Delete file record and all its chunks (used for deleted files).
    pub fn delete_file(&self, path: &str) -> Result<()> {
        self.delete_chunks(path)?;
        self.conn.execute("DELETE FROM files WHERE path = ?1", params![path])?;
        Ok(())
    }

    /// Clear the entire index.
    pub fn clear(&self) -> Result<()> {
        self.conn.execute_batch("DELETE FROM chunks_fts; DELETE FROM chunks; DELETE FROM files;")?;
        Ok(())
    }

    /// Construct a concise map of the repository (files and their top-level symbols).
    pub fn build_repomap(&self) -> Result<String> {
        let mut stmt = self.conn.prepare(
            "SELECT file_path, symbol FROM chunks ORDER BY file_path ASC",
        )?;

        let mut current_file = String::new();
        let mut out = String::new();

        let rows = stmt.query_map([], |row| {
            let file_path: String = row.get(0)?;
            let symbol: Option<String> = row.get(1)?;
            Ok((file_path, symbol))
        })?;

        for row in rows.filter_map(|r| r.ok()) {
            let (file_path, symbol) = row;
            if file_path != current_file {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(&format!("{file_path}:"));
                current_file = file_path;
            }
            if let Some(sym) = symbol {
                out.push_str(&format!("\n  - {sym}"));
            }
        }

        Ok(out)
    }

    /// Cosine similarity search — loads all embeddings into memory and ranks them.
    /// Suitable for codebases up to ~50k chunks; a reranker/ANN index is a v2 concern.
    pub fn similarity_search(
        &self,
        query_embedding: &[f32],
        top_k: usize,
    ) -> Result<Vec<SimilarChunk>> {
        // Skip search if query embedding is all zeros (Ollama not available)
        if query_embedding.iter().all(|&v| v == 0.0) {
            return Ok(vec![]);
        }

        let mut stmt = self.conn.prepare(
            "SELECT file_path, symbol, content, embedding FROM chunks",
        )?;

        let mut scored: Vec<(f32, SimilarChunk)> = stmt
            .query_map([], |row| {
                let file_path: String = row.get(0)?;
                let symbol: Option<String> = row.get(1)?;
                let content: String = row.get(2)?;
                let blob: Vec<u8> = row.get(3)?;
                Ok((file_path, symbol, content, blob))
            })?
            .filter_map(|r| r.ok())
            .filter_map(|(file_path, symbol, content, blob)| {
                let emb = from_blob(&blob);
                if emb.is_empty() || emb.iter().all(|&v| v == 0.0) {
                    return None; // skip un-embedded chunks
                }
                let score = cosine_similarity(query_embedding, &emb);
                Some((score, SimilarChunk { file_path, symbol, content, score }))
            })
            .collect();

        // Sort descending by score
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(top_k);

        Ok(scored.into_iter().map(|(_, chunk)| chunk).collect())
    }

    /// BM25 keyword search via FTS5. Falls back to empty if the table isn't ready.
    pub fn keyword_search(&self, query: &str, top_k: usize) -> Result<Vec<SimilarChunk>> {
        let fts_query = fts5_sanitize(query);
        if fts_query.is_empty() {
            return Ok(vec![]);
        }
        let mut stmt = match self.conn.prepare(
            "SELECT file_path, symbol, content FROM chunks_fts \
             WHERE chunks_fts MATCH ?1 ORDER BY rank LIMIT ?2",
        ) {
            Ok(s) => s,
            Err(_) => return Ok(vec![]),
        };
        let results: Vec<SimilarChunk> = stmt
            .query_map(params![fts_query, top_k as i64], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?, row.get::<_, String>(2)?))
            })?
            .enumerate()
            .filter_map(|(i, r)| {
                r.ok().map(|(file_path, symbol, content)| SimilarChunk {
                    file_path,
                    symbol,
                    content,
                    score: 1.0 / (1.0 + i as f32),
                })
            })
            .collect();
        Ok(results)
    }

    /// Hybrid search: combines embedding cosine similarity and BM25 via reciprocal rank fusion.
    /// Gracefully degrades to keyword-only when embeddings are all-zero.
    pub fn hybrid_search(
        &self,
        query_embedding: &[f32],
        query_text: &str,
        top_k: usize,
    ) -> Result<Vec<SimilarChunk>> {
        const K: f32 = 60.0;
        let oversample = top_k * 2;

        let embed_hits = self.similarity_search(query_embedding, oversample)?;
        let keyword_hits = self.keyword_search(query_text, oversample)?;

        if embed_hits.is_empty() && keyword_hits.is_empty() {
            return Ok(vec![]);
        }

        let mut scores: std::collections::HashMap<String, (f32, SimilarChunk)> =
            std::collections::HashMap::new();

        for (rank, chunk) in embed_hits.into_iter().enumerate() {
            let key = chunk_key(&chunk);
            let rrf = 1.0 / (K + rank as f32 + 1.0);
            scores.entry(key).and_modify(|(s, _)| *s += rrf).or_insert((rrf, chunk));
        }
        for (rank, chunk) in keyword_hits.into_iter().enumerate() {
            let key = chunk_key(&chunk);
            let rrf = 1.0 / (K + rank as f32 + 1.0);
            scores.entry(key).and_modify(|(s, _)| *s += rrf).or_insert((rrf, chunk));
        }

        let mut ranked: Vec<(f32, SimilarChunk)> = scores.into_values().collect();
        ranked.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        ranked.truncate(top_k);

        Ok(ranked.into_iter().map(|(score, mut c)| { c.score = score; c }).collect())
    }

    /// Total number of indexed files.
    pub fn file_count(&self) -> Result<i64> {
        Ok(self.conn.query_row("SELECT COUNT(*) FROM files", [], |r| r.get(0))?)
    }

    /// Total number of indexed chunks.
    pub fn chunk_count(&self) -> Result<i64> {
        Ok(self.conn.query_row("SELECT COUNT(*) FROM chunks", [], |r| r.get(0))?)
    }
}

fn fts5_sanitize(query: &str) -> String {
    // Strip FTS5 operators; keep alphanumeric, whitespace, and underscores.
    query
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace() || *c == '_')
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn chunk_key(chunk: &SimilarChunk) -> String {
    format!(
        "{}::{}::{}",
        chunk.file_path,
        chunk.symbol.as_deref().unwrap_or(""),
        &chunk.content[..chunk.content.len().min(80)]
    )
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

/// Compute a quick content fingerprint for change detection.
/// Uses DefaultHasher — consistent within a binary version, not cryptographic.
pub fn content_hash(data: &[u8]) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    data.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

trait OptionalExt<T> {
    fn optional(self) -> rusqlite::Result<Option<T>>;
}

impl<T> OptionalExt<T> for rusqlite::Result<T> {
    fn optional(self) -> rusqlite::Result<Option<T>> {
        match self {
            Ok(v) => Ok(Some(v)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }
}
