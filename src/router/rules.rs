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

    match config.routing.priority {
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
    }
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
