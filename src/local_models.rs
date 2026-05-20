//! Discover and rank models available on local inference services (Ollama, LM Studio).
//!
//! Called once at REPL startup. Results are stored in `CliDetection` and used by
//! the pipeline selector to pick the best local model for each phase kind.

use anyhow::Result;

// ── Data types ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct DiscoveredModel {
    pub id:               String,
    pub provider:         &'static str, // "ollama" or "lmstudio"
    pub params_b:         Option<f32>,  // estimated billion params (None = unknown)
    pub quality_tier:     u8,           // 1-5, derived from param count
    pub speed_tier:       u8,           // 1-5, inverse of size
    pub is_coder:         bool,         // code-oriented model → better for execution
    pub reasoning_score:  u8,           // 1-5, best for Reasoning phase
    pub execution_score:  u8,           // 1-5, best for Execution phase
}

impl DiscoveredModel {
    pub fn display_label(&self) -> String {
        let params_str = match self.params_b {
            Some(p) if p >= 1.0 => format!("{:.0}B", p),
            Some(p)             => format!("{:.1}B", p),
            None                => "?B".into(),
        };
        let kind = if self.is_coder { "coder" } else { "general" };
        format!("{} ({}, {}, Q{}/S{})", self.id, self.provider, kind, self.quality_tier, self.speed_tier)
    }
}

// ── Public entry point ────────────────────────────────────────────────────────

/// Query all local services and return discovered models, sorted quality DESC.
/// Silently skips services that aren't reachable.
pub async fn discover(ollama_url: &str, lmstudio_url: &str) -> Vec<DiscoveredModel> {
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
    {
        Ok(c) => c,
        Err(_) => return vec![],
    };

    let mut all = Vec::new();

    let ollama_base = strip_path(ollama_url);
    let lmstudio_base = strip_path(lmstudio_url);
    all.extend(discover_ollama(&client, &ollama_base).await);
    all.extend(discover_lmstudio(&client, &lmstudio_base).await);

    // Sort best-first: quality DESC, then speed DESC as tiebreaker
    all.sort_by(|a, b| {
        b.quality_tier.cmp(&a.quality_tier)
            .then(b.speed_tier.cmp(&a.speed_tier))
    });

    all
}

/// Update a model registry with discovered models.
/// Adds discovered models as new entries and updates 'local' and 'local-reasoner' aliases.
pub fn update_registry_with_discovered(
    registry: &mut crate::config::models::ModelRegistry,
    discovered: &[DiscoveredModel],
) {
    if discovered.is_empty() {
        return;
    }

    // Add each discovered model as a specific alias
    for m in discovered {
        let alias = format!("{}-{}", m.provider, m.id.replace(':', "-"));
        registry.models.insert(
            alias,
            crate::config::models::ModelCapabilities {
                provider: m.provider.to_string(),
                id: m.id.clone(),
                context_window: 128_000, // Reasonable default for modern local models
                supports_search_grounding: false,
                supports_vision: false,
                supports_pdf: false,
                supports_cache: false,
                supports_reasoning: m.reasoning_score >= 3,
                quality_tier: m.quality_tier,
                speed_tier: m.speed_tier,
                strengths: if m.is_coder {
                    vec!["code_review".into(), "quick_completion".into()]
                } else {
                    vec!["documentation".into(), "quick_completion".into()]
                },
                weaknesses: vec![],
                is_local: true,
            },
        );
    }

    // Update 'local' to the best execution/general model
    if let Some(best) = best_for_execution(discovered) {
        if let Some(m) = registry.models.get_mut("local") {
            m.id = best.id.clone();
            m.provider = best.provider.to_string();
            m.quality_tier = best.quality_tier;
            m.speed_tier = best.speed_tier;
        }
    }

    // Update 'local-reasoner' to the best reasoning model
    if let Some(best) = best_for_reasoning(discovered) {
        if let Some(m) = registry.models.get_mut("local-reasoner") {
            m.id = best.id.clone();
            m.provider = best.provider.to_string();
            m.quality_tier = best.quality_tier;
            m.speed_tier = best.speed_tier;
        }
    }
}

