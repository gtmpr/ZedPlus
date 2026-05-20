use anyhow::Result;
use rusqlite::params;

use crate::config::schema::RoutingRules;

#[derive(Debug)]
pub struct ReliabilityScore {
    pub model: String,
    pub total_tasks: i64,
    /// Fraction of post-write test runs that passed (0.0 if no test data).
    pub test_pass_rate: f32,
    pub tests_run: i64,
    pub tests_passed: i64,
    /// Fraction of tasks where the user re-asked within 30s (negative signal).
    pub negative_signal_rate: f32,
    /// Fraction of tasks where the user manually overrode away from this model.
    pub override_frequency: f32,
    /// Composite reliability score (0.0 – 1.0).
    pub score: f32,
    /// True when the last 2 consecutive test runs for this model both failed.
    pub fresh_eyes_needed: bool,
}

/// Composite reliability score:
///   50% test pass rate  +  30% (1 – negative signal rate)  +  20% (1 – override frequency)
fn compute_score(test_pass_rate: f32, neg_rate: f32, override_freq: f32) -> f32 {
    (0.5 * test_pass_rate + 0.3 * (1.0 - neg_rate) + 0.2 * (1.0 - override_freq)).clamp(0.0, 1.0)
}

/// Analyze model reliability using three live signals from the local SQLite DB.
pub fn analyze_reliability(conn: &rusqlite::Connection) -> Result<Vec<ReliabilityScore>> {
    // Signal 1 — negative signal rate and total tasks, per model (model_key in usage)
    let mut usage_stmt = conn.prepare(
        "SELECT model, COUNT(*) as total, \
         SUM(CASE WHEN negative_signal = 1 THEN 1 ELSE 0 END) as negatives \
         FROM usage GROUP BY model",
    )?;
    let usage_rows = usage_stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, i64>(2)?,
        ))
    })?;
    // model → (total, negatives)
    let mut usage_map: std::collections::HashMap<String, (i64, i64)> =
        std::collections::HashMap::new();
    for row in usage_rows.filter_map(|r| r.ok()) {
        usage_map.insert(row.0, (row.1, row.2));
    }

    // Signal 2 — override frequency: how often a user overrode *away from* a model
    // (model was the routed model but override_model is non-null)
    let mut override_stmt = conn.prepare(
        "SELECT model, \
         SUM(CASE WHEN override_model IS NOT NULL AND override_model != '' THEN 1 ELSE 0 END) as overrides, \
         COUNT(*) as total \
         FROM usage GROUP BY model",
    )?;
    // model → (overrides, total)
    let mut override_map: std::collections::HashMap<String, (i64, i64)> =
        std::collections::HashMap::new();
    let override_rows = override_stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?, row.get::<_, i64>(2)?))
    })?;
    for row in override_rows.filter_map(|r| r.ok()) {
        override_map.insert(row.0, (row.1, row.2));
    }

    // Signal 3 — test pass rate per model_key in test_runs
    let mut test_stmt = conn.prepare(
        "SELECT model_key, SUM(passed) as passes, COUNT(*) as total \
         FROM test_runs WHERE model_key IS NOT NULL GROUP BY model_key",
    )?;
    // model_key → (passes, total)
    let mut test_map: std::collections::HashMap<String, (i64, i64)> =
        std::collections::HashMap::new();
    let test_rows = test_stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?, row.get::<_, i64>(2)?))
    })?;
    for row in test_rows.filter_map(|r| r.ok()) {
        test_map.insert(row.0, (row.1, row.2));
    }

    // Anti-Loop: last 2 test results per model — if both failed, flag fresh_eyes_needed
    let mut fresh_eyes_set: std::collections::HashSet<String> = std::collections::HashSet::new();
    {
        // Collect all known model_keys from test_runs
        let mut mk_stmt = conn.prepare(
            "SELECT DISTINCT model_key FROM test_runs WHERE model_key IS NOT NULL",
        )?;
        let mk_rows: Vec<String> = mk_stmt
            .query_map([], |row| row.get::<_, String>(0))?
            .filter_map(|r| r.ok())
            .collect();

        for mk in mk_rows {
            let mut last2_stmt = conn.prepare(
                "SELECT passed FROM test_runs WHERE model_key = ?1 ORDER BY ts DESC LIMIT 2",
            )?;
            let results: Vec<i64> = last2_stmt
                .query_map(params![&mk], |row| row.get::<_, i64>(0))?
                .filter_map(|r| r.ok())
                .collect();
            if results.len() == 2 && results[0] == 0 && results[1] == 0 {
                fresh_eyes_set.insert(mk);
            }
        }
    }

    // Build the unified set of all known models
    let all_models: std::collections::HashSet<String> = usage_map
        .keys()
        .chain(test_map.keys())
        .cloned()
        .collect();

    let mut scores: Vec<ReliabilityScore> = all_models
        .into_iter()
        .map(|model| {
            let (total, negatives) = usage_map.get(&model).copied().unwrap_or((0, 0));
            let (overrides, ov_total) = override_map.get(&model).copied().unwrap_or((0, 0));
            let (passes, tests_total) = test_map.get(&model).copied().unwrap_or((0, 0));

            let neg_rate = if total > 0 { negatives as f32 / total as f32 } else { 0.0 };
            let override_freq = if ov_total > 0 { overrides as f32 / ov_total as f32 } else { 0.0 };
            let test_pass_rate = if tests_total > 0 { passes as f32 / tests_total as f32 } else { 0.5 }; // neutral prior when no data

            let score = compute_score(test_pass_rate, neg_rate, override_freq);
            let fresh_eyes_needed = fresh_eyes_set.contains(&model);

            ReliabilityScore {
                total_tasks: total,
                test_pass_rate,
                tests_run: tests_total,
                tests_passed: passes,
                negative_signal_rate: neg_rate,
                override_frequency: override_freq,
                score,
                fresh_eyes_needed,
                model,
            }
        })
        .collect();

    scores.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    Ok(scores)
}

