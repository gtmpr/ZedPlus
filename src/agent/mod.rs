pub mod tools;

use crate::backends::{
    AgentMessage, AgentTurn, Backend, BackendError, CompletionOptions, Message, ToolCall, ToolDef,
};
use std::io::Write as IoWrite;
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};

// ── ReAct text-based fallback ─────────────────────────────────────────────────

/// Text-based ReAct fallback for backends without native tool calling.
/// Appends tool descriptions to the system prompt and parses `<tool_call>` XML blocks.
pub async fn react_fallback(
    backend: &dyn Backend,
    system: Option<&str>,
    messages: &[AgentMessage],
    tool_defs: &[ToolDef],
    model_id: &str,
    max_tokens: u32,
) -> Result<AgentTurn, BackendError> {
    // Build tool description for system prompt injection
    let tool_desc = build_react_tool_desc(tool_defs);
    let combined_system = match system {
        Some(s) if !s.is_empty() => format!("{s}\n\n{tool_desc}"),
        _ => tool_desc,
    };

    // Convert AgentMessages back to plain Messages
    let plain_messages: Vec<Message> = agent_messages_to_plain(messages);

    let opts = CompletionOptions {
        model_id: model_id.to_string(),
        system: Some(combined_system),
        messages: plain_messages,
        max_tokens,
        use_search_grounding: false,
        use_cache: false,
        auto_accept: false,
    };

    let result = backend.complete(opts).await?;
    let text = result.content.clone();

    if text.trim().is_empty() {
        return Err(BackendError::Other(anyhow::anyhow!(
            "empty response from local model — context may be too large or model does not support this format"
        )));
    }

    // Parse <tool_call>...</tool_call> blocks
    let tool_calls = parse_tool_calls(&text);

    if tool_calls.is_empty() {
        Ok(AgentTurn {
            text: Some(text),
            tool_calls: vec![],
            input_tokens: result.input_tokens,
            output_tokens: result.output_tokens,
        })
    } else {
        // Return tool calls; strip the tool_call XML from text
        let clean_text = strip_tool_call_tags(&text);
        Ok(AgentTurn {
            text: if clean_text.trim().is_empty() {
                None
            } else {
                Some(clean_text)
            },
            tool_calls,
            input_tokens: result.input_tokens,
            output_tokens: result.output_tokens,
        })
    }
}

fn build_react_tool_desc(tools: &[ToolDef]) -> String {
    let mut desc = String::from(
        "You have access to the following tools. When you need to use one, output a tool call block \
         on its own line — you MUST include the closing tag:\n\n\
         <tool_call>{\"name\": \"tool_name\", \"arguments\": {\"param\": \"value\"}}</tool_call>\n\n\
         Example — list the current directory:\n\
         <tool_call>{\"name\": \"list_dir\", \"arguments\": {\"path\": \".\", \"depth\": 1}}</tool_call>\n\n\
         Rules:\n\
         - IMPORTANT: For understanding a codebase, ALWAYS start with search_semantic. Do NOT try to read every file or list every directory.\n\
         - SPECIALIZED KNOWLEDGE: If you need expert knowledge in a specific language (e.g. Rust, React) or task (e.g. Performance, Security), use search_skills to find a matching skill pack and activate_skill to enable its expert instructions.\n\
         - Always close the tag with </tool_call>\n\
         - Wait for the [tool_result] before writing your next thought\n\
         - Do NOT describe tool calls in prose; output the block directly\n\
         - Use \"arguments\" (not \"parameters\") as the key\n\n\
         Available tools:\n",
    );
    for t in tools {
        desc.push_str(&format!("- {}(", t.name));
        if let Some(props) = t.parameters["properties"].as_object() {
            let required: Vec<&str> = t.parameters["required"]
                .as_array()
                .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
                .unwrap_or_default();
            let params: Vec<String> = props
                .keys()
                .map(|k| {
                    if required.contains(&k.as_str()) {
                        k.clone()
                    } else {
                        format!("{k}?")
                    }
                })
                .collect();
            desc.push_str(&params.join(", "));
        }
        desc.push_str(&format!("): {}\n", t.description));
    }
    desc
}

