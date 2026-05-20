use anyhow::Result;
use rusqlite::Connection;
use std::path::Path;

pub fn open(path: &Path) -> Result<Connection> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let conn = Connection::open(path)?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
    init_schema(&conn)?;
    run_migrations(&conn);
    Ok(conn)
}

/// Additive schema migrations. Each statement is best-effort — failures are
/// silently ignored so existing databases with different schemas still open.
fn run_migrations(conn: &Connection) {
    let _ = conn.execute_batch("ALTER TABLE session_turns ADD COLUMN persona TEXT;");
    let _ = conn.execute_batch("ALTER TABLE usage ADD COLUMN is_architect_split INTEGER DEFAULT 0;");
    let _ = conn.execute_batch("ALTER TABLE bench_results ADD COLUMN semantic_score REAL DEFAULT 0.0;");
    let _ = conn.execute_batch("ALTER TABLE bench_results ADD COLUMN format_correct INTEGER DEFAULT 1;");
    // Phase 13: link test_runs to the model that triggered them
    let _ = conn.execute_batch("ALTER TABLE test_runs ADD COLUMN model_key TEXT;");
    // Phase 10: Reward signals for reinforcement learning
    let _ = conn.execute_batch("ALTER TABLE usage ADD COLUMN reward_signal REAL DEFAULT 0.0;");
    let _ = conn.execute_batch("ALTER TABLE usage ADD COLUMN edit_accepted INTEGER DEFAULT 0;");
    let _ = conn.execute_batch("ALTER TABLE usage ADD COLUMN session_id TEXT;");
}

fn init_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS files (
            path TEXT PRIMARY KEY,
            hash TEXT NOT NULL,
            indexed_at INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS chunks (
            id INTEGER PRIMARY KEY,
            file_path TEXT NOT NULL,
            symbol TEXT,
            content TEXT NOT NULL,
            embedding BLOB NOT NULL
        );

        CREATE TABLE IF NOT EXISTS usage (
            id INTEGER PRIMARY KEY,
            ts INTEGER NOT NULL,
            model TEXT NOT NULL,
            task_type TEXT,
            input_tokens INTEGER,
            output_tokens INTEGER,
            cost_usd REAL,
            cache_hit INTEGER DEFAULT 0,
            override_model TEXT,
            negative_signal INTEGER DEFAULT 0
        );

        CREATE TABLE IF NOT EXISTS bench_results (
            id INTEGER PRIMARY KEY,
            ts INTEGER NOT NULL,
            model TEXT NOT NULL,
            task_type TEXT,
            example_id TEXT NOT NULL,
            similarity_score REAL,
            length_ratio REAL,
            baseline_model TEXT
        );

        CREATE TABLE IF NOT EXISTS model_registry (
            name TEXT PRIMARY KEY,
            provider TEXT NOT NULL,
            model_id TEXT NOT NULL,
            path TEXT,
            imported_at INTEGER,
            last_trained_at INTEGER,
            is_active INTEGER DEFAULT 1
        );

        CREATE TABLE IF NOT EXISTS train_jobs (
            id INTEGER PRIMARY KEY,
            started_at INTEGER NOT NULL,
            finished_at INTEGER,
            base_model TEXT NOT NULL,
            output_model TEXT,
            dataset_size INTEGER,
            method TEXT,
            status TEXT,
            benchmark_delta REAL
        );

        CREATE TABLE IF NOT EXISTS sessions (
            id TEXT PRIMARY KEY,
            name TEXT,
            project_path TEXT NOT NULL,
            git_branch TEXT,
            started_at INTEGER NOT NULL,
            last_active INTEGER NOT NULL,
            turn_count INTEGER DEFAULT 0,
            total_cost_usd REAL DEFAULT 0,
            status TEXT DEFAULT 'active'
        );

        CREATE TABLE IF NOT EXISTS session_turns (
            id INTEGER PRIMARY KEY,
            session_id TEXT NOT NULL,
            ts INTEGER NOT NULL,
            role TEXT NOT NULL,
            content TEXT NOT NULL,
            model TEXT,
            tokens_in INTEGER,
            tokens_out INTEGER
        );

        CREATE TABLE IF NOT EXISTS test_runs (
            id INTEGER PRIMARY KEY,
            ts INTEGER NOT NULL,
            runner TEXT NOT NULL,
            triggered_by TEXT,
            passed INTEGER NOT NULL,
            failed INTEGER NOT NULL,
            duration_ms INTEGER,
            output TEXT
        );

        CREATE TABLE IF NOT EXISTS bench_perf (
            id INTEGER PRIMARY KEY,
            ts INTEGER NOT NULL,
            benchmark_name TEXT NOT NULL,
            duration_ns INTEGER,
            triggered_by TEXT,
            delta_pct REAL
        );

        CREATE INDEX IF NOT EXISTS idx_chunks_file ON chunks (file_path);
        CREATE INDEX IF NOT EXISTS idx_usage_ts ON usage (ts);
        CREATE INDEX IF NOT EXISTS idx_sessions_project ON sessions (project_path, git_branch, status);
        CREATE INDEX IF NOT EXISTS idx_session_turns_session ON session_turns (session_id);
        "#,
    )?;
    Ok(())
}
