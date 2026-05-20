use crate::config::schema::Config;
use crate::router::classifier::TaskType;

pub struct ArchitectEligibility {
    pub is_eligible: bool,
    pub reason: Option<String>,
}

/// Determine if a query is eligible for Architect/Editor mode split.
pub fn check_eligibility(
    query: &str,
    task_type: &TaskType,
    config: &Config,
) -> ArchitectEligibility {
    if !config.routing.architect_editor.enabled {
        return ArchitectEligibility {
            is_eligible: false,
            reason: None,
        };
    }

    // Architect mode is primary for code-modifying tasks
    let is_code_task = matches!(
        task_type,
        TaskType::CodeReview | TaskType::ComplexReasoning
    );

    if !is_code_task {
        return ArchitectEligibility {
            is_eligible: false,
            reason: Some("Task type not eligible for architect split".to_string()),
        };
    }

    // Heuristic: If query length > threshold or mentions multiple files/features
    let line_count = query.lines().count();
    let threshold = config.routing.architect_editor.threshold_lines as usize;

    if line_count >= threshold {
        return ArchitectEligibility {
            is_eligible: true,
            reason: Some(format!("Query length ({}) exceeds threshold ({})", line_count, threshold)),
        };
    }

    // Also check for multi-file keywords
    let keywords = ["refactor", "implement", "add feature", "architecture", "restructure"];
    let query_lower = query.to_lowercase();
    if keywords.iter().any(|k| query_lower.contains(k)) {
        return ArchitectEligibility {
            is_eligible: true,
            reason: Some("Task complexity suggests architectural planning".to_string()),
        };
    }

    ArchitectEligibility {
        is_eligible: false,
        reason: Some("Task below complexity threshold".to_string()),
    }
}