fn agent_messages_to_plain(messages: &[AgentMessage]) -> Vec<Message> {
    let mut plain: Vec<Message> = Vec::new();

    for m in messages {
        if !m.tool_results.is_empty() {
            // Render tool results as user text
            let mut content = String::new();
            for (id, result_text, is_error) in &m.tool_results {
                if *is_error {
                    content.push_str(&format!("[tool_error for {id}]: {result_text}\n"));
                } else {
                    content.push_str(&format!("[tool_result for {id}]: {result_text}\n"));
                }
            }
            plain.push(Message {
                role: "user".to_string(),
                content: content.trim_end().to_string(),
            });
        } else if m.role == "assistant" && !m.tool_calls.is_empty() {
            // Render tool calls as assistant text so the model sees them in context
            let mut content = String::new();
            if let Some(t) = &m.text {
                if !t.is_empty() {
                    content.push_str(t);
                    content.push('\n');
                }
            }
            for tc in &m.tool_calls {
                let args_str = tc.input.to_string();
                content.push_str(&format!(
                    "<tool_call>{{\"name\": \"{}\", \"arguments\": {}}}</tool_call>\n",
                    tc.name, args_str
                ));
            }
            plain.push(Message {
                role: "assistant".to_string(),
                content: content.trim_end().to_string(),
            });
        } else {
            let content = m.text.clone().unwrap_or_default();
            plain.push(Message {
                role: m.role.clone(),
                content,
            });
        }
    }

    plain
}

fn parse_tool_calls(text: &str) -> Vec<ToolCall> {
    let mut calls = Vec::new();
    let mut remaining = text;

    while let Some(start) = remaining.find("<tool_call>") {
        let after = &remaining[start + "<tool_call>".len()..];

        // Accept with or without closing tag — a missing closing tag means the model
        // truncated its output; we still try to parse what's there.
        let (raw, consumed) = if let Some(end) = after.find("</tool_call>") {
            (after[..end].trim(), end + "</tool_call>".len())
        } else {
            (after.trim(), after.len())
        };

        if let Some(call) = try_parse_tool_call(raw, calls.len()) {
            calls.push(call);
        }

        remaining = &after[consumed..];
    }

    calls
}

/// Try to parse a tool call from the raw content between `<tool_call>` tags.
/// Handles several formats local models commonly produce:
///   1. Standard:   {"name": "list_dir", "arguments": {"path": "."}}
///   2. Parameters: {"name": "list_dir", "parameters": {"path": "."}}
///   3. Shorthand:  list_dir{"path": "."}   (name prefix before JSON object)
///   4. Name only:  list_dir               (no arguments)
fn try_parse_tool_call(raw: &str, idx: usize) -> Option<ToolCall> {
    let raw = raw.trim();

    // Format 1 & 2: starts with '{' — full JSON wrapper
    if raw.starts_with('{') {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(raw) {
            let name = val["name"].as_str().unwrap_or("").to_string();
            if !name.is_empty() {
                // Accept "arguments" or "parameters" as the args key
                let args = if val["arguments"].is_object() || val["arguments"].is_array() {
                    val["arguments"].clone()
                } else if val["parameters"].is_object() || val["parameters"].is_array() {
                    val["parameters"].clone()
                } else {
                    serde_json::Value::Object(Default::default())
                };
                return Some(ToolCall { id: format!("react_{idx}"), name, input: args });
            }
        }
    }

    // Format 3 & 4: shorthand  tool_name{...}  or just  tool_name
    let brace_pos = raw.find(|c: char| c == '{' || c == '(' || c.is_whitespace())
        .unwrap_or(raw.len());
    let name = raw[..brace_pos].trim();

    // Name must be a valid identifier
    if name.is_empty()
        || !name.chars().all(|c| c.is_alphanumeric() || c == '_')
    {
        return None;
    }

    let args_str = raw[brace_pos..].trim();
    let args = if args_str.is_empty() {
        serde_json::Value::Object(Default::default())
    } else if args_str.starts_with('{') {
        serde_json::from_str(args_str)
            .unwrap_or_else(|_| serde_json::Value::Object(Default::default()))
    } else {
        serde_json::Value::Object(Default::default())
    };

    Some(ToolCall { id: format!("react_{idx}"), name: name.to_string(), input: args })
}

fn strip_tool_call_tags(text: &str) -> String {
    let mut result = String::new();
    let mut pos = 0;

    while pos < text.len() {
        match text[pos..].find("<tool_call>") {
            None => {
                result.push_str(&text[pos..]);
                break;
            }
            Some(rel) => {
                // Keep text before the tag
                result.push_str(&text[pos..pos + rel]);
                let after = pos + rel + "<tool_call>".len();
                if let Some(end_rel) = text[after..].find("</tool_call>") {
                    // Skip tag and its content
                    pos = after + end_rel + "</tool_call>".len();
                } else {
                    // No closing tag — drop everything from <tool_call> to end of string
                    break;
                }
            }
        }
    }

    result
}

// ── Main agentic loop ─────────────────────────────────────────────────────────

