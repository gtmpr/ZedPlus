use anyhow::Result;
use rusqlite::Connection;
use std::collections::{HashMap, HashSet};
use std::path::Path;

#[derive(Debug)]
pub struct BenchEntry {
    pub id: String,
    pub query: String,
    pub gold: String,
    pub task_type: String,
}

#[derive(Debug, Clone)]
pub struct BenchScore {
    pub example_id: String,
    pub task_type: String,
    pub token_f1: f32,
    pub semantic_sim: f32,
    pub length_ratio: f32,
    pub format_correct: bool,
}

/// Check if the prediction follows the required <tool_call> XML format if the gold has it.
pub fn check_format(gold: &str, pred: &str) -> bool {
    let gold_has_call = gold.contains("<tool_call>");
    if !gold_has_call {
        return true; // format check only applies to tool tasks
    }
    
    let pred_has_call = pred.contains("<tool_call>") && pred.contains("</tool_call>");
    if !pred_has_call {
        return false;
    }
    
    // Check if it's valid JSON inside
    if let (Some(start), Some(end)) = (pred.find("<tool_call>"), pred.find("</tool_call>")) {
        let json_str = &pred[start + 11..end];
        return serde_json::from_str::<serde_json::Value>(json_str).is_ok();
    }
    
    false
}

