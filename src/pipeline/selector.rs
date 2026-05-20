//! Generic model selection for pipeline phases.
//!
//! Each phase kind has a cascade: tries the best available backend first,
//! falls through to the next until one works. The pipeline calls each in
//! order and stops at the first success.
//!
//! Execution phases (needing agent_step / tool use) NEVER include CLI
//! backends — they only do `complete()` and will error on agent_step.

use crate::{backends::{self, claude_cli::ClaudeCliBackend, gemini_cli::GeminiCliBackend}, config, setup::detector::CliDetection};

/// What a pipeline phase requires from its model.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PhaseKind {
    /// Architecture design, risk analysis, plan verification, arch compliance.
    /// Needs: quality_tier >= 3, complex_reasoning or code_review strength.
    Reasoning,

    /// Build planning, QC review, test plan.
    /// Needs: structured output, quality_tier >= 2. Speed preferred over quality.
    Planning,

    /// Actual code writing via tool loop (read_file / write_file / run_command).
    /// Needs: agent_step support. CLI backends are EXCLUDED.
    Execution,
}

/// One resolved backend option, with enough metadata to log and fall back.
pub struct BackendChoice {
    pub backend:           Box<dyn backends::Backend>,
    /// Model ID string to pass into CompletionOptions / agent_step.
    /// Empty for CLI backends (they ignore it).
    pub model_id:          String,
    /// Human-readable label for phase headers and devlog.
    pub label:             String,
    /// True when backed by a CLI subscription (no per-token billing).
    pub is_subscription:   bool,
    /// True when this backend can do agent_step (tool use).
    pub supports_tool_use: bool,
}

/// Returns all candidates for a phase, best-first.
/// The pipeline tries each in order and stops at the first success.
pub fn cascade(
    kind: PhaseKind,
    cfg: &config::LoadedConfig,
    cli: &CliDetection,
    ollama_url: &str,
) -> Vec<BackendChoice> {
    match kind {
        PhaseKind::Reasoning => reasoning_cascade(cfg, cli, ollama_url),
        PhaseKind::Planning  => planning_cascade(cfg, cli, ollama_url),
        PhaseKind::Execution => execution_cascade(cfg, cli, ollama_url),
    }
}

// ── Reasoning cascade ─────────────────────────────────────────────────────────
// Preferred order: user prefs → subscription CLIs (free) → high-quality API → local if strong

fn reasoning_cascade(
    cfg: &config::LoadedConfig,
    cli: &CliDetection,
    ollama_url: &str,
) -> Vec<BackendChoice> {
    let mut out: Vec<BackendChoice> = Vec::new();

    // User-configured preferences prepend the automatic cascade
    let mut prefs = preferred_choices(&cfg.config.pipeline.reasoning, cfg, ollama_url);
    out.append(&mut prefs);

    if cli.claude {
        out.push(BackendChoice {
            backend:           Box::new(ClaudeCliBackend::new(&cli.claude_bin)),
            model_id:          String::new(),
            label:             "claude-cli (subscription)".into(),
            is_subscription:   true,
            supports_tool_use: false,
        });
    }

    if cli.gemini {
        out.push(BackendChoice {
            backend:           Box::new(GeminiCliBackend::new(&cli.gemini_bin)),
            model_id:          String::new(),
            label:             "gemini-cli (subscription)".into(),
            is_subscription:   true,
            supports_tool_use: false,
        });
    }

    // Cloud models: quality_tier >= 3 with reasoning/review strength, sorted best-first
    let mut cloud = api_models_ranked(cfg, ollama_url, |m| {
        m.quality_tier >= 3
            && m.strengths.iter().any(|s| s == "complex_reasoning" || s == "code_review")
    });
    out.append(&mut cloud);

    // Discovered local models ranked by reasoning score (after cloud — reasoning prefers quality)
    if !cli.local_models.is_empty() {
        let ranked = crate::local_models::ranked_for_reasoning(&cli.local_models);
        for m in ranked {
            out.push(discovered_choice(m, cli, ollama_url));
        }
    }

    // Static local models with quality_tier >= 3 as a free fallback
    let mut local = local_models_ranked(cfg, ollama_url, |m| m.quality_tier >= 3);
    out.append(&mut local);

    // Last resort: any available cloud model
    let mut any_cloud = api_models_ranked(cfg, ollama_url, |_| true);
    out.append(&mut any_cloud);

    dedup(out)
}

// ── Planning cascade ──────────────────────────────────────────────────────────
// Speed over raw quality. User prefs → subscription CLIs → local (free) → cheapest API.