/// Run the agentic loop until the model stops calling tools or `max_steps` is reached.
///
/// Returns `(final_text, total_input_tokens, total_output_tokens)`.
pub async fn run(
    query: &str,
    system: Option<&str>,
    history: &[Message],
    backend: &dyn Backend,
    model_id: &str,
    max_tokens: u32,
    max_steps: u32,
    cwd: &std::path::Path,
    auto_accept: bool,
    ollama_url: &str,
    cancel: Arc<AtomicBool>,
) -> anyhow::Result<(String, u32, u32)> {
    let mut agent_messages: Vec<AgentMessage> = Vec::new();
    let mut active_system = system.map(|s| s.to_string()).unwrap_or_default();

    // Convert history into AgentMessages
    for m in history {
        let am = if m.role == "user" {
            AgentMessage::user_text(m.content.clone())
        } else {
            AgentMessage::assistant(Some(m.content.clone()), vec![])
        };
        agent_messages.push(am);
    }

    // Add the user's query
    agent_messages.push(AgentMessage::user_text(query));

    let all_tools = tools::all_tools();
    let mut accumulated_text = String::new();
    let mut total_input = 0u32;
    let mut total_output = 0u32;
    // Load Skill Registry for discovery tools
    let skill_dir = crate::platform::dirs::config_dir()?.join("skills");
    let mut skill_registry = crate::skills::SkillRegistry::new(skill_dir);
    let _ = skill_registry.load_all().await;
let mut tools_executed = 0u32;
let mut retry_count = 0u32;
let mut last_model_id = model_id.to_string();

for step in 0..max_steps {
    if cancel.load(Ordering::Relaxed) {
        eprintln!("\n\x1b[33m[cancelled]\x1b[0m");
        break;
    }

    // Anti-Loop: If we've retried the same step twice, swap models
    let current_model = if retry_count >= 2 {
        let swap = if last_model_id.contains("claude") { "gemini-2.5-pro" } else { "claude-sonnet-4-6" };
        println!("\x1b[33m[agent] Detected possible loop. Swapping to {} for a fresh perspective...\x1b[0m", swap);
        swap.to_string()
    } else {
        last_model_id.clone()
    };

    // Try native tool calling...
    let sys_ref = if active_system.is_empty() { None } else { Some(active_system.as_str()) };
    let step_result = backend
        .agent_step(sys_ref, &agent_messages, &all_tools, &current_model, max_tokens)
        .await;

        let turn: AgentTurn = match step_result {
            Ok(t) => t,
            Err(BackendError::Other(ref e))
                if e.to_string().contains("does not support tool use") =>
            {
                react_fallback(backend, sys_ref, &agent_messages, &all_tools, model_id, max_tokens)
                    .await
                    .map_err(|e| anyhow::anyhow!("{e}"))?
            }
            Err(e) => return Err(anyhow::anyhow!("{e}")),
        };

        total_input = total_input.saturating_add(turn.input_tokens);
        total_output = total_output.saturating_add(turn.output_tokens);

        // Print text if present
        if let Some(ref text) = turn.text {
            print!("{text}");
            let _ = std::io::stdout().flush();
            if !accumulated_text.is_empty() {
                accumulated_text.push('\n');
            }
            accumulated_text.push_str(text);
        }

        // If no tool calls — we're done
        if turn.tool_calls.is_empty() {
            break;
        }

        tools_executed += turn.tool_calls.len() as u32;

        // Execute each tool call
        let mut results: Vec<(String, String, bool)> = Vec::new();
        for tc in &turn.tool_calls {
            let input_display = truncate_json_display(&tc.input, 120);
            println!("\n\x1b[36m[tool: {}({})]\x1b[0m", tc.name, input_display);
            let _ = std::io::stdout().flush();

            let (output, is_error) = match tc.name.as_str() {
                "search_semantic" => execute_semantic_search(&tc.input, ollama_url).await,
                "search_skills"   => execute_skill_search(&tc.input, &skill_registry),
                "activate_skill"  => {
                    let (res, err, injection) = execute_skill_activation(&tc.input, &skill_registry);
                    if !err {
                        if let Some(inj) = injection {
                            if !active_system.is_empty() {
                                active_system.push_str("\n\n");
                            }
                            active_system.push_str(&format!("### Skill Activated: {}\n{}", tc.input["name"], inj));
                            println!("\x1b[32m[agent] Skill activated! Injected domain knowledge into system prompt.\x1b[0m");
                        }
                    }
                    (res, err)
                }
                _ => {
                    let res = tools::execute(&tc.name, &tc.input, cwd, auto_accept);
                    // Trigger tests if file was written
                    if tc.name == "write_file" && !res.1 {
                        let test_res = crate::tester::runner::run_background_tests(cwd.to_path_buf(), "agent".to_string(), Some(model_id.to_string())).await;
                        if let Ok(tr) = test_res {
                            if !tr.passed {
                                retry_count += 1;
                                (format!("FILE WRITTEN SUCCESSFULLY, BUT TESTS FAILED:\n\n{}", tr.stderr), true)
                            } else {
                                retry_count = 0; // Reset on success
                                res
                            }
                        } else { res }
                    } else { res }
                }
            };

            // Print a brief result summary (first 200 chars)
            let summary = if output.len() > 200 {
                format!("{}...", &output[..200])
            } else {
                output.clone()
            };
            if is_error {
                println!("\x1b[31m[error]\x1b[0m {summary}");
            } else {
                println!("\x1b[90m[result]\x1b[0m {summary}");
            }
            let _ = std::io::stdout().flush();

            results.push((tc.id.clone(), output, is_error));
        }

        // Add assistant turn to history
        agent_messages.push(AgentMessage::assistant(
            turn.text.clone(),
            turn.tool_calls
                .into_iter()
                .collect(),
        ));

        // Add tool results as a user message
        agent_messages.push(AgentMessage::tool_results(results));

        // Safety: print newline after tool cycle
        println!();

        // Check if we hit the step limit on the next iteration
        if step + 1 >= max_steps {
            println!("\x1b[33m[agent: reached max_steps={max_steps}, stopping]\x1b[0m");
        }
    }

    // Only error if the model produced nothing at all — a text-only response with
    // no tool calls is a valid direct answer and should not be treated as a failure.
    if tools_executed == 0 && accumulated_text.trim().is_empty() {
        return Err(anyhow::anyhow!(
            "model returned no text and no tool calls — does not support ReAct format, trying next backend"
        ));
    }

    Ok((accumulated_text, total_input, total_output))
}