/// Load entries from distillation JSONL files in data_dir.
pub fn load_entries(data_dir: &Path, max: usize) -> Vec<BenchEntry> {
    let mut entries = Vec::new();

    let Ok(dir) = std::fs::read_dir(data_dir) else {
        return entries;
    };

    let mut files: Vec<_> = dir
        .flatten()
        .filter(|e| {
            e.path()
                .extension()
                .and_then(|x| x.to_str())
                .map(|x| x == "jsonl")
                .unwrap_or(false)
        })
        .map(|e| e.path())
        .collect();
    files.sort_by(|a, b| b.cmp(a)); // newest first

    for file in &files {
        let Ok(content) = std::fs::read_to_string(file) else { continue; };
        for line in content.lines() {
            if line.trim().is_empty() { continue; }
            let Ok(obj) = serde_json::from_str::<serde_json::Value>(line) else { continue; };
            let query = obj
                .get("instruction")
                .or_else(|| obj.get("input"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let gold = obj
                .get("output")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let task = obj
                .get("task_type")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();

            if query.is_empty() || gold.is_empty() {
                continue;
            }

            let id = format!("{:x}", entries.len());
            entries.push(BenchEntry { id, query, gold, task_type: task });
            if entries.len() >= max {
                return entries;
            }
        }
    }

    entries
}

/// Token F1: precision/recall/F1 over whitespace-split tokens.
pub fn token_f1(gold: &str, pred: &str) -> f32 {
    let gold_toks: HashSet<&str> = gold.split_whitespace().collect();
    let pred_toks: HashSet<&str> = pred.split_whitespace().collect();
    if gold_toks.is_empty() && pred_toks.is_empty() {
        return 1.0;
    }
    let common = gold_toks.intersection(&pred_toks).count() as f32;
    let p = if pred_toks.is_empty() {
        0.0
    } else {
        common / pred_toks.len() as f32
    };
    let r = if gold_toks.is_empty() {
        0.0
    } else {
        common / gold_toks.len() as f32
    };
    if p + r == 0.0 { 0.0 } else { 2.0 * p * r / (p + r) }
}

pub fn length_ratio(gold: &str, pred: &str) -> f32 {
    let g = gold.len() as f32;
    let p = pred.len() as f32;
    if g == 0.0 {
        return if p == 0.0 { 1.0 } else { 0.0 };
    }
    (p / g).min(g / p)
}

pub fn save_result(
    conn: &Connection,
    model: &str,
    baseline: Option<&str>,
    score: &BenchScore,
    ts: i64,
) -> Result<()> {
    conn.execute(
        "INSERT INTO bench_results \
         (ts, model, task_type, example_id, similarity_score, semantic_score, length_ratio, format_correct, baseline_model) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        rusqlite::params![
            ts,
            model,
            score.task_type,
            score.example_id,
            score.token_f1,
            score.semantic_sim,
            score.length_ratio,
            score.format_correct as i32,
            baseline,
        ],
    )?;
    Ok(())
}

pub fn load_last_results(conn: &Connection, model: &str, limit: usize) -> Vec<BenchScore> {
    let Ok(mut stmt) = conn.prepare(
        "SELECT example_id, task_type, similarity_score, semantic_score, length_ratio, format_correct \
         FROM bench_results WHERE model = ?1 ORDER BY ts DESC LIMIT ?2",
    ) else {
        return vec![];
    };
    stmt.query_map(rusqlite::params![model, limit as i64], |row| {
        Ok(BenchScore {
            example_id: row.get(0)?,
            task_type: row.get(1)?,
            token_f1: row.get::<_, f64>(2)? as f32,
            semantic_sim: row.get::<_, f64>(3)? as f32,
            length_ratio: row.get::<_, f64>(4)? as f32,
            format_correct: row.get::<_, i32>(5)? != 0,
        })
    })
    .map(|rows| rows.flatten().collect())
    .unwrap_or_default()
}

/// Returns the last benchmark run timestamp for this model, or None.
pub fn last_run_ts(conn: &Connection, model: &str) -> Option<i64> {
    conn.query_row(
        "SELECT ts FROM bench_results WHERE model = ?1 ORDER BY ts DESC LIMIT 1",
        rusqlite::params![model],
        |row| row.get(0),
    )
    .ok()
}

pub fn print_summary(
    model: &str,
    scores: &[BenchScore],
    baseline_model: Option<&str>,
    baseline_scores: Option<&[BenchScore]>,
) {
    if scores.is_empty() {
        println!("  No results to display.");
        return;
    }

    let avg_f1: f32 = scores.iter().map(|s| s.token_f1).sum::<f32>() / scores.len() as f32;
    let avg_sem: f32 = scores.iter().map(|s| s.semantic_sim).sum::<f32>() / scores.len() as f32;
    let avg_len: f32 = scores.iter().map(|s| s.length_ratio).sum::<f32>() / scores.len() as f32;
    let format_acc: f32 = scores.iter().filter(|s| s.format_correct).count() as f32 / scores.len() as f32;

    println!("\n  ── Benchmark Results ─────────────────────────────────────");
    println!("  Model:          {model}");
    println!("  Samples:        {}", scores.len());
    println!("  Avg token-F1:   {:.3}  (lexical overlap)", avg_f1);
    println!("  Avg semantic:   {:.3}  (embedding similarity)", avg_sem);
    println!("  Format Acc:     {:.1}% (valid <tool_call> tags)", format_acc * 100.0);
    println!("  Avg length-fit: {:.3}  (1.0 = same length)", avg_len);

    // Per-task breakdown
    let mut task_map: HashMap<&str, Vec<f32>> = HashMap::new();
    for s in scores {
        task_map.entry(s.task_type.as_str()).or_default().push(s.semantic_sim);
    }
    if !task_map.is_empty() {
        println!("\n  Semantic similarity by task type:");
        let mut tasks: Vec<_> = task_map.iter().collect();
        tasks.sort_by_key(|(k, _)| *k);
        for (task, sims) in tasks {
            let avg = sims.iter().sum::<f32>() / sims.len() as f32;
            println!("    {:<22} {:.3}  (n={})", task, avg, sims.len());
        }
    }

    if let (Some(bname), Some(bscores)) = (baseline_model, baseline_scores) {
        if !bscores.is_empty() {
            let b_avg = bscores.iter().map(|s| s.semantic_sim).sum::<f32>() / bscores.len() as f32;
            let delta = avg_sem - b_avg;
            println!("\n  vs baseline ({bname}):");
            println!("  Baseline semantic: {:.3}", b_avg);
            println!(
                "  Delta:             {:+.3}  ({})",
                delta,
                if delta > 0.01 {
                    "better ✓"
                } else if delta < -0.01 {
                    "worse ✗"
                } else {
                    "similar ≈"
                }
            );
        }
    }
    println!("  ──────────────────────────────────────────────────────────");
}
