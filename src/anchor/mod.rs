// Phase 13b: Goal Anchoring and Minimal Footprint
//
// Features:
// - Enforce scope (narrow vs broad)
// - Anchor original goal through entire session
// - Detect scope violations
// - Change confirmation gates
// - Multi-step task decomposition
// - Negative signal flagging for scope creep

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Scope enforcement: narrow (default) or broad
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum GoalScope {
    /// Answer only what was asked; note adjacent issues but don't fix them
    Narrow,
    /// Fix adjacent issues found while solving the goal
    Broad,
}

impl Default for GoalScope {
    fn default() -> Self {
        GoalScope::Narrow
    }
}

/// The original goal statement, verbatim, anchored for the entire session
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoalAnchor {
    /// First user message of the task
    pub original_query: String,
    /// Scope setting for this goal
    pub scope: GoalScope,
    /// Timestamp when goal was set (in seconds since Unix epoch)
    pub created_at: u64,
    /// Whether user has re-asked about scope violations
    pub scope_violation_reported: bool,
}

impl GoalAnchor {
    pub fn new(query: String, scope: GoalScope) -> Self {
        use std::time::{SystemTime, UNIX_EPOCH};
        
        let created_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        
        GoalAnchor {
            original_query: query,
            scope,
            created_at,
            scope_violation_reported: false,
        }
    }
    
    /// Generate system prompt injection for goal anchoring
    pub fn as_system_instruction(&self) -> String {
        let scope_instruction = match self.scope {
            GoalScope::Narrow => {
                "Answer only what was asked. If you notice adjacent issues, \
                 mention them but don't fix them. Minimal footprint."
            }
            GoalScope::Broad => {
                "You may fix adjacent issues found while solving the main goal."
            }
        };
        
        format!(
            "Original goal (anchor this for the entire session): {}\n\n\
             Scope: {}\n\
             {}",
            self.original_query,
            match self.scope {
                GoalScope::Narrow => "narrow",
                GoalScope::Broad => "broad",
            },
            scope_instruction
        )
    }
}

/// Change that would be applied by an AI response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProposedChange {
    pub file_path: String,
    pub change_type: ChangeType,
    pub diff: String,
    pub related_to_goal: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ChangeType {
    Modify,
    Create,
    Delete,
}

/// Confirmation gate for applying changes
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangeConfirmation {
    pub changes: Vec<ProposedChange>,
    /// Whether all changes directly address the original goal
    pub all_in_scope: bool,
    /// Out-of-scope changes (scope violations in narrow mode)
    pub out_of_scope: Vec<ProposedChange>,
}

impl ChangeConfirmation {
    pub fn new(changes: Vec<ProposedChange>, scope: &GoalScope) -> Self {
        let out_of_scope = if *scope == GoalScope::Narrow {
            changes
                .iter()
                .filter(|c| !c.related_to_goal)
                .cloned()
                .collect()
        } else {
            Vec::new()
        };
        
        let all_in_scope = out_of_scope.is_empty();
        
        ChangeConfirmation {
            changes,
            all_in_scope,
            out_of_scope,
        }
    }
    
    /// Generate user-facing confirmation prompt
    pub fn render_prompt(&self) -> String {
        let mut prompt = String::new();
        
        if !self.all_in_scope {
            prompt.push_str(
                "⚠️  Some changes are outside the original goal (scope: narrow)\n\n"
            );
            for change in &self.out_of_scope {
                prompt.push_str(&format!(
                    "Out of scope: {} ({})\n",
                    change.file_path, 
                    match change.change_type {
                        ChangeType::Modify => "modify",
                        ChangeType::Create => "create",
                        ChangeType::Delete => "delete",
                    }
                ));
            }
            prompt.push('\n');
        }
        
        prompt.push_str("Changes to apply:\n\n");
        for change in &self.changes {
            prompt.push_str(&format!(
                "{} {} ({})\n",
                match change.change_type {
                    ChangeType::Create => "📝 create",
                    ChangeType::Modify => "✏️  modify",
                    ChangeType::Delete => "🗑️  delete",
                },
                change.file_path,
                if change.related_to_goal { "in-scope" } else { "out-of-scope" }
            ));
        }
        
        prompt.push_str("\n[Y] apply all | [n] skip | [e] edit | [?] show diffs: ");
        prompt
    }
}

/// Multi-step task decomposition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskPlan {
    pub steps: Vec<TaskStep>,
    pub total_estimated_tokens: u32,
    pub total_estimated_cost_usd: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskStep {
    pub number: usize,
    pub description: String,
    pub estimated_tokens: u32,
    pub estimated_cost_usd: f64,
    pub requires_approval: bool,
}

impl TaskPlan {
    pub fn render_for_approval(&self) -> String {
        let mut output = String::from("📋 Task Plan\n\n");
        
        for step in &self.steps {
            output.push_str(&format!(
                "Step {}: {}\n  Estimate: {} tokens (~${:.4})\n\n",
                step.number,
                step.description,
                step.estimated_tokens,
                step.estimated_cost_usd
            ));
        }
        
        output.push_str(&format!(
            "Total: {} tokens (~${:.4})\n\n\
             [A] approve all | [s] step by step | [c] cancel: ",
            self.total_estimated_tokens,
            self.total_estimated_cost_usd
        ));
        
        output
    }
}

/// Negative signal: user re-asked about something, indicating scope violation or poor response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScopeViolationSignal {
    pub session_id: String,
    pub turn_number: u32,
    pub original_response: String,
    pub user_concern: String,
    /// Files or areas that were unexpectedly changed
    pub unexpected_changes: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_goal_anchor_narrow() {
        let anchor = GoalAnchor::new(
            "Add a login function".to_string(),
            GoalScope::Narrow,
        );
        
        let instr = anchor.as_system_instruction();
        assert!(instr.contains("Answer only what was asked"));
        assert!(instr.contains("Add a login function"));
    }
    
    #[test]
    fn test_change_confirmation_scope() {
        let changes = vec![
            ProposedChange {
                file_path: "auth.rs".to_string(),
                change_type: ChangeType::Modify,
                diff: "".to_string(),
                related_to_goal: true,
            },
            ProposedChange {
                file_path: "logging.rs".to_string(),
                change_type: ChangeType::Modify,
                diff: "".to_string(),
                related_to_goal: false,
            },
        ];
        
        let confirmation = ChangeConfirmation::new(changes, &GoalScope::Narrow);
        assert!(!confirmation.all_in_scope);
        assert_eq!(confirmation.out_of_scope.len(), 1);
    }
    
    #[test]
    fn test_task_plan_rendering() {
        let plan = TaskPlan {
            steps: vec![
                TaskStep {
                    number: 1,
                    description: "Analyze requirements".to_string(),
                    estimated_tokens: 100,
                    estimated_cost_usd: 0.01,
                    requires_approval: false,
                },
            ],
            total_estimated_tokens: 100,
            total_estimated_cost_usd: 0.01,
        };
        
        let rendered = plan.render_for_approval();
        assert!(rendered.contains("Step 1"));
        assert!(rendered.contains("100 tokens"));
    }
}
