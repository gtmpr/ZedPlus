use crate::platform::auth;
use anyhow::Result;
use inquire::{MultiSelect, Select};
use reqwest::Client;
use std::collections::HashMap;

#[derive(Debug, Clone, Default)]
pub struct SelectedServices {
    pub anthropic: bool,
    pub google: bool,
    pub openai: bool,
    pub ollama: bool,
    pub ollama_url: String,
    pub ollama_models: Vec<String>,
    pub lmstudio: bool,
    pub lmstudio_url: String,
    pub lmstudio_models: Vec<String>,
}

impl SelectedServices {
    pub fn any_cloud(&self) -> bool {
        self.anthropic || self.google || self.openai
    }

    pub fn providers(&self) -> Vec<&'static str> {
        let mut v = Vec::new();
        if self.anthropic { v.push("anthropic"); }
        if self.google    { v.push("google"); }
        if self.openai    { v.push("openai"); }
        if self.ollama    { v.push("ollama"); }
        if self.lmstudio  { v.push("lmstudio"); }
        v
    }
}

pub async fn prompt_services(client: &Client) -> Result<SelectedServices> {
    let ollama_url = auth::OLLAMA_DEFAULT_URL;
    let lmstudio_url = auth::LMSTUDIO_DEFAULT_URL;

    // Probe local servers in parallel
    let (ollama_running, lmstudio_running) = tokio::join!(
        auth::check_ollama(client, ollama_url),
        auth::check_lmstudio(client, lmstudio_url),
    );

    let options = vec![
        format!("Anthropic (Claude)       — best for complex code & reasoning"),
        format!("Google AI (Gemini)       — best for web search + data analysis"),
        format!("OpenAI (GPT-4o)          — broad ecosystem compatibility"),
        format!(
            "Ollama (local/free)      — {}",
            if ollama_running { "detected running ✓" } else { "not detected (start with: ollama serve)" }
        ),
        format!(
            "LM Studio (local/free)   — {}",
            if lmstudio_running { "detected running ✓" } else { "not detected (start LM Studio + enable server)" }
        ),
    ];

    let selected = MultiSelect::new("Which AI services do you have access to?", options)
        .with_default(&[0, 1]) // Anthropic + Google preselected
        .with_help_message("Space to toggle, Enter to confirm — deselect all to skip")
        .prompt()
        .unwrap_or_default();

    let mut svc = SelectedServices {
        ollama_url: ollama_url.to_string(),
        lmstudio_url: lmstudio_url.to_string(),
        ..Default::default()
    };

    for s in &selected {
        if s.starts_with("Anthropic") { svc.anthropic = true; }
        if s.starts_with("Google")    { svc.google = true; }
        if s.starts_with("OpenAI")    { svc.openai = true; }
        if s.starts_with("Ollama")    { svc.ollama = true; }
        if s.starts_with("LM Studio") { svc.lmstudio = true; }
    }

    if svc.ollama {
        svc.ollama_models = auth::ollama_models(client, &svc.ollama_url).await;
        if !ollama_running {
            println!("  ⚠  Ollama is not running. Start it with `ollama serve` before using local models.");
        } else if svc.ollama_models.is_empty() {
            println!("  ⚠  Ollama is running but no models are pulled.");
            println!("     Pull one with: ollama pull llama3.2:8b");
        } else {
            println!("  Ollama models: {}", svc.ollama_models.join(", "));
        }
    }

    if svc.lmstudio {
        svc.lmstudio_models = auth::lmstudio_models(client, &svc.lmstudio_url).await;
        if !lmstudio_running {
            println!("  ⚠  LM Studio server not running. Load a model and enable the local server in LM Studio.");
        } else if svc.lmstudio_models.is_empty() {
            println!("  ⚠  LM Studio is running but no models are loaded.");
        } else {
            println!("  LM Studio models: {}", svc.lmstudio_models.join(", "));
        }
    }

    Ok(svc)
}

/// Authenticate each selected cloud provider, return provider → key map.
pub async fn configure_all_services(
    svc: &SelectedServices,
    client: &Client,
) -> Result<HashMap<String, String>> {
    let mut keys: HashMap<String, String> = HashMap::new();

    if svc.anthropic {
        println!("\n  ── Anthropic (Claude) ──────────────────────────────────────");
        match configure_provider(client, "anthropic").await {
            Ok(key) => { keys.insert("anthropic".into(), key); }
            Err(e) => println!("  Skipped: {e}"),
        }
    }

    if svc.google {
        println!("\n  ── Google AI (Gemini) ──────────────────────────────────────");
        match configure_provider(client, "google").await {
            Ok(key) => { keys.insert("google".into(), key); }
            Err(e) => println!("  Skipped: {e}"),
        }
    }

    if svc.openai {
        println!("\n  ── OpenAI (GPT-4o) ─────────────────────────────────────────");
        match configure_provider(client, "openai").await {
            Ok(key) => { keys.insert("openai".into(), key); }
            Err(e) => println!("  Skipped: {e}"),
        }
    }

    Ok(keys)
}

async fn configure_provider(client: &Client, provider: &str) -> Result<String> {
    let (display_name, url) = match provider {
        "anthropic" => ("Anthropic", auth::ANTHROPIC_KEYS_URL),
        "google"    => ("Google AI Studio", auth::GOOGLE_AI_STUDIO_URL),
        "openai"    => ("OpenAI", auth::OPENAI_KEYS_URL),
        p => (p, ""),
    };

    let choice = Select::new(
        &format!("  How would you like to authenticate with {}?", display_name),
        vec![
            "[B] Open browser → paste key",
            "[M] Manual entry (paste key directly)",
            "[S] Skip for now",
        ],
    )
    .prompt()?;

    if choice.starts_with("[B]") {
        auth::browser_assist_key(client, display_name, url).await
    } else if choice.starts_with("[S]") {
        anyhow::bail!("skipped")
    } else {
        auth::manual_key(client, display_name).await
    }
}
