use crate::backends::{
    map_reqwest_err, AgentMessage, AgentTurn, Backend, BackendError, CompletionOptions,
    CompletionResult, ToolDef,
};
use async_trait::async_trait;
use futures::StreamExt;
use reqwest::Client;
use serde_json::{json, Value};

pub struct OllamaBackend {
    client: Client,
    base_url: String,
}

impl OllamaBackend {
    pub fn new(base_url: &str) -> Self {
        let base_url = base_url.trim_end_matches('/').to_string();
        Self {
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(120))
                .build()
                .expect("reqwest client"),
            base_url,
        }
    }

    fn chat_url(&self) -> String {
        format!("{}/api/chat", self.base_url)
    }

    pub async fn is_available(&self) -> bool {
        self.client
            .get(format!("{}/api/tags", self.base_url))
            .timeout(std::time::Duration::from_secs(3))
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false)
    }

    fn build_messages(&self, opts: &CompletionOptions) -> Vec<Value> {
        let mut messages = Vec::new();
        if let Some(sys) = &opts.system {
            if !sys.is_empty() {
                messages.push(json!({"role": "system", "content": sys}));
            }
        }
        for m in &opts.messages {
            messages.push(json!({"role": m.role, "content": m.content}));
        }
        messages
    }

    async fn check_status(resp: reqwest::Response) -> Result<reqwest::Response, BackendError> {
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            if status.as_u16() == 404 {
                return Err(BackendError::Other(anyhow::anyhow!(
                    "Ollama model not found — run `ollama pull <model>` to download it"
                )));
            }
            return Err(BackendError::Other(anyhow::anyhow!(
                "Ollama HTTP {status}: {body}"
            )));
        }
        Ok(resp)
    }
}

#[async_trait]
impl Backend for OllamaBackend {
    fn name(&self) -> &str {
        "ollama"
    }

    async fn complete(&self, opts: CompletionOptions) -> Result<CompletionResult, BackendError> {
        let messages = self.build_messages(&opts);
        let body = json!({
            "model": opts.model_id,
            "messages": messages,
            "stream": false,
        });

        let resp = Self::check_status(
            self.client
                .post(self.chat_url())
                .json(&body)
                .send()
                .await
                .map_err(map_reqwest_err)?,
        )
        .await?;

        let data: Value = resp
            .json()
            .await
            .map_err(|e| BackendError::Other(e.into()))?;

        let content = data["message"]["content"]
            .as_str()
            .unwrap_or("")
            .to_string();
        let input_tokens = data["prompt_eval_count"].as_u64().unwrap_or(0) as u32;
        let output_tokens = data["eval_count"].as_u64().unwrap_or(0) as u32;

        Ok(CompletionResult {
            content,
            input_tokens,
            output_tokens,
            cache_hit: false,
        })
    }

    async fn agent_step(
        &self,
        system: Option<&str>,
        messages: &[AgentMessage],
        tools: &[ToolDef],
        model_id: &str,
        max_tokens: u32,
    ) -> Result<AgentTurn, BackendError> {
        crate::agent::react_fallback(self, system, messages, tools, model_id, max_tokens).await
    }

    async fn complete_streaming(
        &self,
        opts: CompletionOptions,
        on_token: Box<dyn Fn(String) + Send>,
    ) -> Result<CompletionResult, BackendError> {
        let messages = self.build_messages(&opts);
        let body = json!({
            "model": opts.model_id,
            "messages": messages,
            "stream": true,
        });

        let resp = Self::check_status(
            self.client
                .post(self.chat_url())
                .json(&body)
                .send()
                .await
                .map_err(map_reqwest_err)?,
        )
        .await?;

        let mut stream = resp.bytes_stream();
        let mut buf = String::new();
        let mut content = String::new();
        let mut input_tokens = 0u32;
        let mut output_tokens = 0u32;

        while let Some(chunk) = stream.next().await {
            let bytes = chunk.map_err(|e| BackendError::Other(e.into()))?;
            buf.push_str(&String::from_utf8_lossy(&bytes));

            // Ollama streams newline-delimited JSON (NDJSON), not SSE
            while let Some(pos) = buf.find('\n') {
                let line = buf[..pos].trim_end_matches('\r').to_string();
                buf.drain(..=pos);

                if line.is_empty() {
                    continue;
                }
                let event: Value = match serde_json::from_str(&line) {
                    Ok(v) => v,
                    Err(_) => continue,
                };

                if let Some(token) = event["message"]["content"].as_str() {
                    if !token.is_empty() {
                        let token = token.to_string();
                        content.push_str(&token);
                        on_token(token);
                    }
                }

                // Final chunk has done=true with token counts
                if event["done"].as_bool().unwrap_or(false) {
                    input_tokens = event["prompt_eval_count"].as_u64().unwrap_or(0) as u32;
                    output_tokens = event["eval_count"].as_u64().unwrap_or(0) as u32;
                }
            }
        }

        Ok(CompletionResult {
            content,
            input_tokens,
            output_tokens,
            cache_hit: false,
        })
    }
}