fn execute_skill_search(input: &serde_json::Value, registry: &crate::skills::SkillRegistry) -> (String, bool) {
    let query = match input["query"].as_str() {
        Some(q) => q,
        None => return ("Missing 'query' argument".to_string(), true),
    };
    
    let mut matches = registry.find_by_query(query);
    
    // Also include built-ins in the search
    let builtins = crate::skills::builtin_packs();
    for (name, pack) in &builtins {
        if name.contains(query) || pack.description.to_lowercase().contains(&query.to_lowercase()) {
            if !matches.iter().any(|p| p.name == *name) {
                matches.push(pack);
            }
        }
    }

    if matches.is_empty() {
        return (format!("No skills found matching '{query}'. Try a broader keyword like 'rust', 'python', or 'react'."), false);
    }

    let mut out = format!("Found {} skills matching '{query}':\n\n", matches.len());
    for pack in matches {
        out.push_str(&format!("- {}: {}\n", pack.name, pack.description));
    }
    out.push_str("\nUse activate_skill(name) to enable one.");
    (out, false)
}

fn execute_skill_activation(input: &serde_json::Value, registry: &crate::skills::SkillRegistry) -> (String, bool, Option<String>) {
    let name = match input["name"].as_str() {
        Some(n) => n,
        None => return ("Missing 'name' argument".to_string(), true, None),
    };

    // Check installed skills first
    let pack = if let Some(p) = registry.get(name) {
        Some(p.clone())
    } else {
        // Check built-ins
        crate::skills::builtin_packs().get(name).cloned()
    };

    match pack {
        Some(p) => {
            let injection = p.system_prompt_injection.clone();
            (format!("Skill '{}' activated successfully.", p.name), false, injection)
        }
        None => (format!("Skill pack '{}' not found. Use search_skills to find valid names.", name), true, None),
    }
}

async fn execute_semantic_search(input: &serde_json::Value, ollama_url: &str) -> (String, bool) {
    let query = match input["query"].as_str() {
        Some(q) => q,
        None => return ("Missing 'query' argument".to_string(), true),
    };
    let top_k = input["top_k"].as_u64().map(|n| (n as usize).min(10)).unwrap_or(5);

    match crate::indexer::similarity_search(query, top_k, ollama_url).await {
        Ok(chunks) if chunks.is_empty() => {
            (
                "No results — the index may be empty or Ollama is not running.\n\
                 Run `zedplus index` to build the semantic index."
                    .to_string(),
                false,
            )
        }
        Ok(chunks) => {
            let mut out = format!("Top {} semantic matches for: {query}\n\n", chunks.len());
            for (i, chunk) in chunks.iter().enumerate() {
                let symbol = chunk.symbol.as_deref().unwrap_or("(file)");
                out.push_str(&format!(
                    "── [{i}] {} :: {} (score {:.2}) ──\n{}\n\n",
                    chunk.file_path, symbol, chunk.score, chunk.content
                ));
            }
            (out, false)
        }
        Err(e) => (format!("Semantic search error: {e}"), true),
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn truncate_json_display(val: &serde_json::Value, max_len: usize) -> String {
    let s = val.to_string();
    if s.len() <= max_len {
        s
    } else {
        format!("{}...", &s[..max_len])
    }
}
