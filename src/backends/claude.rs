use crate::backends::{
    map_reqwest_err, AgentMessage, AgentTurn, Backend, BackendError, CompletionOptions,
    CompletionResult, ToolCall, ToolDef,
};
use async_trait::async_trait;
use futures::StreamExt;
use reqwest::Client;
use serde_json::{json, Value};

const API_URL: &str = "https://api.anthropic.com/v1/messages";
const API_VERSION: &str = "2023-06-01";
const CACHE_BETA: &str = "prompt-caching-2024-07-31";

pub struct ClaudeBackend {
    client: Client,
    api_key: String,
}

impl ClaudeBackend {
    pub fn new(api_key: &str) -> Self {
        Self {
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(120))
                .build()
                .expect("reqwest client"),
            api_key: api_key.to_string(),
        }
    }

    fn build_body(&self, opts: &CompletionOptions, stream: bool) -> Value {
        let messages: Vec<Value> = opts
            .messages
            .iter()
            .map(|m| json!({"role": m.role, "content": m.content}))
            .collect();

        let mut body = json!({
            "model": opts.model_id,
            "max_tokens": opts.max_tokens,
            "stream": stream,
            "messages": messages,
        });

        if let Some(sys) = &opts.system {
            if !sys.is_empty() {
                // Wrap system in cache_control block when caching is enabled
                let sys_block = if opts.use_cache {
                    json!([{"type": "text", "text": sys, "cache_control": {"type": "ephemeral"}}])
                } else {
                    json!([{"type": "text", "text": sys}])
                };
                body["system"] = sys_block;
            }
        }

        body
    }

    fn request(&self, use_cache: bool) -> reqwest::RequestBuilder {
        let mut req = self
            .client
            .post(API_URL)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", API_VERSION)
            .header("content-type", "application/json");
        if use_cache {
            req = req.header("anthropic-beta", CACHE_BETA);
        }
        req
    }

    async fn check_status(resp: reqwest::Response) -> Result<reqwest::Response, BackendError> {
        let status = resp.status();
        if status.as_u16() == 401 || status.as_u16() == 403 {
            let body = resp.text().await.unwrap_or_default();
            return Err(BackendError::Auth(format!("Anthropic {status}: {body}")));
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
impl Backend for ClaudeBackend {
    fn name(&self) -> &str {
        "claude"
    }

    async fn complete(&self, opts: CompletionOptions) -> Result<CompletionResult, BackendError> {
        let use_cache = opts.use_cache;
        let body = self.build_body(&opts, false);

        let resp = Self::check_status(
            self.request(use_cache)
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

        let content = data["content"][0]["text"]
            .as_str()
            .unwrap_or("")
            .to_string();
        let input_tokens = data["usage"]["input_tokens"].as_u64().unwrap_or(0) as u32;
        let output_tokens = data["usage"]["output_tokens"].as_u64().unwrap_or(0) as u32;
        let cache_hit = data["usage"]["cache_read_input_tokens"]
            .as_u64()
            .unwrap_or(0)
            > 0;

        Ok(CompletionResult {
            content,
            input_tokens,
            output_tokens,
            cache_hit,
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
        // Build tools array
        let tools_json: Vec<Value> = tools
            .iter()
            .map(|t| {
                json!({
                    "name": t.name,
                    "description": t.description,
                    "input_schema": t.parameters,
                })
            })
            .collect();

        // Convert AgentMessages to Anthropic content format
        let api_messages: Vec<Value> = messages
            .iter()
            .map(|m| {
                let content: Vec<Value> = if !m.tool_results.is_empty() {
                    // User message returning tool results
                    m.tool_results
                        .iter()
                        .map(|(id, content, is_error)| {
                            json!({
                                "type": "tool_result",
                                "tool_use_id": id,
                                "content": content,
                                "is_error": is_error,
                            })
                        })
                        .collect()
                } else if m.role == "assistant" && !m.tool_calls.is_empty() {
                    // Assistant message with tool calls (possibly with text)
                    let mut blocks: Vec<Value> = Vec::new();
                    if let Some(text) = &m.text {
                        if !text.is_empty() {
                            blocks.push(json!({"type": "text", "text": text}));
                        }
                    }
                    for tc in &m.tool_calls {
                        blocks.push(json!({
                            "type": "tool_use",
                            "id": tc.id,
                            "name": tc.name,
                            "input": tc.input,
                        }));
                    }
                    blocks
                } else {
                    // Plain text message
                    let text = m.text.as_deref().unwrap_or("");
                    vec![json!({"type": "text", "text": text})]
                };

                json!({"role": m.role, "content": content})
            })
            .collect();

        let mut body = json!({
            "model": model_id,
            "max_tokens": max_tokens,
            "messages": api_messages,
            "tools": tools_json,
            "tool_choice": {"type": "auto"},
        });

        if let Some(sys) = system {
            if !sys.is_empty() {
                body["system"] = json!([{"type": "text", "text": sys}]);
            }
        }

        let resp = Self::check_status(
            self.request(false)
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

        let input_tokens = data["usage"]["input_tokens"].as_u64().unwrap_or(0) as u32;
        let output_tokens = data["usage"]["output_tokens"].as_u64().unwrap_or(0) as u32;

        let mut text: Option<String> = None;
        let mut tool_calls: Vec<ToolCall> = Vec::new();

        if let Some(content_blocks) = data["content"].as_array() {
            for block in content_blocks {
                match block["type"].as_str() {
                    Some("text") => {
                        let t = block["text"].as_str().unwrap_or("").to_string();
                        if !t.is_empty() {
                            text = Some(t);
                        }
                    }
                    Some("tool_use") => {
                        tool_calls.push(ToolCall {
                            id: block["id"].as_str().unwrap_or("").to_string(),
                            name: block["name"].as_str().unwrap_or("").to_string(),
                            input: block["input"].clone(),
                        });
                    }
                    _ => {}
                }
            }
        }

        Ok(AgentTurn {
            text,
            tool_calls,
            input_tokens,
            output_tokens,
        })
    }

    async fn complete_streaming(
        &self,
        opts: CompletionOptions,
        on_token: Box<dyn Fn(String) + Send>,
    ) -> Result<CompletionResult, BackendError> {
        let use_cache = opts.use_cache;
        let body = self.build_body(&opts, true);

        let resp = Self::check_status(
            self.request(use_cache)
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
        let mut cache_hit = false;

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
                match event["type"].as_str() {
                    Some("message_start") => {
                        let usage = &event["message"]["usage"];
                        input_tokens = usage["input_tokens"].as_u64().unwrap_or(0) as u32;
                        cache_hit =
                            usage["cache_read_input_tokens"].as_u64().unwrap_or(0) > 0;
                    }
                    Some("content_block_delta") => {
                        if let Some(text) = event["delta"]["text"].as_str() {
                            let text = text.to_string();
                            content.push_str(&text);
                            on_token(text);
                        }
                    }
                    Some("message_delta") => {
                        output_tokens =
                            event["usage"]["output_tokens"].as_u64().unwrap_or(0) as u32;
                    }
                    _ => {}
                }
            }
        }

        Ok(CompletionResult {
            content,
            input_tokens,
            output_tokens,
            cache_hit,
        })
    }
}
