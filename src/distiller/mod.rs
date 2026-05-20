pub mod bench;
pub mod trainer;

use anyhow::Result;
use chrono::Utc;
use rusqlite::params;
use serde_json::json;
use std::io::Write as IoWrite;

pub struct DistillEntry {
    pub query: String,
    pub response: String,
    pub model_key: String,
    pub model_id: String,
    pub task_type: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cost_usd: f64,
    pub cache_hit: bool,
    pub override_model: Option<String>,
    pub is_architect_split: bool,
    pub reward_signal: f64,
    pub edit_accepted: bool,
    pub session_id: Option<String>,
}

/// Append an Alpaca-format JSONL line and write a usage row to SQLite.
pub fn record(entry: DistillEntry) -> Result<()> {
    let now = Utc::now();
    let ts = now.timestamp();

    write_jsonl(&entry, ts, &now)?;
    write_usage(&entry, ts)?;

    Ok(())
}

/// Update the reward signal for the most recent usage row.
pub fn update_reward(reward: f64) -> Result<()> {
    let db_path = crate::platform::dirs::db_file()?;
    let conn = crate::db::open(&db_path)?;
    conn.execute(
        "UPDATE usage SET reward_signal = ?1 WHERE id = (SELECT MAX(id) FROM usage)",
        params![reward],
    )?;
    Ok(())
}

/// Mark the most recent usage row as accepted.
pub fn mark_accepted() -> Result<()> {
    let db_path = crate::platform::dirs::db_file()?;
    let conn = crate::db::open(&db_path)?;
    conn.execute(
        "UPDATE usage SET edit_accepted = 1, reward_signal = reward_signal + 0.5 WHERE id = (SELECT MAX(id) FROM usage)",
        [],
    )?;
    Ok(())
}

/// Read and optionally filter the distillation JSONL, yielding raw JSON lines.
pub fn export(
    task_filter: Option<&str>,
    model_filter: Option<&str>,
    since_ts: Option<i64>,
    weighted: bool,
) -> Result<Vec<String>> {
    let distill_dir = crate::platform::dirs::distill_dir()?;
    let mut lines = Vec::new();

    // Collect all monthly files, sorted newest-first
    let mut files: Vec<_> = std::fs::read_dir(&distill_dir)?
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .extension()
                .and_then(|x| x.to_str())
                .map(|x| x == "jsonl")
                .unwrap_or(false)
        })
        .map(|e| e.path())
        .collect();
    files.sort_by(|a, b| b.cmp(a));

    let now_ts = Utc::now().timestamp();

    for file in &files {
        let raw = std::fs::read_to_string(file)?;
        for line in raw.lines() {
            if line.trim().is_empty() {
                continue;
            }
            let v: serde_json::Value = match serde_json::from_str(line) {
                Ok(v) => v,
                Err(_) => continue,
            };

            // Apply filters
            if let Some(task) = task_filter {
                if v["task_type"].as_str().unwrap_or("") != task {
                    continue;
                }
            }
            if let Some(model) = model_filter {
                if v["model"].as_str().unwrap_or("") != model {
                    continue;
                }
            }
            let entry_ts = v["ts"].as_i64().unwrap_or(0);
            if let Some(since) = since_ts {
                if entry_ts < since {
                    continue;
                }
            }

            if weighted {
                // Include multiple copies of recent examples to weight training
                let age_days = (now_ts - entry_ts).max(0) / 86400;
                let copies: usize = if age_days < 30 { 4 } else if age_days < 90 { 2 } else { 1 };
                for _ in 0..copies {
                    lines.push(line.to_string());
                }
            } else {
                lines.push(line.to_string());
            }
        }
    }

    Ok(lines)
}

fn write_jsonl(entry: &DistillEntry, ts: i64, now: &chrono::DateTime<Utc>) -> Result<()> {
    let distill_dir = crate::platform::dirs::distill_dir()?;
    let filename = format!("{}.jsonl", now.format("%Y-%m"));
    let path = distill_dir.join(filename);

    let record = json!({
        "instruction": entry.query,
        "input": "",
        "output": entry.response,
        "model": entry.model_key,
        "task_type": entry.task_type,
        "session_id": entry.session_id,
        "ts": ts,
    });

    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;

    writeln!(file, "{}", record)?;
    Ok(())
}

fn write_usage(entry: &DistillEntry, ts: i64) -> Result<()> {
    let db_path = crate::platform::dirs::db_file()?;
    let conn = crate::db::open(&db_path)?;

    // Detect negative signal: if there's a usage row within the last 30s,
    // the user likely re-asked because the prior response was unsatisfactory.
    let last_ts: Option<i64> = conn
        .query_row(
            "SELECT ts FROM usage ORDER BY ts DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .ok();
    let negative_signal = last_ts.map(|t| ts - t < 30).unwrap_or(false) as i32;

    conn.execute(
        "INSERT INTO usage (ts, model, task_type, input_tokens, output_tokens, cost_usd, cache_hit, override_model, negative_signal, is_architect_split, reward_signal, edit_accepted, session_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
        params![
            ts,
            entry.model_key,
            entry.task_type,
            entry.input_tokens,
            entry.output_tokens,
            entry.cost_usd,
            entry.cache_hit as i32,
            entry.override_model,
            negative_signal,
            entry.is_architect_split as i32,
            entry.reward_signal,
            entry.edit_accepted as i32,
            entry.session_id,
        ],
    )?;

    Ok(())
}
