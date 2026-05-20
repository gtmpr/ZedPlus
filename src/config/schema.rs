use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub locale: LocaleConfig,

    #[serde(default)]
    pub routing: RoutingConfig,

    #[serde(default)]
    pub privacy: PrivacyConfig,

    #[serde(default)]
    pub behavior: BehaviorConfig,

    #[serde(default)]
    pub training: TrainingConfig,

    #[serde(default)]
    pub sessions: SessionsConfig,

    #[serde(default)]
    pub testing: TestingConfig,

    #[serde(default)]
    pub hooks: HooksConfig,

    #[serde(default)]
    pub services: ServicesConfig,

    #[serde(default)]
    pub update: UpdateConfig,

    #[serde(default)]
    pub multimodal: MultimodalConfig,

    #[serde(default)]
    pub skills: SkillsConfig,

    #[serde(default)]
    pub pipeline: PipelineConfig,

    #[serde(default)]
    pub persona: PersonaConfig,

    #[serde(default)]
    pub brainstorm: BrainstormConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocaleConfig {
    pub country: String,
    pub timezone: String,
    pub language: String,
    pub date_format: String,
    pub units: String,
    pub currency: String,
}

impl Default for LocaleConfig {
    fn default() -> Self {
        Self {
            country: "US".to_string(),
            timezone: "UTC".to_string(),
            language: "en-US".to_string(),
            date_format: "MM/DD/YYYY".to_string(),
            units: "imperial".to_string(),
            currency: "USD".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RoutingConfig {
    #[serde(default)]
    pub priority: RoutingPriority,

    #[serde(default)]
    pub rules: RoutingRules,

    #[serde(default)]
    pub fallback_chain: FallbackChain,

    #[serde(default)]
    pub overrides: HashMap<String, String>,

    #[serde(default)]
    pub architect_editor: ArchitectEditorConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum RoutingPriority {
    #[default]
    Balanced,
    Quality,
    Cost,
    LocalFirst,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingRules {
    pub web_search: String,
    pub quick_completion: String,
    pub code_review: String,
    pub complex_reasoning: String,
    pub data_analysis: String,
    pub documentation: String,
    pub fallback: String,
}

impl Default for RoutingRules {
    fn default() -> Self {
        Self {
            web_search: "gemini-flash".to_string(),
            quick_completion: "local".to_string(),
            code_review: "claude-sonnet".to_string(),
            complex_reasoning: "claude-sonnet".to_string(),
            data_analysis: "gemini-pro".to_string(),
            documentation: "claude-haiku".to_string(),
            fallback: "claude-haiku".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FallbackChain {
    pub local_failure: String,
    pub timeout_secs: u64,
}

impl Default for FallbackChain {
    fn default() -> Self {
        Self {
            local_failure: "claude-haiku".to_string(),
            timeout_secs: 30,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchitectEditorConfig {
    pub enabled: bool,
    pub architect_model: String,
    pub editor_model: String,
    pub threshold_lines: u32,
}

impl Default for ArchitectEditorConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            architect_model: "claude-sonnet".to_string(),
            editor_model: "claude-haiku".to_string(),
            threshold_lines: 50,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PrivacyConfig {
    pub cloud_allowed: Option<bool>,
    pub clipboard_detection: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BehaviorConfig {
    pub default_scope: Scope,
    pub stream: bool,
    pub cost_nudge_threshold_usd: f64,
}

impl Default for BehaviorConfig {
    fn default() -> Self {
        Self {
            default_scope: Scope::Narrow,
            stream: true,
            cost_nudge_threshold_usd: 0.50,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    #[default]
    Narrow,
    Broad,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrainingConfig {
    pub auto_train: bool,
    pub auto_train_min_new: u32,
    pub auto_train_schedule: TrainingSchedule,
    /// Thresholds for a session to be considered "significant" for training
    pub significance_thresholds: SignificanceThresholds,
    pub lora_rank: u32,
    pub lora_alpha: u32,
    /// Preference for training environment
    pub environment: TrainingEnvironment,
    /// Primary use case for training (General, Coding, Writing)
    pub primary_use: TrainingUse,
}

impl Default for TrainingConfig {
    fn default() -> Self {
        Self {
            auto_train: false,
            auto_train_min_new: 200,
            auto_train_schedule: TrainingSchedule::Volume,
            significance_thresholds: SignificanceThresholds::default(),
            lora_rank: 16,
            lora_alpha: 32,
            environment: TrainingEnvironment::Auto,
            primary_use: TrainingUse::General,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignificanceThresholds {
    pub min_cost_usd: f64,
    pub min_files_written: u32,
    pub min_turns: u32,
}

impl Default for SignificanceThresholds {
    fn default() -> Self {
        Self {
            min_cost_usd: 1.0,
            min_files_written: 3,
            min_turns: 20,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum TrainingEnvironment {
    #[default]
    Auto,
    Docker,
    Venv,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum TrainingUse {
    #[default]
    General,
    Coding,
    Writing,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum TrainingSchedule {
    #[default]
    Volume,
    Weekly,
    Manual,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionsConfig {
    pub auto_resume_threshold_hours: u32,
    pub max_resume_candidates: u32,
}

impl Default for SessionsConfig {
    fn default() -> Self {
        Self {
            auto_resume_threshold_hours: 24,
            max_resume_candidates: 3,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestingConfig {
    pub auto_run: bool,
    pub runner: String,
    pub suggest_tests: bool,
    pub run_benchmarks: bool,
    pub snapshot_dir: String,
}

impl Default for TestingConfig {
    fn default() -> Self {
        Self {
            auto_run: true,
            runner: "auto".to_string(),
            suggest_tests: true,
            run_benchmarks: false,
            snapshot_dir: ".zedplus/snapshots".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HooksConfig {
    pub before_apply_change: Option<String>,
    pub after_apply_change: Option<String>,
    pub before_commit: Option<String>,
    pub after_commit: Option<String>,
    pub before_session: Option<String>,
    pub after_session: Option<String>,
    pub before_search: Option<String>,
    pub before_cloud_send: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ServicesConfig {
    pub anthropic: bool,
    pub google: bool,
    pub openai: bool,
    pub ollama: bool,
    pub ollama_url: Option<String>,
    pub lmstudio: bool,
    pub lmstudio_url: Option<String>,
    pub use_cases: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateConfig {
    pub check_on_startup: bool,
    pub auto_install: bool,
}

impl Default for UpdateConfig {
    fn default() -> Self {
        Self {
            check_on_startup: true,
            auto_install: false,
        }
    }
}

/// Phase 13a: Multimodal Configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MultimodalConfig {
    /// Enable vision (image) support
    pub enable_vision: bool,
    /// Enable PDF document support
    pub enable_pdf: bool,
    /// Max image file size in MB
    pub max_image_size_mb: u32,
    /// Max PDF file size in MB
    pub max_pdf_size_mb: u32,
    /// Allow clipboard image detection
    pub clipboard_images: bool,
}

impl Default for MultimodalConfig {
    fn default() -> Self {
        Self {
            enable_vision: true,
            enable_pdf: true,
            max_image_size_mb: 20,
            max_pdf_size_mb: 50,
            clipboard_images: false,
        }
    }
}

/// Per-phase model preferences — tried before the automatic cascade.
/// Each entry is a model alias from models.toml (e.g. "gemini-pro-3-1").
/// Leave empty to use the automatic cascade.
///
/// Example in ~/.config/zedplus/config.toml:
///   [pipeline]
///   reasoning = ["gemini-pro-3-1", "claude-sonnet-4-5"]
///   planning  = ["gemini-flash-2-5", "claude-haiku-4-5"]
///   execution = ["qwen-local", "claude-haiku-4-5"]
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PipelineConfig {
    #[serde(default)]
    pub reasoning: Vec<String>,
    #[serde(default)]
    pub planning: Vec<String>,
    #[serde(default)]
    pub execution: Vec<String>,
}

/// Phase 8b: Developer Personas Configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersonaConfig {
    /// Persona active at session start (empty = none)
    pub default_persona: String,
    /// Whether to show persona reminder in the prompt
    pub show_in_prompt: bool,
}

impl Default for PersonaConfig {
    fn default() -> Self {
        Self {
            default_persona: String::new(),
            show_in_prompt: true,
        }
    }
}

/// Phase 8c: Multi-agent Brainstorm Configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrainstormConfig {
    /// Default strategy when /debate is used without a strategy prefix
    pub default_strategy: String,
    /// Jaccard similarity threshold for convergence detection
    pub convergence_threshold: f64,
    /// Max rounds for Delphi strategy
    pub max_delphi_rounds: u32,
}

impl Default for BrainstormConfig {
    fn default() -> Self {
        Self {
            default_strategy: "debate".to_string(),
            convergence_threshold: 0.62,
            max_delphi_rounds: 3,
        }
    }
}

/// Phase 13c: Skill Packs Configuration
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SkillsConfig {
    /// Enable skill pack system
    pub enabled: bool,
    /// Active skill packs
    pub active: Vec<String>,
    /// Auto-suggest packs based on usage
    pub auto_suggest: bool,
    /// Show skill pack info in routing explain
    pub show_in_explain: bool,
}
