use clap::{Parser, Subcommand, Args};

#[derive(Parser)]
#[command(
    name = "zedplus",
    about = "Smart AI routing CLI with realtime code indexing and response distillation",
    version,
    long_about = None,
)]
pub struct Cli {
    /// Open the REPL with an optional pre-loaded query
    pub query: Option<String>,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand)]
pub enum Command {
    /// First-time setup wizard — configure AI services, routing, and local LLM
    Init(InitArgs),

    /// Re-authenticate a provider (or all providers)
    Auth(AuthArgs),

    /// Watch a directory and build the code index
    Index(IndexArgs),

    /// Send a query non-interactively (for scripts and CI)
    Ask(AskArgs),

    /// Force a web search via Gemini Search grounding
    Search(SearchArgs),

    /// Resume the most recent session in the current directory
    Resume,

    /// Clear the current session context (distillation data is preserved)
    Clear,

    /// Show cost and token usage reports
    Usage(UsageArgs),

    /// Export distillation JSONL with optional filters
    Distill(DistillArgs),

    /// Trigger or monitor local model training
    Train(TrainArgs),

    /// Benchmark a local model against a baseline
    Bench(BenchArgs),

    /// Manage models (list, add, import)
    Model(ModelArgs),

    /// Analyze usage patterns and suggest routing optimizations
    Profile(ProfileArgs),

    /// Show, edit, or reset configuration
    Config(ConfigArgs),

    /// Check for or install ZedPlus binary updates
    Update(UpdateArgs),

    /// Generate and execute a shell command from natural language
    Shell(ShellArgs),

    /// Manage sessions (list, resume, rename, archive)
    Session(SessionArgs),

    /// Manage skill packs (list, install, suggest, create)
    Skills(SkillsArgs),
}

#[derive(Args)]
pub struct InitArgs {
    /// Generate a ZEDPLUS.md context file from the codebase index
    #[arg(long)]
    pub context: bool,
}

#[derive(Args)]
pub struct AuthArgs {
    /// Provider to authenticate (anthropic, google, openai, ollama)
    #[arg(long)]
    pub provider: Option<String>,

    /// Revoke stored credentials for a provider
    #[arg(long, value_name = "PROVIDER")]
    pub revoke: Option<String>,
}

#[derive(Args)]
pub struct IndexArgs {
    /// Directory to index (defaults to current directory)
    pub path: Option<std::path::PathBuf>,

    /// Reset the index and re-index from scratch
    #[arg(long)]
    pub reset: bool,
}

#[derive(Args)]
pub struct AskArgs {
    /// The query to send
    pub query: String,

    /// Override the model for this query
    #[arg(long)]
    pub model: Option<String>,

    /// Force local model for this query
    #[arg(long)]
    pub local: bool,

    /// Force the cheapest available model
    #[arg(long)]
    pub cheap: bool,

    /// Show routing decision and cost estimate
    #[arg(long)]
    pub explain: bool,

    /// Disable streaming — collect full response before printing
    #[arg(long)]
    pub no_stream: bool,

    /// Attach an image file (vision models)
    #[arg(long, value_name = "FILE")]
    pub image: Option<std::path::PathBuf>,

    /// Attach a file (PDF, CSV, or plain text)
    #[arg(long, value_name = "FILE")]
    pub file: Option<std::path::PathBuf>,

    /// Scope: narrow (default) or broad
    #[arg(long, default_value = "narrow")]
    pub scope: String,

    /// Force architect/editor mode
    #[arg(long)]
    pub architect: bool,

    /// Disable architect/editor mode
    #[arg(long)]
    pub no_architect: bool,

    /// Disable all interactive prompts (CI/headless mode)
    #[arg(long)]
    pub no_interactive: bool,

    /// Output format: terminal (default), json, plain
    #[arg(long, default_value = "terminal")]
    pub output: String,

    /// Exit 1 if AI response contains warnings/errors
    #[arg(long)]
    pub exit_code: bool,

    /// Extract code blocks from the response and offer to apply them to files
    #[arg(long)]
    pub apply: bool,

    /// Run in agentic mode with tool use (read/write files, run commands, search)
    #[arg(long)]
    pub agent: bool,

    /// Auto-accept all write_file and run_command confirmations (like --dangerously-skip-permissions)
    #[arg(long)]
    pub yes: bool,
}

#[derive(Args)]
pub struct SearchArgs {
    /// The search query
    pub query: String,

    /// Disable streaming
    #[arg(long)]
    pub no_stream: bool,
}

#[derive(Args)]
pub struct UsageArgs {
    /// Show today's usage only
    #[arg(long)]
    pub today: bool,

    /// Show this month's usage
    #[arg(long)]
    pub month: bool,

    /// Filter by project path
    #[arg(long)]
    pub project: Option<String>,
}

#[derive(Args)]
pub struct DistillArgs {
    /// Output file (defaults to stdout)
    #[arg(long, short)]
    pub out: Option<std::path::PathBuf>,

