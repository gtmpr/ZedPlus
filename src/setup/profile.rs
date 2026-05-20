use crate::config::schema::{RoutingPriority, TrainingConfig, TrainingSchedule};
use crate::setup::detector::LocalLlmVerdict;
use anyhow::Result;
use inquire::{MultiSelect, Select};

#[derive(Debug, Clone)]
pub struct UserProfile {
    pub use_cases: Vec<String>,
    pub priority: RoutingPriority,
}

const USE_CASE_OPTIONS: &[&str] = &[
    "Web development (React, Vue, Node, APIs)",
    "Backend / systems / low-level",
    "Mobile development (iOS / Android / Flutter)",
    "Data analysis / ML / notebooks",
    "DevOps / infra / scripts",
    "Writing / documentation",
];

pub fn prompt_use_cases() -> Result<Vec<String>> {
    let selected = MultiSelect::new(
        "What do you primarily use AI for?  (pick all that apply)",
        USE_CASE_OPTIONS.iter().map(|s| s.to_string()).collect(),
    )
    .with_help_message("Space to toggle, Enter to confirm")
    .prompt()?;
    Ok(selected)
}

pub fn prompt_routing_priority() -> Result<RoutingPriority> {
    let options = vec![
        "Balanced          — quality + cost (recommended for most users)",
        "Highest quality   — cost is secondary",
        "Lowest cost       — limited credits",
        "Local first       — privacy / offline",
    ];

    let choice = Select::new("What's your routing priority?", options).prompt()?;

    let priority = if choice.starts_with("Balanced") {
        RoutingPriority::Balanced
    } else if choice.starts_with("Highest") {
        RoutingPriority::Quality
    } else if choice.starts_with("Lowest") {
        RoutingPriority::Cost
    } else {
        RoutingPriority::LocalFirst
    };

    Ok(priority)
}

pub fn prompt_auto_train(verdict: &LocalLlmVerdict) -> Result<TrainingConfig> {
    if !verdict.can_train_lora() {
        // Device can't train — still explain why, return disabled config
        match verdict {
            LocalLlmVerdict::Disabled { reason } => {
                println!(
                    "  ⚠  Auto-training disabled: {reason}."
                );
                println!(
                    "     Distillation data will still accumulate for external training."
                );
            }
            LocalLlmVerdict::CpuOnly { .. } => {
                println!(
                    "  ⚠  Auto-training requires a GPU with ≥6 GB VRAM."
                );
                println!(
                    "     Distillation data will still accumulate — use `zedplus train` with an external GPU."
                );
            }
            _ => {}
        }
        return Ok(TrainingConfig { auto_train: false, ..Default::default() });
    }

    let options = vec![
        "Yes — auto-train when I accumulate 200+ new conversations (recommended)",
        "Yes — auto-train weekly regardless of volume",
        "No  — I'll trigger training manually with `zedplus train`",
    ];

    let choice = Select::new(
        "Local model auto-training\nZedPlus can improve your local model by fine-tuning on your\nconversations during idle periods. Opt in?",
        options,
    )
    .prompt()?;

    let (auto_train, schedule) = if choice.starts_with("Yes — auto-train when") {
        (true, TrainingSchedule::Volume)
    } else if choice.starts_with("Yes — auto-train weekly") {
        (true, TrainingSchedule::Weekly)
    } else {
        (false, TrainingSchedule::Manual)
    };

    Ok(TrainingConfig {
        auto_train,
        auto_train_schedule: schedule,
        ..Default::default()
    })
}
