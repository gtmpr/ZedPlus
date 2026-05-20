use crate::config::{
    costs::CostsTable,
    models::ModelRegistry,
    schema::{Config, RoutingPriority},
};
use super::classifier::TaskType;

pub fn select_editor_alias(config: &Config, registry: &ModelRegistry) -> String {
    let configured = &config.routing.architect_editor.editor_model;
    if registry.get(configured).is_some() {
        return configured.clone();
    }

    // Otherwise find highest speed tier model with decent quality
    registry.models.iter()
        .filter(|(_, m)| m.speed_tier >= 4)
        .max_by_key(|(_, m)| m.quality_tier)
        .map(|(k, _)| k.clone())
        .unwrap_or_else(|| config.routing.rules.fallback.clone())
}

pub fn select_architect_alias(config: &Config, registry: &ModelRegistry) -> String {
    // If explicitly configured in architect_editor config
    let configured = &config.routing.architect_editor.architect_model;
    if registry.get(configured).is_some() {
        return configured.clone();
    }

    // Otherwise find the highest quality tier model with supports_reasoning
    registry.models.iter()
        .filter(|(_, m)| m.supports_reasoning)
        .max_by_key(|(_, m)| m.quality_tier)
        .map(|(k, _)| k.clone())
        .unwrap_or_else(|| config.routing.rules.complex_reasoning.clone())
}

/// Select the model alias for a task, applying priority mode and project overrides.
pub fn select_alias(task: &TaskType, config: &Config, registry: &ModelRegistry) -> String {
    // Project override wins
    if let Some(override_model) = config.routing.overrides.get(task.as_str()) {
        return override_model.clone();
    }

    let base = base_alias_for_task(task, config);

    let resolved = match config.routing.priority {
        RoutingPriority::Balanced | RoutingPriority::Quality => base,

        RoutingPriority::LocalFirst => {
            // Find highest-quality local model that matches the task strength
            let strength = task_strength(task);
            let best_local = registry
                .models
                .iter()
                .filter(|(_, m)| {
                    m.is_local
                        && (strength.is_empty() || m.strengths.iter().any(|s| s == strength))
                })
                .max_by_key(|(_, m)| m.quality_tier);

            if let Some((key, _)) = best_local {
                key.clone()
            } else {
                base
            }
        }

        RoutingPriority::Cost => {
            // Find cheapest non-local model that matches the task strength
            // (CostsTable not available here — resolved at route() level using model IDs)
            base
        }
    };

    // If the resolved alias isn't in the registry (e.g. default "claude-sonnet" doesn't exist),
    // pick the best available model for this task type rather than falling through to name inference.
    if registry.get(&resolved).is_none() {
        let strength = task_strength(task);
        if let Some((key, _)) = registry
            .models
            .iter()
            .filter(|(_, m)| {
                !m.is_local
                    && (strength.is_empty() || m.strengths.iter().any(|s| s == strength))
            })
            .max_by_key(|(_, m)| (m.quality_tier, m.speed_tier))
        {
            return key.clone();
        }
    }

    resolved
}

/// Find the cheapest model for a task (excluding the currently chosen key).
/// Returns (alias, estimated_cost) or None if no alternative exists.
pub fn cheapest_for_task(
    task: &TaskType,
    registry: &ModelRegistry,
    costs: &CostsTable,
    exclude_key: &str,
) -> Option<(String, f64)> {
    let strength = task_strength(task);
    let input_est = 500u32;
    let output_est = 500u32;

    registry
        .models
        .iter()
        .filter(|(key, m)| {
            key.as_str() != exclude_key
                && !m.is_local
                && (strength.is_empty() || m.strengths.iter().any(|s| s == strength))
        })
        .map(|(key, m)| {
            let cost = costs.cost_usd(&m.id, input_est, output_est);
            (key.clone(), cost)
        })
        .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
}

fn base_alias_for_task(task: &TaskType, config: &Config) -> String {
    let rules = &config.routing.rules;
    match task {
        TaskType::WebSearch => rules.web_search.clone(),
        TaskType::CodeReview => rules.code_review.clone(),
        TaskType::ComplexReasoning => rules.complex_reasoning.clone(),
        TaskType::DataAnalysis => rules.data_analysis.clone(),
        TaskType::Documentation => rules.documentation.clone(),
        TaskType::QuickCompletion => rules.quick_completion.clone(),
        TaskType::Fallback => rules.fallback.clone(),
    }
}

