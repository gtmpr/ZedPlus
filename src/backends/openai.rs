use crate::backends::{
    map_reqwest_err, AgentMessage, AgentTurn, Backend, BackendError, CompletionOptions,
    CompletionResult, ToolCall, ToolDef,
};
use async_trait::async_trait;
use futures::StreamExt;
use reqwest::Client;
use serde_json::{json, Value};

const API_URL: &str = "https://api.openai.com/v1/chat/completions";
const LMSTUDIO_URL: &str = "http://localhost:1234/v1/chat/completions";

pub struct OpenAiBackend {
    client: Client,
    api_key: String,
    base_url: String,
    /// Use ReAct text-based tool calling instead of native function calling.
    /// Needed for local models (LM Studio) that don't support the OpenAI tool spec.
    use_react: bool,
}

impl OpenAiBackend {
    pub fn new(api_key: &str) -> Self {
        Self::new_compat(API_URL, api_key, false)
    }

    pub fn new_lmstudio(base_url: &str) -> Self {
        let url = if base_url.is_empty() {
            LMSTUDIO_URL.to_string()
        } else {
            format!("{}/v1/chat/completions", base_url.trim_end_matches('/'))
        };
        Self::new_compat(&url, "", true)
    }

    pub fn new_compat(url: &str, api_key: &str, use_react: bool) -> Self {
        Self {
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(120))
                .build()
                .expect("reqwest client"),
            api_key: api_key.to_string(),
            base_url: url.to_string(),
            use_react,
        }
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
        let status = resp.status();
        if status.as_u16() == 401 {
            let body = resp.text().await.unwrap_or_default();
            return Err(BackendError::Auth(format!("OpenAI {status}: {body}")));
        }
        if status.as_u16() == 429 {
            return Err(BackendError::RateLimit);
        }
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(BackendError::Other(anyhow::anyhow!("HTTP {status}: {body}")));
        }
        Ok(resp)
    }
}

#[async_trait]
impl Backend for OpenAiBackend {
    fn name(&self) -> &str {
        "openai"
    }

    async fn agent_step(
        &self,
        system: Option<&str>,
        messages: &[AgentMessage],
        tools: &[ToolDef],
        model_id: &str,
        max_tokens: u32,
    ) -> Result<AgentTurn, BackendError> {
        if self.use_react {
            return crate::agent::react_fallback(self, system, messages, tools, model_id, max_tokens).await;
        }

        // Build tools array in OpenAI format
        let tools_json: Vec<Value> = tools
            .iter()
            .map(|t| {
                json!({
                    "type": "function",
                    "function": {
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.parameters,
                    }
                })
            })
            .collect();

        // Build messages array
        let mut api_messages: Vec<Value> = Vec::new();
        if let Some(sys) = system {
            if !sys.is_empty() {
                api_messages.push(json!({"role": "system", "content": sys}));
            }
        }

        for m in messages {
            if !m.tool_results.is_empty() {
                // Each tool result becomes a separate "tool" role message
                for (id, content, _is_error) in &m.tool_results {
                    api_messages.push(json!({
                        "role": "tool",
                        "tool_call_id": id,
                        "content": content,
                    }));
                }
            } else if m.role == "assistant" && !m.tool_calls.is_empty() {
                // Assistant message with tool_calls
                let tc_json: Vec<Value> = m
                    .tool_calls
                    .iter()
                    .map(|tc| {
                        json!({
                            "id": tc.id,
                            "type": "function",
                            "function": {
                                "name": tc.name,
                                "arguments": tc.input.to_string(),
                            }
                        })
                    })
                    .collect();

                let mut msg = json!({
                    "role": "assistant",
                    "tool_calls": tc_json,
                });
                if let Some(text) = &m.text {
                    if !text.is_empty() {
                        msg["content"] = json!(text);
                    }
                }
                api_messages.push(msg);
            } else {
                // Plain text message
                let content = m.text.as_deref().unwrap_or("");
                api_messages.push(json!({"role": m.role, "content": content}));
            }
        }

        let body = json!({
            "model": model_id,
            "messages": api_messages,
            "max_tokens": max_tokens,
            "tools": tools_json,
            "tool_choice": "auto",
        });

        let resp = Self::check_status(
            self.client
                .post(&self.base_url)
                .bearer_auth(&self.api_key)
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

        let input_tokens = data["usage"]["prompt_tokens"].as_u64().unwrap_or(0) as u32;
        let output_tokens = data["usage"]["completion_tokens"].as_u64().unwrap_or(0) as u32;

        let choice = &data["choices"][0];
        let message = &choice["message"];
        let finish_reason = choice["finish_reason"].as_str().unwrap_or("");

        let text = message["content"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());

        // Parse tool calls regardless of finish_reason — local LLMs (LM Studio, Ollama OpenAI compat)
        // often return finish_reason="stop" even when tool_calls are present.
        let mut tool_calls: Vec<ToolCall> = Vec::new();
        if let Some(tcs) = message["tool_calls"].as_array() {
            for tc in tcs {
                let arguments_str = tc["function"]["arguments"].as_str().unwrap_or("{}");
                let input: Value = serde_json::from_str(arguments_str)
                    .unwrap_or_else(|_| json!({}));
                let name = tc["function"]["name"].as_str().unwrap_or("").to_string();
                if !name.is_empty() {
                    tool_calls.push(ToolCall {
                        id: tc["id"].as_str().unwrap_or(&name).to_string(),
                        name,
                        input,
                    });
                }
            }
        }
        let _ = finish_reason; // consumed above; suppress unused warning

        Ok(AgentTurn {
            text,
            tool_calls,
            input_tokens,
            output_tokens,
        })
    }

    async fn complete(&self, opts: CompletionOptions) -> Result<CompletionResult, BackendError> {
        let messages = self.build_messages(&opts);
        let body = json!({
            "model": opts.model_id,
            "messages": messages,
            "max_tokens": opts.max_tokens,
            "stream": false,
        });

        let resp = Self::check_status(
            self.client
                .post(&self.base_url)
                .bearer_auth(&self.api_key)
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

        let content = data["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("")
            .to_string();
        let input_tokens = data["usage"]["prompt_tokens"].as_u64().unwrap_or(0) as u32;
        let output_tokens = data["usage"]["completion_tokens"].as_u64().unwrap_or(0) as u32;

        Ok(CompletionResult {
            content,
            input_tokens,
            output_tokens,
            cache_hit: false,
        })
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
            "max_tokens": opts.max_tokens,
            "stream": true,
            "stream_options": {"include_usage": true},
        });

        let resp = Self::check_status(
            self.client
                .post(&self.base_url)
                .bearer_auth(&self.api_key)
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

            while let Some(pos) = buf.find('\n') {
                let line = buf[..pos].trim_end_matches('\r').to_string();
                buf.drain(..=pos);

                let data_str = match line.strip_prefix("data: ") {
                    Some(d) => d,
                    None => continue,
                };
                if data_str == "[DONE]" {
                    break;
                }
                let event: Value = match serde_json::from_str(data_str) {
                    Ok(v) => v,
                    Err(_) => continue,
                };

                // Token content from streaming delta
                if let Some(token) = event["choices"][0]["delta"]["content"].as_str() {
                    let token = token.to_string();
                    content.push_str(&token);
                    on_token(token);
                }

                // Usage is in the final chunk (with stream_options.include_usage)
                if let Some(usage) = event.get("usage") {
                    if !usage.is_null() {
                        input_tokens = usage["prompt_tokens"].as_u64().unwrap_or(0) as u32;
                        output_tokens =
                            usage["completion_tokens"].as_u64().unwrap_or(0) as u32;
                    }
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