#[derive(Debug)]
pub struct RoutingSuggestion {
    pub task_type: String,
    /// What the routing rules currently say for this task
    pub current_alias: String,
    /// What the user actually chose (most-used override for this task)
    pub suggested_alias: String,
    /// How many times the user manually overrode to the suggested model
    pub override_count: i64,
    /// How many negative signals (re-ask) were recorded for the current model on this task
    pub negative_signals: i64,
}

impl RoutingSuggestion {
    pub fn diff_line(&self) -> String {
        format!(
            "  {:<22}  {}  →  {}  ({} overrides{})",
            self.task_type,
            self.current_alias,
            self.suggested_alias,
            self.override_count,
            if self.negative_signals > 0 {
                format!(", {} negative signals", self.negative_signals)
            } else {
                String::new()
            }
        )
    }
}

/// Analyse the usage table and return suggestions where the user consistently
/// overrode the routing for a task type.
pub fn analyze(
    conn: &rusqlite::Connection,
    rules: &RoutingRules,
    min_overrides: i64,
) -> Result<Vec<RoutingSuggestion>> {
    // Find the most-used override model per task_type
    let mut stmt = conn.prepare(
        "SELECT task_type, override_model, COUNT(*) as cnt \
         FROM usage \
         WHERE override_model IS NOT NULL AND override_model != '' \
         GROUP BY task_type, override_model \
         ORDER BY task_type, cnt DESC",
    )?;

    // Collect: task_type → (best_override_alias, count)
    let mut best: std::collections::HashMap<String, (String, i64)> =
        std::collections::HashMap::new();

    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, i64>(2)?,
        ))
    })?;

    for row in rows.filter_map(|r| r.ok()) {
        let (task, model, cnt) = row;
        best.entry(task).or_insert((model, cnt));
        // (first row per task is already the highest due to ORDER BY cnt DESC)
    }

    // Count negative signals per (task_type, current routing rule model)
    let mut neg_stmt = conn.prepare(
        "SELECT task_type, COUNT(*) FROM usage \
         WHERE negative_signal = 1 AND override_model IS NULL \
         GROUP BY task_type",
    )?;
    let neg_rows = neg_stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
    })?;
    let negatives: std::collections::HashMap<String, i64> =
        neg_rows.filter_map(|r| r.ok()).collect();

    let mut suggestions = Vec::new();

    for (task_type, (suggested_alias, override_count)) in best {
        if override_count < min_overrides {
            continue;
        }

        let current_alias = current_rule_for_task(&task_type, rules);

        // Only suggest if the override is actually different from the current rule
        if suggested_alias == current_alias {
            continue;
        }

        let negative_signals = *negatives.get(&task_type).unwrap_or(&0);

        suggestions.push(RoutingSuggestion {
            task_type,
            current_alias,
            suggested_alias,
            override_count,
            negative_signals,
        });
    }

    // Sort by override count descending
    suggestions.sort_by(|a, b| b.override_count.cmp(&a.override_count));

    Ok(suggestions)
}

/// Apply suggestions to a mutable RoutingRules, returning the list of changed fields.
pub fn apply(rules: &mut RoutingRules, suggestions: &[RoutingSuggestion]) -> Vec<String> {
    let mut changed = Vec::new();
    for s in suggestions {
        let field = match s.task_type.as_str() {
            "web_search" => {
                rules.web_search = s.suggested_alias.clone();
                "routing.rules.web_search"
            }
            "code_review" => {
                rules.code_review = s.suggested_alias.clone();
                "routing.rules.code_review"
            }
            "complex_reasoning" => {
                rules.complex_reasoning = s.suggested_alias.clone();
                "routing.rules.complex_reasoning"
            }
            "data_analysis" => {
                rules.data_analysis = s.suggested_alias.clone();
                "routing.rules.data_analysis"
            }
            "documentation" => {
                rules.documentation = s.suggested_alias.clone();
                "routing.rules.documentation"
            }
            "quick_completion" => {
                rules.quick_completion = s.suggested_alias.clone();
                "routing.rules.quick_completion"
            }
            "fallback" => {
                rules.fallback = s.suggested_alias.clone();
                "routing.rules.fallback"
            }
            _ => continue,
        };
        changed.push(format!(
            "{} = \"{}\"  # was \"{}\"",
            field, s.suggested_alias, s.current_alias
        ));
    }
    changed
}

fn current_rule_for_task(task: &str, rules: &RoutingRules) -> String {
    match task {
        "web_search" => rules.web_search.clone(),
        "code_review" => rules.code_review.clone(),
        "complex_reasoning" => rules.complex_reasoning.clone(),
        "data_analysis" => rules.data_analysis.clone(),
        "documentation" => rules.documentation.clone(),
        "quick_completion" => rules.quick_completion.clone(),
        "fallback" => rules.fallback.clone(),
        _ => "unknown".to_string(),
    }
}