fn planning_cascade(
    cfg: &config::LoadedConfig,
    cli: &CliDetection,
    ollama_url: &str,
) -> Vec<BackendChoice> {
    let mut out: Vec<BackendChoice> = Vec::new();

    let mut prefs = preferred_choices(&cfg.config.pipeline.planning, cfg, ollama_url);
    out.append(&mut prefs);

    // Gemini CLI preferred for planning — fast, large context, subscription
    if cli.gemini {
        out.push(BackendChoice {
            backend:           Box::new(GeminiCliBackend::new(&cli.gemini_bin)),
            model_id:          String::new(),
            label:             "gemini-cli (subscription)".into(),
            is_subscription:   true,
            supports_tool_use: false,
        });
    }

    if cli.claude {
        out.push(BackendChoice {
            backend:           Box::new(ClaudeCliBackend::new(&cli.claude_bin)),
            model_id:          String::new(),
            label:             "claude-cli (subscription)".into(),
            is_subscription:   true,
            supports_tool_use: false,
        });
    }

    // Discovered local models sorted by speed_tier — fast local is ideal for planning
    if !cli.local_models.is_empty() {
        let mut ranked: Vec<&crate::local_models::DiscoveredModel> =
            cli.local_models.iter().collect();
        ranked.sort_by(|a, b| b.speed_tier.cmp(&a.speed_tier)
            .then(b.quality_tier.cmp(&a.quality_tier)));
        for m in ranked {
            out.push(discovered_choice(m, cli, ollama_url));
        }
    }

    // Cheapest fast cloud model (flash/haiku tier) — structured output more
    // reliable than local models for planning tasks
    let mut cheap_cloud = api_models_ranked_by_speed(cfg, ollama_url, |m| {
        m.quality_tier <= 3 && m.speed_tier >= 3
    });
    out.append(&mut cheap_cloud);

    // Static local model with quality_tier >= 2 — free fallback if no cloud key
    let mut local = local_models_ranked(cfg, ollama_url, |m| m.quality_tier >= 2);
    out.append(&mut local);

    // Any cloud model as final fallback
    let mut any = api_models_ranked(cfg, ollama_url, |_| true);
    out.append(&mut any);

    dedup(out)
}

// ── Execution cascade ─────────────────────────────────────────────────────────
// Requires agent_step. CLI backends excluded. User prefs → local first (free) → API.
// Claude/Anthropic API is preferred over Gemini for native tool use reliability.

fn execution_cascade(cfg: &config::LoadedConfig, cli: &CliDetection, ollama_url: &str) -> Vec<BackendChoice> {
    let mut out: Vec<BackendChoice> = Vec::new();

    let mut prefs = preferred_choices(&cfg.config.pipeline.execution, cfg, ollama_url);
    out.append(&mut prefs);

    // When local inference services are running, only use them.
    // If they all fail (e.g. model doesn't support ReAct), the phase is skipped
    // rather than silently billing cloud API credits.
    let has_local_services = !cli.local_models.is_empty();

    if has_local_services {
        let ranked = crate::local_models::ranked_for_execution(&cli.local_models);
        for m in ranked {
            out.push(discovered_choice(m, cli, ollama_url));
        }
        // Also include static local config entries as additional local options
        let mut static_local = local_models_ranked(cfg, ollama_url, |m| m.quality_tier >= 2);
        out.append(&mut static_local);
        return dedup(out);
    }

    // No local services running — fall through to cloud API backends.
    // Claude/Anthropic first — most reliable native tool calling.
    let mut claude = api_models_ranked(cfg, ollama_url, |m| {
        m.provider == "claude" || m.provider == "anthropic"
    });
    out.append(&mut claude);

    // OpenAI as second cloud option
    let mut openai = api_models_ranked(cfg, ollama_url, |m| {
        (m.provider == "openai") && (m.quality_tier >= 3 || m.strengths.iter().any(|s| s == "code_review"))
    });
    out.append(&mut openai);

    // Other strong cloud models
    let mut strong_cloud = api_models_ranked(cfg, ollama_url, |m| {
        m.quality_tier >= 3 || m.strengths.iter().any(|s| s == "code_review")
    });
    out.append(&mut strong_cloud);

    // Any remaining cloud model
    let mut any = api_models_ranked(cfg, ollama_url, |_| true);
    out.append(&mut any);

    dedup(out)
}

// ── Shared helpers ────────────────────────────────────────────────────────────