/// Return the best model for a reasoning phase (largest general model).
pub fn best_for_reasoning(models: &[DiscoveredModel]) -> Option<&DiscoveredModel> {
    models.iter().max_by_key(|m| m.reasoning_score)
}

/// Return the best model for an execution/code phase (code models preferred, then size).
pub fn best_for_execution(models: &[DiscoveredModel]) -> Option<&DiscoveredModel> {
    models.iter().max_by_key(|m| m.execution_score)
}

/// Return sorted candidates for a reasoning phase.
pub fn ranked_for_reasoning(models: &[DiscoveredModel]) -> Vec<&DiscoveredModel> {
    let mut v: Vec<&DiscoveredModel> = models.iter().collect();
    v.sort_by(|a, b| b.reasoning_score.cmp(&a.reasoning_score));
    v
}

/// Return sorted candidates for an execution phase.
pub fn ranked_for_execution(models: &[DiscoveredModel]) -> Vec<&DiscoveredModel> {
    let mut v: Vec<&DiscoveredModel> = models.iter().collect();
    v.sort_by(|a, b| b.execution_score.cmp(&a.execution_score));
    v
}

// ── Service discovery ─────────────────────────────────────────────────────────

async fn discover_ollama(client: &reqwest::Client, base_url: &str) -> Vec<DiscoveredModel> {
    let url = format!("{base_url}/api/tags");
    let data = match fetch_json(client, &url).await {
        Ok(v) => v,
        Err(_) => return vec![],
    };

    let mut out = Vec::new();
    if let Some(arr) = data["models"].as_array() {
        for m in arr {
            let name = m["name"].as_str().unwrap_or("").to_string();
            if !name.is_empty() {
                out.push(score_model(name, "ollama"));
            }
        }
    }
    out
}

async fn discover_lmstudio(client: &reqwest::Client, base_url: &str) -> Vec<DiscoveredModel> {
    let data = match fetch_json(client, &format!("{base_url}/v1/models")).await {
        Ok(v) => v,
        Err(_) => return vec![],
    };

    let mut out = Vec::new();
    if let Some(arr) = data["data"].as_array() {
        for m in arr {
            let id = m["id"].as_str().unwrap_or("").to_string();
            if !id.is_empty() {
                out.push(score_model(id, "lmstudio"));
            }
        }
    }
    out
}

async fn fetch_json(client: &reqwest::Client, url: &str) -> Result<serde_json::Value> {
    let resp = client.get(url).send().await?;
    if !resp.status().is_success() {
        anyhow::bail!("HTTP {}", resp.status());
    }
    Ok(resp.json().await?)
}

// ── Model scoring ─────────────────────────────────────────────────────────────

