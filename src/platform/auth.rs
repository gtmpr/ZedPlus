use anyhow::{Context, Result};
use inquire::{Password, PasswordDisplayMode};
use reqwest::Client;
use serde::Deserialize;
use std::time::Duration;

pub const ANTHROPIC_KEYS_URL: &str = "https://console.anthropic.com/settings/keys";
pub const GOOGLE_AI_STUDIO_URL: &str = "https://aistudio.google.com/app/apikey";
pub const OPENAI_KEYS_URL: &str = "https://platform.openai.com/api-keys";
pub const OLLAMA_DEFAULT_URL: &str = "http://localhost:11434";
pub const LMSTUDIO_DEFAULT_URL: &str = "http://localhost:1234";

const VALIDATE_TIMEOUT: Duration = Duration::from_secs(10);

// ── Browser-assisted API key flow ────────────────────────────────────────────

/// Opens the provider's key management page, then loops prompting until valid.
pub async fn browser_assist_key(
    client: &Client,
    provider_name: &str,
    url: &str,
) -> Result<String> {
    println!("  Opening {} console...", provider_name);
    if let Err(e) = open::that_detached(url) {
        println!("  (Browser did not open automatically: {e})");
        println!("  Open manually: {url}");
    }
    prompt_until_valid(client, provider_name).await
}

/// Prompts for an API key without opening a browser.
pub async fn manual_key(client: &Client, provider_name: &str) -> Result<String> {
    prompt_until_valid(client, provider_name).await
}

async fn prompt_until_valid(client: &Client, provider_name: &str) -> Result<String> {
    loop {
        let key = Password::new(&format!("  {} API key:", provider_name))
            .without_confirmation()
            .with_display_mode(PasswordDisplayMode::Masked)
            .prompt()
            .context("Prompt cancelled")?;

        let key = key.trim().to_string();
        if key.is_empty() {
            println!("  Key cannot be empty.");
            continue;
        }

        print!("  Validating...");
        match validate_key(client, provider_name, &key).await {
            Ok(()) => {
                println!(" ✓");
                return Ok(key);
            }
            Err(e) => {
                println!(" ✗  {e}");
                println!("  Try again or press Ctrl+C to skip.");
            }
        }
    }
}

// ── Key validation ─────────────────────────────────────────────────────────

pub async fn validate_key(client: &Client, provider: &str, key: &str) -> Result<()> {
    match provider {
        "anthropic" => validate_anthropic(client, key).await,
        "google" => validate_google(client, key).await,
        "openai" => validate_openai(client, key).await,
        _ => Ok(()),
    }
}

async fn validate_anthropic(client: &Client, key: &str) -> Result<()> {
    let resp = client
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&serde_json::json!({
            "model": "claude-haiku-4-5-20251001",
            "max_tokens": 1,
            "messages": [{"role": "user", "content": "hi"}]
        }))
        .timeout(VALIDATE_TIMEOUT)
        .send()
        .await
        .context("Network error — check your internet connection")?;

    match resp.status().as_u16() {
        200 | 429 => Ok(()),
        401 => anyhow::bail!("Invalid API key"),
        403 => anyhow::bail!("API key lacks permissions"),
        s => {
            let body = resp.text().await.unwrap_or_default();
            let snippet = body.chars().take(120).collect::<String>();
            anyhow::bail!("Unexpected status {s}: {snippet}")
        }
    }
}

async fn validate_google(client: &Client, key: &str) -> Result<()> {
    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.0-flash:generateContent?key={key}"
    );
    let resp = client
        .post(&url)
        .header("content-type", "application/json")
        .json(&serde_json::json!({
            "contents": [{"parts": [{"text": "hi"}]}],
            "generationConfig": {"maxOutputTokens": 1}
        }))
        .timeout(VALIDATE_TIMEOUT)
        .send()
        .await
        .context("Network error — check your internet connection")?;

    match resp.status().as_u16() {
        200 | 429 => Ok(()),
        400 => {
            let body = resp.text().await.unwrap_or_default();
            if body.contains("API_KEY_INVALID") || body.contains("API key not valid") {
                anyhow::bail!("Invalid API key")
            } else {
                Ok(()) // other 400 means key is valid, e.g. quota exceeded
            }
        }
        401 | 403 => anyhow::bail!("Invalid or unauthorized API key"),
        s => {
            let body = resp.text().await.unwrap_or_default();
            let snippet = body.chars().take(120).collect::<String>();
            anyhow::bail!("Unexpected status {s}: {snippet}")
        }
    }
}

async fn validate_openai(client: &Client, key: &str) -> Result<()> {
    let resp = client
        .get("https://api.openai.com/v1/models")
        .bearer_auth(key)
        .timeout(VALIDATE_TIMEOUT)
        .send()
        .await
        .context("Network error — check your internet connection")?;

    match resp.status().as_u16() {
        200 | 429 => Ok(()),
        401 => anyhow::bail!("Invalid API key"),
        403 => anyhow::bail!("API key lacks permissions"),
        s => {
            let body = resp.text().await.unwrap_or_default();
            let snippet = body.chars().take(120).collect::<String>();
            anyhow::bail!("Unexpected status {s}: {snippet}")
        }
    }
}