fn task_strength(task: &TaskType) -> &'static str {
    match task {
        TaskType::CodeReview => "code_review",
        TaskType::ComplexReasoning => "complex_reasoning",
        TaskType::DataAnalysis => "data_analysis",
        TaskType::Documentation => "documentation",
        TaskType::WebSearch => "web_search",
        TaskType::QuickCompletion | TaskType::Fallback => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{
        costs::CostsTable,
        models::{ModelCapabilities, ModelRegistry},
        schema::{Config, RoutingRules},
    };
    use std::collections::HashMap;

    fn make_registry_with(entries: &[(&str, &str, u8, bool, Vec<&str>)]) -> ModelRegistry {
        let mut models = HashMap::new();
        for (alias, id, quality, is_local, strengths) in entries {
            models.insert(alias.to_string(), ModelCapabilities {
                provider: "test".to_string(),
                id: id.to_string(),
                context_window: 100_000,
                supports_search_grounding: false,
                supports_vision: false,
                supports_pdf: false,
                supports_cache: false,
                supports_reasoning: true,
                quality_tier: *quality,
                speed_tier: 3,
                strengths: strengths.iter().map(|s| s.to_string()).collect(),
                weaknesses: vec![],
                is_local: *is_local,
            });
        }
        ModelRegistry { models }
    }

    fn make_config_with_rules(rules: RoutingRules) -> Config {
        let mut cfg = Config::default();
        cfg.routing.rules = rules;
        cfg
    }

    #[test]
    fn select_alias_uses_configured_rule() {
        let registry = make_registry_with(&[
            ("gemini-flash", "gemini-flash-2.5", 3, false, vec!["web_search"]),
        ]);
        let cfg = Config::default(); // web_search → "gemini-flash"
        let alias = select_alias(&TaskType::WebSearch, &cfg, &registry);
        assert_eq!(alias, "gemini-flash");
    }

    #[test]
    fn select_alias_falls_back_when_configured_missing() {
        // Config points to "claude-sonnet" but only "gemini-flash" is in registry
        let registry = make_registry_with(&[
            ("gemini-flash", "gemini-flash-2.5", 3, false, vec!["code_review"]),
        ]);
        let cfg = Config::default(); // code_review → "claude-sonnet" (not in registry)
        let alias = select_alias(&TaskType::CodeReview, &cfg, &registry);
        // Should fall back to gemini-flash (best available with "code_review" strength)
        assert_eq!(alias, "gemini-flash");
    }

    #[test]
    fn select_alias_respects_project_override() {
        let registry = make_registry_with(&[
            ("my-model", "my-model-id", 4, false, vec![]),
            ("gemini-flash", "gemini-flash-2.5", 3, false, vec![]),
        ]);
        let mut cfg = Config::default();
        cfg.routing.overrides.insert("web_search".to_string(), "my-model".to_string());
        let alias = select_alias(&TaskType::WebSearch, &cfg, &registry);
        assert_eq!(alias, "my-model");
    }

    #[test]
    fn select_alias_local_first_prefers_local() {
        let registry = make_registry_with(&[
            ("cloud-model", "cloud-id", 5, false, vec![]),
            ("local-model", "local-id", 3, true, vec![]),
        ]);
        let mut cfg = Config::default();
        cfg.routing.priority = RoutingPriority::LocalFirst;
        let alias = select_alias(&TaskType::QuickCompletion, &cfg, &registry);
        assert_eq!(alias, "local-model");
    }

    #[test]
    fn select_alias_picks_highest_quality_when_fallback_needed() {
        let registry = make_registry_with(&[
            ("model-a", "model-a-id", 3, false, vec!["complex_reasoning"]),
            ("model-b", "model-b-id", 5, false, vec!["complex_reasoning"]),
        ]);
        // Config points to "claude-sonnet" which is not in registry
        let cfg = Config::default(); // complex_reasoning → "claude-sonnet"
        let alias = select_alias(&TaskType::ComplexReasoning, &cfg, &registry);
        assert_eq!(alias, "model-b"); // highest quality_tier
    }
}