    /// Output format: alpaca (default) or sharegpt
    #[arg(long, default_value = "alpaca")]
    pub format: String,

    /// Apply recency weighting (30d=1.0x, 90d=0.5x, older=0.25x)
    #[arg(long)]
    pub weighted: bool,

    /// Filter by task type
    #[arg(long)]
    pub task: Option<String>,

    /// Filter by model
    #[arg(long)]
    pub model: Option<String>,

    /// Filter by date (ISO 8601)
    #[arg(long)]
    pub since: Option<String>,

    /// Opt-in anonymized community contribution export
    #[arg(long)]
    pub export_community: bool,

    /// Review before community export
    #[arg(long)]
    pub review: bool,
}

#[derive(Args)]
pub struct TrainArgs {
    /// Base model identifier
    #[arg(long)]
    pub base: Option<String>,

    /// Training data file (JSONL)
    #[arg(long)]
    pub data: Option<std::path::PathBuf>,

    /// LoRA fine-tuning (default)
    #[arg(long)]
    pub lora: bool,

    /// Full fine-tuning (slow, high VRAM)
    #[arg(long)]
    pub full: bool,

    /// Show status of a background training job
    #[arg(long)]
    pub status: bool,

    /// Automatically run a benchmark against the baseline after training
    #[arg(long)]
    pub bench: bool,
}

#[derive(Args)]
pub struct BenchArgs {
    /// Model to evaluate
    #[arg(long)]
    pub model: Option<String>,

    /// Baseline model to compare against
    #[arg(long)]
    pub baseline: Option<String>,

    /// Number of distillation samples to benchmark (default: 50)
    #[arg(long, default_value_t = 50)]
    pub samples: usize,

    /// Show historical benchmark results from the DB instead of running a new benchmark
    #[arg(long)]
    pub history: bool,
}

#[derive(Args)]
pub struct ModelArgs {
    #[command(subcommand)]
    pub command: ModelCommand,
}

#[derive(Subcommand)]
pub enum ModelCommand {
    /// List all known models with capabilities
    List,

    /// Scaffold a models.toml entry for a new model
    Add {
        provider: String,
        model_id: String,
    },

    /// Register a local or LoRA model
    Import {
        /// Path or Ollama model ID
        source: String,
        /// User-facing alias
        #[arg(long)]
        name: String,
    },

    /// List community LoRA adapters (v2)
    #[command(name = "adapters")]
    Adapters(AdaptersArgs),

    /// Show model reliability leaderboard (test pass rate, negative signals, override frequency)
    Rank,
}

#[derive(Args)]
pub struct AdaptersArgs {
    #[command(subcommand)]
    pub command: AdaptersCommand,
}

#[derive(Subcommand)]
pub enum AdaptersCommand {
    List,
    Install { name: String },
}

#[derive(Args)]
pub struct ProfileArgs {
    /// Analyze usage and suggest routing changes
    #[arg(long)]
    pub optimize: bool,

    /// Apply suggested changes without prompting
    #[arg(long)]
    pub apply: bool,
}

#[derive(Args)]
pub struct ConfigArgs {
    /// Show current configuration
    #[arg(long)]
    pub show: bool,

    /// Open config in $EDITOR
    #[arg(long)]
    pub edit: bool,

    /// Reset config to defaults
    #[arg(long)]
    pub reset: bool,

    /// Set a config value (e.g. routing.rules.code_review=claude-opus)
    #[arg(long, value_name = "KEY=VALUE")]
    pub set: Option<String>,
}

#[derive(Args)]
pub struct UpdateArgs {
    /// Check for a newer version without installing
    #[arg(long)]
    pub check: bool,
}

#[derive(Args)]
pub struct ShellArgs {
    /// Natural language description of the command
    pub description: String,

    /// Return just the command string (for shell integration)
    #[arg(long)]
    pub inline: bool,

    /// Install shell hotkey integration
    #[arg(long)]
    pub install_hotkey: bool,
}

#[derive(Args)]
pub struct SessionArgs {
    #[command(subcommand)]
    pub command: SessionCommand,
}

#[derive(Subcommand)]
pub enum SessionCommand {
    /// List sessions for current project
    List {
        /// Show sessions across all projects
        #[arg(long)]
        all: bool,
    },
    /// Resume a session by name
    Resume { name: String },
    /// Rename a session
    Rename { old: String, new: String },
    /// Archive a session (hide from resume list)
    Archive { name: String },
}

#[derive(Args)]
pub struct SkillsArgs {
    #[command(subcommand)]
    pub command: SkillsCommand,
}

#[derive(Subcommand)]
pub enum SkillsCommand {
    /// List installed and available skill packs
    List,
    /// Install a skill pack
    Install { name: String },
    /// Suggest skill packs based on your usage
    Suggest,
    /// Scaffold a custom skill pack
    Create {
        #[arg(long)]
        name: String,
    },
}