/// Cloud (non-local) models with an available API key, sorted by quality DESC.
fn api_models_ranked(
    cfg: &config::LoadedConfig,
    ollama_url: &str,
    filter: impl Fn(&crate::config::models::ModelCapabilities) -> bool,
) -> Vec<BackendChoice> {
    let mut candidates: Vec<_> = cfg.models.models.iter()
        .filter(|(_, m)| !m.is_local && filter(m))
        .filter_map(|(alias, m)| {
            let key = crate::get_api_key(&m.provider, cfg).ok()?;
            Some((alias.clone(), m.clone(), key))
        })
        .collect();

    // Best quality first, then fastest as tiebreaker
    candidates.sort_by(|a, b| {
        b.1.quality_tier.cmp(&a.1.quality_tier)
            .then(b.1.speed_tier.cmp(&a.1.speed_tier))
    });

    candidates.into_iter().map(|(alias, m, key)| BackendChoice {
        backend:           backends::create_backend(&m.provider, &key, ollama_url),
        model_id:          m.id.clone(),
        label:             format!("{alias} (API)"),
        is_subscription:   false,
        supports_tool_use: true,
    }).collect()
}

/// Cloud models sorted by speed DESC (for planning phases that want fast output).
fn api_models_ranked_by_speed(
    cfg: &config::LoadedConfig,
    ollama_url: &str,
    filter: impl Fn(&crate::config::models::ModelCapabilities) -> bool,
) -> Vec<BackendChoice> {
    let mut candidates: Vec<_> = cfg.models.models.iter()
        .filter(|(_, m)| !m.is_local && filter(m))
        .filter_map(|(alias, m)| {
            let key = crate::get_api_key(&m.provider, cfg).ok()?;
            Some((alias.clone(), m.clone(), key))
        })
        .collect();

    candidates.sort_by(|a, b| {
        b.1.speed_tier.cmp(&a.1.speed_tier)
            .then(b.1.quality_tier.cmp(&a.1.quality_tier))
    });

    candidates.into_iter().map(|(alias, m, key)| BackendChoice {
        backend:           backends::create_backend(&m.provider, &key, ollama_url),
        model_id:          m.id.clone(),
        label:             format!("{alias} (API)"),
        is_subscription:   false,
        supports_tool_use: true,
    }).collect()
}

/// Local (is_local=true) models sorted by quality DESC.
fn local_models_ranked(
    cfg: &config::LoadedConfig,
    ollama_url: &str,
    filter: impl Fn(&crate::config::models::ModelCapabilities) -> bool,
) -> Vec<BackendChoice> {
    let mut candidates: Vec<_> = cfg.models.models.iter()
        .filter(|(_, m)| m.is_local && filter(m))
        .map(|(alias, m)| (alias.clone(), m.clone()))
        .collect();

    candidates.sort_by(|a, b| b.1.quality_tier.cmp(&a.1.quality_tier));

    candidates.into_iter().map(|(alias, m)| BackendChoice {
        backend:           backends::create_backend(&m.provider, "", ollama_url),
        model_id:          m.id.clone(),
        label:             format!("{alias} (local)"),
        is_subscription:   false,
        supports_tool_use: true,
    }).collect()
}

/// Build a BackendChoice from a live-discovered local model.
/// Routes to the correct service URL based on the model's provider.
fn discovered_choice(m: &crate::local_models::DiscoveredModel, cli: &CliDetection, ollama_url: &str) -> BackendChoice {
    let url = if m.provider == "lmstudio" { &cli.lmstudio_url } else { ollama_url };
    BackendChoice {
        backend:           backends::create_backend(m.provider, "", url),
        model_id:          m.id.clone(),
        label:             format!("{} ({}/local)", m.id, m.provider),
        is_subscription:   false,
        supports_tool_use: true,
    }
}

/// Build BackendChoices for user-specified model aliases, in order.
/// Aliases that don't resolve or have no API key are silently skipped.
fn preferred_choices(
    aliases: &[String],
    cfg: &config::LoadedConfig,
    ollama_url: &str,
) -> Vec<BackendChoice> {
    let mut out = Vec::new();
    for alias in aliases {
        let Some(m) = cfg.models.get(alias) else { continue };
        if m.is_local {
            out.push(BackendChoice {
                backend:           backends::create_backend(&m.provider, "", ollama_url),
                model_id:          m.id.clone(),
                label:             format!("{alias} (local/pref)"),
                is_subscription:   false,
                supports_tool_use: true,
            });
        } else {
            let Ok(key) = crate::get_api_key(&m.provider, cfg) else { continue };
            out.push(BackendChoice {
                backend:           backends::create_backend(&m.provider, &key, ollama_url),
                model_id:          m.id.clone(),
                label:             format!("{alias} (API/pref)"),
                is_subscription:   false,
                supports_tool_use: true,
            });
        }
    }
    out
}

/// Remove duplicates by label (avoids e.g. adding the same API model twice from
/// two filter passes). Preserves first occurrence (highest priority).
fn dedup(choices: Vec<BackendChoice>) -> Vec<BackendChoice> {
    let mut seen = std::collections::HashSet::new();
    choices.into_iter()
        .filter(|c| seen.insert(c.label.clone()))
        .collect()
}