fn score_model(id: String, provider: &'static str) -> DiscoveredModel {
    let lower = id.to_lowercase();
    let params_b = extract_params(&lower);

    let quality_tier: u8 = match params_b {
        Some(p) if p >= 65.0 => 5,
        Some(p) if p >= 25.0 => 4,
        Some(p) if p >= 10.0 => 3,
        Some(p) if p >= 4.0  => 2,
        Some(_)               => 1,
        None                  => 2, // unknown → assume small-medium
    };

    // Speed inversely proportional to size
    let speed_tier: u8 = match quality_tier {
        5 => 1,
        4 => 2,
        3 => 3,
        2 => 4,
        _ => 5,
    };

    // Code-oriented models (better at tool use / code generation)
    let is_coder = [
        "code", "coder", "codellama", "starcoder", "deepseek-coder",
        "qwen2.5-coder", "qwq", "devstral", "codestral", "granite-code",
        "wizard-coder", "magicoder", "phind-code",
    ]
    .iter()
    .any(|kw| lower.contains(kw));

    // Reasoning: general models at face value, code models slightly penalised
    // (they can reason but are tuned for code rather than broad analysis)
    let reasoning_score: u8 = if is_coder {
        quality_tier.saturating_sub(1).max(1)
    } else {
        quality_tier
    };

    // Execution: code models get a bonus; pure chat/general models at face value
    let execution_score: u8 = if is_coder {
        (quality_tier + 1).min(5)
    } else {
        quality_tier
    };

    DiscoveredModel {
        id,
        provider,
        params_b,
        quality_tier,
        speed_tier,
        is_coder,
        reasoning_score,
        execution_score,
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Extract the largest parameter count mentioned in a model name.
///
/// Handles patterns like: "7b", "8b", "1.5b", "0.5b", "70b", "13b"
/// Works for names like "llama3.2:8b", "qwen2.5-coder:32b", "Meta-Llama-3.1-8B-...",
/// "mistral-7b-instruct", "phi-4-14b".
fn extract_params(lower: &str) -> Option<f32> {
    let bytes = lower.as_bytes();
    let len = bytes.len();
    let mut best: Option<f32> = None;
    let mut i = 0;

    while i < len {
        if !bytes[i].is_ascii_digit() {
            i += 1;
            continue;
        }

        let start = i;
        while i < len && bytes[i].is_ascii_digit() { i += 1; }

        // Optional decimal part
        if i + 1 < len && bytes[i] == b'.' && bytes[i + 1].is_ascii_digit() {
            i += 1;
            while i < len && bytes[i].is_ascii_digit() { i += 1; }
        }

        // Must be immediately followed by 'b', not 'b' as part of a word like "base"
        if i < len && bytes[i] == b'b' {
            let after = bytes.get(i + 1).copied().unwrap_or(0);
            // Accept end-of-string, or non-alpha char after 'b' (like ':', '-', '_', '/')
            if !after.is_ascii_alphabetic() {
                let num_str = &lower[start..i];
                if let Ok(v) = num_str.parse::<f32>() {
                    if v > 0.0 && v < 1000.0 {
                        if best.map_or(true, |cur| v > cur) {
                            best = Some(v);
                        }
                    }
                }
            }
            i += 1;
        }
    }

    best
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(s: &str) -> Option<f32> {
        extract_params(&s.to_lowercase())
    }

    #[test]
    fn simple_suffixes() {
        assert_eq!(params("llama3:7b"), Some(7.0));
        assert_eq!(params("mistral:8b"), Some(8.0));
        assert_eq!(params("codellama:70b"), Some(70.0));
        assert_eq!(params("qwen2.5:0.5b"), Some(0.5));
        assert_eq!(params("phi-4-14b"), Some(14.0));
    }

    #[test]
    fn decimal_params() {
        assert_eq!(params("qwen2.5-coder:1.5b"), Some(1.5));
        assert_eq!(params("llama3.2:3b"), Some(3.0));
    }

    #[test]
    fn mixed_case_and_separators() {
        assert_eq!(params("Meta-Llama-3.1-8B-Instruct"), Some(8.0));
        assert_eq!(params("mistral-7b-instruct"), Some(7.0));
        assert_eq!(params("gemma4:27b"), Some(27.0));
    }

    #[test]
    fn picks_largest_when_multiple() {
        // "3.2" in the model version vs "8b" param — should pick 8
        assert_eq!(params("llama3.2:8b"), Some(8.0));
    }

    #[test]
    fn no_params_returns_none() {
        assert_eq!(params("unknown-model"), None);
        assert_eq!(params("nomic-embed-text"), None);
    }

    #[test]
    fn score_model_coder_flag() {
        let m = score_model("qwen2.5-coder:7b".to_string(), "ollama");
        assert!(m.is_coder);
        assert!(m.execution_score > m.reasoning_score);
    }

    #[test]
    fn score_model_general_flag() {
        let m = score_model("llama3:70b".to_string(), "ollama");
        assert!(!m.is_coder);
        assert_eq!(m.quality_tier, 5);
        assert_eq!(m.speed_tier, 1);
    }
}

/// Strip any path from a URL, keeping only scheme + host + port.
/// "http://localhost:11434/api/v1" → "http://localhost:11434"
fn strip_path(url: &str) -> String {
    for prefix in &["https://", "http://"] {
        if let Some(rest) = url.strip_prefix(prefix) {
            let host_port = rest.split('/').next().unwrap_or(rest);
            return format!("{prefix}{host_port}");
        }
    }
    url.to_string()
}
