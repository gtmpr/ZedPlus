pub mod embedder;
pub mod git;
pub mod parser;
pub mod store;
pub mod watcher;

use crate::platform;
use anyhow::Result;
use embedder::Embedder;
use parser::{is_indexable, should_skip_dir};
use std::path::{Path, PathBuf};
use store::{content_hash, IndexStore};

const OLLAMA_DEFAULT_URL: &str = "http://localhost:11434";

pub async fn run(root: PathBuf, reset: bool) -> Result<()> {
    let root = root.canonicalize().unwrap_or(root);
    let db_path = platform::dirs::db_file()?;
    let store = IndexStore::open(&db_path)?;

    if reset {
        store.clear()?;
        println!("Index cleared.");
    }

    let embedder = Embedder::new(OLLAMA_DEFAULT_URL);
    let embed_available = embedder.is_available().await;

    if !embed_available {
        println!(
            "⚠  Ollama not reachable at {OLLAMA_DEFAULT_URL} — indexing without embeddings."
        );
        println!(
            "   Similarity search will be disabled until Ollama runs with `nomic-embed-text`."
        );
        println!("   Start Ollama, then run `zedplus index --reset` to re-embed.");
    } else {
        println!("✓ Ollama connected — using {} for embeddings.", embedder::DEFAULT_MODEL);
    }

    // ── Initial index pass ───────────────────────────────────────────────────
    println!("Indexing {}...", root.display());
    let indexed = index_directory(&root, &store, &embedder, embed_available).await?;
    let (files, chunks) = (store.file_count()?, store.chunk_count()?);
    println!("✓ Indexed {indexed} files — {files} total files, {chunks} total chunks in index.");

    // ── Git context report ───────────────────────────────────────────────────
    if git::is_git_repo(&root) {
        let branch = git::current_branch(&root).unwrap_or_else(|| "unknown".into());
        println!("  Git repo detected — branch: {branch}");
    }

    // ── Watch loop ───────────────────────────────────────────────────────────
    println!("Watching for changes — press Ctrl+C to stop.\n");
    let mut fw = watcher::FileWatcher::new(&root)?;

    loop {
        let changed = watcher::collect_debounced(&mut fw.rx, 500).await;
        if changed.is_empty() {
            break; // channel closed
        }

        let mut reindexed = 0u32;
        for path in &changed {
            match handle_changed_path(path, &root, &store, &embedder, embed_available).await {
                Ok(true)  => reindexed += 1,
                Ok(false) => {}
                Err(e)    => eprintln!("  ⚠ Error processing {}: {e}", path.display()),
            }
        }

        if reindexed > 0 {
            let chunks = store.chunk_count().unwrap_or(0);
            println!("↺  Re-indexed {reindexed} file(s) — {chunks} chunks total.");
        }
    }

    Ok(())
}

/// Walk `root` and index any files whose content hash has changed.
/// Returns the number of files newly indexed this pass.
async fn index_directory(
    root: &Path,
    store: &IndexStore,
    embedder: &Embedder,
    embed_available: bool,
) -> Result<u32> {
    let mut count = 0u32;
    let mut walk = vec![root.to_path_buf()];

    while let Some(dir) = walk.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(e) => { eprintln!("  ⚠ Cannot read {}: {e}", dir.display()); continue; }
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if !should_skip_dir(name) {
                    walk.push(path);
                }
            } else if path.is_file() && is_indexable(&path) {
                match index_file(&path, store, embedder, embed_available).await {
                    Ok(true)  => count += 1,
                    Ok(false) => {}
                    Err(e)    => eprintln!("  ⚠ {}: {e}", path.display()),
                }
            }
        }
    }

    Ok(count)
}

/// Index a single file. Returns Ok(true) if the file was (re)indexed, Ok(false) if unchanged.
async fn index_file(
    path: &Path,
    store: &IndexStore,
    embedder: &Embedder,
    embed_available: bool,
) -> Result<bool> {
    let content_bytes = std::fs::read(path)?;
    let hash = content_hash(&content_bytes);
    let path_str = path.to_string_lossy();

    if !store.needs_reindex(&path_str, &hash)? {
        return Ok(false);
    }

    let content = match std::str::from_utf8(&content_bytes) {
        Ok(s) => s.to_string(),
        Err(_) => return Ok(false), // skip binary files
    };

    let chunks = parser::parse_file(path, &content);
    if chunks.is_empty() {
        return Ok(false);
    }

    store.delete_chunks(&path_str)?;
    store.upsert_file(&path_str, &hash)?;

    for chunk in &chunks {
        let embedding = if embed_available {
            match embedder.embed(&chunk.content).await {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("  ⚠ Embedding failed for {path_str}: {e}");
                    embedder::Embedder::zero_embedding()
                }
            }
        } else {
            embedder::Embedder::zero_embedding()
        };

        store.insert_chunk(&path_str, chunk.symbol.as_deref(), &chunk.content, &embedding)?;
    }

    Ok(true)
}

/// Handle a file system event: re-index modified files, remove deleted files.
async fn handle_changed_path(
    path: &Path,
    root: &Path,
    store: &IndexStore,
    embedder: &Embedder,
    embed_available: bool,
) -> Result<bool> {
    // Ignore paths outside the watched root
    if !path.starts_with(root) {
        return Ok(false);
    }

    let path_str = path.to_string_lossy();

    if !path.exists() {
        // File was deleted
        store.delete_file(&path_str)?;
        println!("  ✗ Removed from index: {}", path.display());
        return Ok(true);
    }

    if !is_indexable(path) {
        return Ok(false);
    }

    // Skip directories we don't care about
    for component in path.components() {
        let name = component.as_os_str().to_string_lossy();
        if should_skip_dir(&name) {
            return Ok(false);
        }
    }

    let reindexed = index_file(path, store, embedder, embed_available).await?;
    if reindexed {
        println!("  ↺ Re-indexed: {}", path.display());
    }
    Ok(reindexed)
}

// ── Public API ───────────────────────────────────────────────────────────────

/// Index the codebase once (no file watcher). Returns (new_files, total_files, total_chunks).
/// Files whose content hash hasn't changed are skipped — subsequent calls are fast.
pub async fn index_snapshot(root: &Path, ollama_url: &str) -> Result<(u32, i64, i64)> {
    let db_path = platform::dirs::db_file()?;
    let store = IndexStore::open(&db_path)?;
    let embedder = Embedder::new(ollama_url);
    let embed_available = embedder.is_available().await;

    let new_files = index_directory(root, &store, &embedder, embed_available).await?;
    let total_files = store.file_count()?;
    let total_chunks = store.chunk_count()?;
    Ok((new_files, total_files, total_chunks))
}

/// Search the index for chunks similar to `query`.
/// Returns top-K results. Returns empty if embeddings are unavailable.
pub async fn similarity_search(
    query: &str,
    top_k: usize,
    ollama_url: &str,
) -> Result<Vec<store::SimilarChunk>> {
    let db_path = platform::dirs::db_file()?;
    let store = IndexStore::open(&db_path)?;
    let embedder = Embedder::new(ollama_url);

    if !embedder.is_available().await {
        return Ok(vec![]);
    }

    let embedding = embedder.embed(query).await?;
    store.similarity_search(&embedding, top_k)
}
