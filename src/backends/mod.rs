use anyhow::Result;
use async_trait::async_trait;

pub mod claude;
pub mod claude_cli;
pub mod gemini;
pub mod gemini_cli;
pub mod ollama;
pub mod openai;

#[derive(Debug, thiserror::Error)]
pub enum BackendError {
    #[error("Rate limit exceeded — try again in a moment")]
    RateLimit,
    #[error("Request timed out")]
    Timeout,
    #[error("Authentication failed: {0}")]
    Auth(String),
    #[error("Backend error: {0}")]
    Other(#[from] anyhow::Error),
}

#[derive(Clone, Debug)]
pub struct Message {
    pub role: String,
    pub content: String,
}

pub struct CompletionOptions {
    pub model_id: String,
    pub system: Option<String>,
    pub messages: Vec<Message>,
    pub max_tokens: u32,
    pub use_search_grounding: bool,
    pub use_cache: bool,
    /// When true, CLI backends pass --yes to suppress their own prompts.
    pub auto_accept: bool,
}

impl Default for CompletionOptions {
    fn default() -> Self {
        Self {
            model_id: String::new(),
            system: None,
            messages: vec![],
            max_tokens: 4096,
            use_search_grounding: false,
            use_cache: false,
            auto_accept: false,
        }
    }
}

pub struct CompletionResult {
    pub content: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cache_hit: bool,
}

#[async_trait]
pub trait Backend: Send + Sync {
    fn name(&self) -> &str;

    /// Non-streaming: collects the full response before returning.
    async fn complete(&self, opts: CompletionOptions) -> Result<CompletionResult, BackendError>;

    /// Streaming: calls `on_token` for each text chunk as it arrives from the API.
    async fn complete_streaming(
        &self,
        opts: CompletionOptions,
        on_token: Box<dyn Fn(String) + Send>,
    ) -> Result<CompletionResult, BackendError>;

    /// Agentic step: sends messages with tool definitions and returns text + tool calls.
    async fn agent_step(
        &self,
        system: Option<&str>,
        messages: &[AgentMessage],
        tools: &[ToolDef],
        model_id: &str,
        max_tokens: u32,
    ) -> Result<AgentTurn, BackendError>;
}

// ── Agent types ───────────────────────────────────────────────────────────────

/// Definition of a tool the model can call.
pub struct ToolDef {
    pub name: &'static str,
    pub description: &'static str,
    pub parameters: serde_json::Value, // JSON Schema object
}

/// A single tool invocation requested by the model.
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub input: serde_json::Value,
}

/// A message in the agentic conversation.
/// A single turn can carry text, tool_calls (assistant → model requesting tools),
/// or tool_results (user → returning tool outputs).
pub struct AgentMessage {
    pub role: String, // "user" or "assistant"
    pub text: Option<String>,
    pub tool_calls: Vec<ToolCall>,                      // assistant blocks: tool_use
    pub tool_results: Vec<(String, String, bool)>,      // (call_id, content, is_error)
}

impl AgentMessage {
    pub fn user_text(s: impl Into<String>) -> Self {
        AgentMessage {
            role: "user".into(),
            text: Some(s.into()),
            tool_calls: vec![],
            tool_results: vec![],
        }
    }

    pub fn assistant(text: Option<String>, calls: Vec<ToolCall>) -> Self {
        AgentMessage {
            role: "assistant".into(),
            text,
            tool_calls: calls,
            tool_results: vec![],
        }
    }

    pub fn tool_results(results: Vec<(String, String, bool)>) -> Self {
        AgentMessage {
            role: "user".into(),
            text: None,
            tool_calls: vec![],
            tool_results: results,
        }
    }
}

/// The result of a single agentic step.
pub struct AgentTurn {
    pub text: Option<String>,
    pub tool_calls: Vec<ToolCall>,
    pub input_tokens: u32,
    pub output_tokens: u32,
}

/// Map a reqwest transport error to a `BackendError`.
pub fn map_reqwest_err(e: reqwest::Error) -> BackendError {
    if e.is_timeout() {
        return BackendError::Timeout;
    }
    if e.is_connect() {
        let url = e.url().map(|u| u.to_string()).unwrap_or_default();
        if url.contains("11434") {
            return BackendError::Other(anyhow::anyhow!(
                "Ollama is not running (connection refused on {url}).\n\
                 \n\
                 Options:\n\
                 \x20 • Start Ollama:            ollama serve\n\
                 \x20 • Use a cloud model:       zedplus ask --model claude-haiku \"...\"\n\
                 \x20 • Configure a cloud key:   zedplus auth --provider anthropic"
            ));
        }
        if url.contains("1234") {
            return BackendError::Other(anyhow::anyhow!(
                "LM Studio is not running (connection refused on {url}).\n\
                 \n\
                 Options:\n\
                 \x20 • Start LM Studio, load a model, enable the local server\n\
                 \x20 • Use a cloud model:       zedplus ask --model claude-haiku \"...\"\n\
                 \x20 • Configure a cloud key:   zedplus auth --provider anthropic"
            ));
        }
        return BackendError::Other(anyhow::anyhow!(
            "Connection refused: {url}\nCheck that the service is running."
        ));
    }
    if e.is_status() {
        if let Some(status) = e.status() {
            if status.as_u16() == 401 || status.as_u16() == 403 {
                return BackendError::Auth(format!("HTTP {status} — check your API key"));
            }
            if status.as_u16() == 429 {
                return BackendError::RateLimit;
            }
        }
    }
    BackendError::Other(e.into())
}

/// Resolve a short model alias (e.g. "claude-haiku") to `(provider, actual_model_id)`.
/// Returns `None` only when the alias is completely unrecognisable.
pub fn resolve_model(
    alias: &str,
    registry: &crate::config::models::ModelRegistry,
) -> Option<(String, String)> {
    // Exact key match first
    if let Some(m) = registry.get(alias) {
        return Some((m.provider.clone(), m.id.clone()));
    }
    // Prefix match: "gemini-flash" matches "gemini-flash-3-1" or "gemini-flash-2-5".
    // When multiple keys share the prefix, pick the lexicographically largest (highest version).
    let best = registry
        .models
        .iter()
        .filter(|(key, _)| key.starts_with(alias))
        .max_by_key(|(key, _)| key.as_str());
    if let Some((_, m)) = best {
        return Some((m.provider.clone(), m.id.clone()));
    }
    None
}

/// Instantiate the right backend for a given provider.
pub fn create_backend(provider: &str, api_key: &str, ollama_url: &str) -> Box<dyn Backend> {
    match provider {
        "claude" | "anthropic" => Box::new(claude::ClaudeBackend::new(api_key)),
        "claude-cli" => Box::new(claude_cli::ClaudeCliBackend::new("claude")),
        "gemini" | "google" => Box::new(gemini::GeminiBackend::new(api_key)),
        "gemini-cli" => Box::new(gemini_cli::GeminiCliBackend::new("gemini")),
        "openai" => Box::new(openai::OpenAiBackend::new(api_key)),
        "ollama" => Box::new(ollama::OllamaBackend::new(ollama_url)),
        "lmstudio" => Box::new(openai::OpenAiBackend::new_lmstudio(ollama_url)),
        _ => Box::new(claude::ClaudeBackend::new(api_key)),
    }
}
