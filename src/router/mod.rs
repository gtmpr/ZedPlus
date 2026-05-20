pub mod adaptive;
pub mod architect;
pub mod classifier;
pub mod cost;
pub mod rules;

pub use classifier::TaskType;

use crate::{
    backends,
    config::{costs::CostsTable, models::ModelRegistry, schema::Config},
};

#[derive(Debug)]
pub struct RoutingDecision {
    pub model_key: String,
    pub model_id: String,
    pub provider: String,
    pub task_type: TaskType,
    pub use_search_grounding: bool,
    pub use_cache: bool,
    pub estimated_input_tokens: u32,
    pub estimated_cost_usd: f64,
    pub reason: String,
    pub cheapest_alternative: Option<(String, f64)>,
    pub is_architect_mode: bool,
}

/// Core routing pipeline: classify → select → estimate → return decision.
pub fn route(
    query: &str,
    config: &Config,
    registry: &ModelRegistry,
    costs: &CostsTable,
    forced_model: Option<&str>,
    force_local: bool,
    force_cheap: bool,
) -> RoutingDecision {
    let task_type = classifier::classify(query);
    let eligibility = architect::check_eligibility(query, &task_type, config);
    let is_architect = eligibility.is_eligible;

    let alias = if force_local {
        // If architect mode and forced local, use the configured architect model if it's local,
        // otherwise find the best local reasoner.
        if is_architect {
             let arch = &config.routing.architect_editor.architect_model;
             if registry.get(arch).map(|m| m.is_local).unwrap_or(false) {
                 arch.clone()
             } else {
                 registry.models.iter()
                    .filter(|(_, m)| m.is_local && m.supports_reasoning)
                    .max_by_key(|(_, m)| m.quality_tier)
                    .map(|(k, _)| k.clone())
                    .unwrap_or_else(|| "local-reasoner".to_string())
             }
        } else {
            let qc = &config.routing.rules.quick_completion;
            if registry.get(qc).map(|m| m.is_local).unwrap_or(false) {
                qc.clone()
            } else {
                registry
                    .models
                    .iter()
                    .filter(|(_, m)| m.is_local)
                    .max_by_key(|(_, m)| m.quality_tier)
                    .map(|(k, _)| k.clone())
                    .unwrap_or_else(|| "local".to_string())
            }
        }
    } else if force_cheap {
        config.routing.rules.fallback.clone()
    } else if let Some(m) = forced_model {
        m.to_string()
    } else if is_architect {
        rules::select_architect_alias(config, registry)
    } else {
        rules::select_alias(&task_type, config, registry)
    };

    let (provider, model_id) = backends::resolve_model(&alias, registry)
        .unwrap_or_else(|| {
            let prov = infer_provider(&alias);
            (prov, alias.clone())
        });

    let caps = registry.get(&alias);
    let use_cache = caps.map(|m| m.supports_cache).unwrap_or(false);
    let use_search = caps.map(|m| m.supports_search_grounding).unwrap_or(false)
        && matches!(task_type, TaskType::WebSearch);

    // Estimate cost: query tokens + ~500 system prompt overhead, ~1024 output
    let input_est = cost::estimate_tokens(query) + 500;
    let output_est = 1024u32;
    let estimated_cost = cost::estimate_cost(&model_id, input_est, output_est, costs);

    let cheapest = if forced_model.is_none() && !force_local && !force_cheap {
        rules::cheapest_for_task(&task_type, registry, costs, &alias)
    } else {
        None
    };

    let reason = if forced_model.is_some() {
        format!("user override → {alias}")
    } else if force_local {
        "forced local".to_string()
    } else if force_cheap {
        format!("forced cheap → {alias}")
    } else {
        format!("{} rule → {alias}", task_type.as_str())
    };

    RoutingDecision {
        model_key: alias,
        model_id,
        provider,
        task_type,
        use_search_grounding: use_search,
        use_cache,
        estimated_input_tokens: input_est,
        estimated_cost_usd: estimated_cost,
        reason,
        cheapest_alternative: cheapest,
        is_architect_mode: is_architect,
    }
}

fn infer_provider(model_id: &str) -> String {
    if model_id.starts_with("claude") {
        "claude".to_string()
    } else if model_id.starts_with("gemini") {
        "gemini".to_string()
    } else if model_id.starts_with("gpt") || model_id.starts_with("o1") || model_id.starts_with("o3") {
        "openai".to_string()
    } else {
        "ollama".to_string()
    }
}