/// Returns true if an LM Studio server is reachable at the given URL.
pub async fn check_lmstudio(client: &Client, base_url: &str) -> bool {
    client
        .get(format!("{base_url}/v1/models"))
        .timeout(Duration::from_secs(3))
        .send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false)
}

/// Returns the list of models loaded in a running LM Studio instance.
pub async fn lmstudio_models(client: &Client, base_url: &str) -> Vec<String> {
    #[derive(Deserialize)]
    struct ModelsResp {
        data: Vec<LmModel>,
    }
    #[derive(Deserialize)]
    struct LmModel {
        id: String,
    }

    let Ok(resp) = client
        .get(format!("{base_url}/v1/models"))
        .timeout(Duration::from_secs(3))
        .send()
        .await
    else {
        return vec![];
    };
    resp.json::<ModelsResp>()
        .await
        .map(|t| t.data.into_iter().map(|m| m.id).collect())
        .unwrap_or_default()
}

/// Returns true if an Ollama server is reachable at the given URL.
pub async fn check_ollama(client: &Client, base_url: &str) -> bool {
    client
        .get(format!("{base_url}/api/tags"))
        .timeout(Duration::from_secs(3))
        .send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false)
}

/// Returns the list of models pulled in a running Ollama instance.
pub async fn ollama_models(client: &Client, base_url: &str) -> Vec<String> {
    #[derive(Deserialize)]
    struct TagsResp {
        models: Vec<OllamaModel>,
    }
    #[derive(Deserialize)]
    struct OllamaModel {
        name: String,
    }

    let Ok(resp) = client
        .get(format!("{base_url}/api/tags"))
        .timeout(Duration::from_secs(3))
        .send()
        .await
    else {
        return vec![];
    };
    resp.json::<TagsResp>()
        .await
        .map(|t| t.models.into_iter().map(|m| m.name).collect())
        .unwrap_or_default()
}

// ── Google OAuth 2.0 Device flow (advanced) ─────────────────────────────────
// Requires a Google Cloud OAuth client_id with the Generative Language scope.
// Most users will use API keys from Google AI Studio instead.

const GOOGLE_DEVICE_ENDPOINT: &str = "https://oauth2.googleapis.com/device/code";
const GOOGLE_TOKEN_ENDPOINT: &str = "https://oauth2.googleapis.com/token";
const GOOGLE_SCOPE: &str = "https://www.googleapis.com/auth/generative-language";

#[derive(Deserialize)]
struct DeviceCodeResp {
    device_code: String,
    user_code: String,
    verification_url: String,
    expires_in: u64,
    interval: u64,
}

#[derive(Deserialize)]
struct TokenResp {
    access_token: Option<String>,
    refresh_token: Option<String>,
    error: Option<String>,
}

pub struct OAuthTokens {
    pub access_token: String,
    pub refresh_token: Option<String>,
}

/// Full OAuth 2.0 device authorization flow for Google AI.
/// `client_id` must be from a Google Cloud project with the Generative Language API enabled.
pub async fn google_oauth_device_flow(
    client: &Client,
    client_id: &str,
    client_secret: &str,
) -> Result<OAuthTokens> {
    // 1. Request device code
    let dc: DeviceCodeResp = client
        .post(GOOGLE_DEVICE_ENDPOINT)
        .form(&[
            ("client_id", client_id),
            ("scope", GOOGLE_SCOPE),
        ])
        .timeout(VALIDATE_TIMEOUT)
        .send()
        .await
        .context("Failed to request device code")?
        .json()
        .await
        .context("Failed to parse device code response")?;

    // 2. Show code + open browser
    println!("\n  Opening browser → {}", dc.verification_url);
    println!("  Code: {}  (expires in {}s)\n", dc.user_code, dc.expires_in);
    if let Err(e) = open::that_detached(&dc.verification_url) {
        println!("  (Could not open browser automatically: {e})");
    }

    // 3. Poll token endpoint
    let poll_interval = Duration::from_secs(dc.interval.max(5));
    let deadline = std::time::Instant::now() + Duration::from_secs(dc.expires_in);

    loop {
        tokio::time::sleep(poll_interval).await;
        if std::time::Instant::now() > deadline {
            anyhow::bail!("Device code expired — please try again");
        }

        let token: TokenResp = client
            .post(GOOGLE_TOKEN_ENDPOINT)
            .form(&[
                ("client_id", client_id),
                ("client_secret", client_secret),
                ("device_code", dc.device_code.as_str()),
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ])
            .timeout(VALIDATE_TIMEOUT)
            .send()
            .await
            .context("Token poll failed")?
            .json()
            .await
            .context("Failed to parse token response")?;

        match token.error.as_deref() {
            Some("authorization_pending") => {
                print!(".");
                continue;
            }
            Some("slow_down") => {
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue;
            }
            Some(e) => anyhow::bail!("OAuth error: {e}"),
            None => {}
        }

        if let Some(access_token) = token.access_token {
            println!(" ✓");
            return Ok(OAuthTokens {
                access_token,
                refresh_token: token.refresh_token,
            });
        }
    }
}
