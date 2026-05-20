use crate::backends::{
    map_reqwest_err, AgentMessage, AgentTurn, Backend, BackendError, CompletionOptions,
    CompletionResult, ToolCall, ToolDef,
};
use async_trait::async_trait;
use futures::StreamExt;
use reqwest::Client;
use serde_json::{json, Value};

const BASE_URL: &str =
    "https://generativelanguage.googleapis.com/v1beta/models";

pub struct GeminiBackend {
    client: Client,
    api_key: String,
}

impl GeminiBackend {
    pub fn new(api_key: &str) -> Self {
        Self {
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(120))
                .build()
                .expect("reqwest client"),
            api_key: api_key.to_string(),
        }
    }

    fn build_body(&self, opts: &CompletionOptions) -> Value {
        // Map messages: assistant → model role
        let contents: Vec<Value> = opts
            .messages
            .iter()
            .map(|m| {
                let role = if m.role == "assistant" { "model" } else { &m.role };
                json!({"role": role, "parts": [{"text": m.content}]})
            })
            .collect();

        let mut body = json!({
            "contents": contents,
            "generationConfig": {
                "maxOutputTokens": opts.max_tokens,
            },
        });

        if let Some(sys) = &opts.system {
            if !sys.is_empty() {
                body["systemInstruction"] = json!({"parts": [{"text": sys}]});
            }
        }

        if opts.use_search_grounding {
            body["tools"] = json!([{"google_search": {}}]);
        }

        body
    }

    fn stream_url(&self, model_id: &str) -> String {
        format!(
            "{BASE_URL}/{model_id}:streamGenerateContent?alt=sse&key={}",
            self.api_key
        )
    }

    fn generate_url(&self, model_id: &str) -> String {
        format!(
            "{BASE_URL}/{model_id}:generateContent?key={}",
            self.api_key
        )
    }

    async fn check_status(resp: reqwest::Response) -> Result<reqwest::Response, BackendError> {
        let status = resp.status();
        if status.as_u16() == 401 || status.as_u16() == 403 {
            let body = resp.text().await.unwrap_or_default();
            return Err(BackendError::Auth(format!("Gemini {status}: {body}")));
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

    fn extract_text_from_chunk(event: &Value) -> Vec<String> {
        let mut texts = Vec::new();
        if let Some(candidates) = event["candidates"].as_array() {
            for candidate in candidates {
                if let Some(parts) = candidate["content"]["parts"].as_array() {
                    for part in parts {
                        if let Some(text) = part["text"].as_str() {
                            texts.push(text.to_string());
                        }
                    }
                }
            }
        }
        texts
    }

    fn extract_usage(event: &Value) -> (u32, u32) {
        let meta = &event["usageMetadata"];
        let input = meta["promptTokenCount"].as_u64().unwrap_or(0) as u32;
        let output = meta["candidatesTokenCount"].as_u64().unwrap_or(0) as u32;
        (input, output)
    }
}

#[async_trait]
impl Backend for GeminiBackend {
    fn name(&self) -> &str {
        "gemini"
    }

    async fn agent_step(
        &self,
        system: Option<&str>,
        messages: &[AgentMessage],
        tools: &[ToolDef],
        model_id: &str,
        max_tokens: u32,
    ) -> Result<AgentTurn, BackendError> {
        // Build function declarations
        let function_declarations: Vec<Value> = tools
            .iter()
            .map(|t| {
                json!({
                    "name": t.name,
                    "description": t.description,
                    "parameters": t.parameters,
                })
            })
            .collect();

        // Convert AgentMessages to Gemini contents format
        let contents: Vec<Value> = messages
            .iter()
            .map(|m| {
                let role = if m.role == "assistant" { "model" } else { "user" };
                let parts: Vec<Value> = if !m.tool_results.is_empty() {
                    // User message returning function responses
                    m.tool_results
                        .iter()
                        .map(|(id, content, _is_error)| {
                            // Parse content as JSON if possible, else wrap as string
                            let response_val = serde_json::from_str::<Value>(content)
                                .unwrap_or_else(|_| json!({"output": content}));
                            json!({
                                "functionResponse": {
                                    "name": id,
                                    "response": response_val,
                                }
                            })
                        })
                        .collect()
                } else if m.role == "assistant" && !m.tool_calls.is_empty() {
                    // Model message with function calls (possibly with text)
                    let mut parts: Vec<Value> = Vec::new();
                    if let Some(text) = &m.text {
                        if !text.is_empty() {
                            parts.push(json!({"text": text}));
                        }
                    }
                    for tc in &m.tool_calls {
                        parts.push(json!({
                            "functionCall": {
                                "name": tc.name,
                                "args": tc.input,
                            }
                        }));
                    }
                    parts
                } else {
                    let text = m.text.as_deref().unwrap_or("");
                    vec![json!({"text": text})]
                };

                json!({"role": role, "parts": parts})
            })
            .collect();

        let allowed_names: Vec<&str> = tools.iter().map(|t| t.name).collect();

        let mut body = json!({
            "contents": contents,
            "tools": [{"functionDeclarations": function_declarations}],
            "toolConfig": {
                "functionCallingConfig": {
                    "mode": "ANY",
                    "allowedFunctionNames": allowed_names,
                }
            },
            "generationConfig": {
                "maxOutputTokens": max_tokens,
            },
        });

        if let Some(sys) = system {
            if !sys.is_empty() {
                body["systemInstruction"] = json!({"parts": [{"text": sys}]});
            }
        }

        let url = self.generate_url(model_id);
        let resp = Self::check_status(
            self.client
                .post(&url)
                .header("content-type", "application/json")
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

        let (input_tokens, output_tokens) = Self::extract_usage(&data);

        let mut text: Option<String> = None;
        let mut tool_calls: Vec<ToolCall> = Vec::new();

        if let Some(candidates) = data["candidates"].as_array() {
            for candidate in candidates {
                if let Some(parts) = candidate["content"]["parts"].as_array() {
                    for part in parts {
                        if let Some(t) = part["text"].as_str() {
                            if !t.is_empty() {
                                text = Some(t.to_string());
                            }
                        }
                        if let Some(fc) = part.get("functionCall") {
                            let name = fc["name"].as_str().unwrap_or("").to_string();
                            // Use function name as id so functionResponse.name matches
                            tool_calls.push(ToolCall {
                                id: name.clone(),
                                name,
                                input: fc["args"].clone(),
                            });
                        }
                    }
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

    async fn complete(&self, opts: CompletionOptions) -> Result<CompletionResult, BackendError> {
        let url = self.generate_url(&opts.model_id);
        let body = self.build_body(&opts);

        let resp = Self::check_status(
            self.client
                .post(&url)
                .header("content-type", "application/json")
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

        let mut content = String::new();
        for text in Self::extract_text_from_chunk(&data) {
            content.push_str(&text);
        }
        let (input_tokens, output_tokens) = Self::extract_usage(&data);

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
        let url = self.stream_url(&opts.model_id);
        let body = self.build_body(&opts);

        let resp = Self::check_status(
            self.client
                .post(&url)
                .header("content-type", "application/json")
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
                let event: Value = match serde_json::from_str(data_str) {
                    Ok(v) => v,
                    Err(_) => continue,
                };

                for text in Self::extract_text_from_chunk(&event) {
                    content.push_str(&text);
                    on_token(text);
                }

                // Update usage from every chunk — last one is the final count
                let (inp, out) = Self::extract_usage(&event);
                if inp > 0 {
                    input_tokens = inp;
                }
                if out > 0 {
                    output_tokens = out;
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
