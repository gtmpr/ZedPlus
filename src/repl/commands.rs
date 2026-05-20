/// Per-query flags that are set by slash commands and reset after each response.
#[derive(Debug, Default, Clone)]
pub struct QueryFlags {
    pub explain: bool,
    pub local: bool,
    pub cheap: bool,
    pub model: Option<String>,
    pub scope: Option<String>,
    /// @-mention override: "claude-cli", "gemini-cli", etc. Bypasses normal routing cascade.
    pub force_provider: Option<String>,
}

/// Parsed input: either a user query (with optional per-query flags) or a session action.
#[derive(Debug)]
pub enum ReplInput {
    Query { text: String, flags: QueryFlags },
    Apply,
    Agent,
    Accept,
    Clear,
    Usage,
    History,
    Index,
    Help,
    Models,
    Build { query: String },
    /// `/persona [name|off]` — list personas, activate one, or clear.
    Persona { name: Option<String> },
    /// `/debate [strategy] <query>` — multi-agent brainstorm.
    Debate { strategy: String, query: String },
    Exit,
}

/// Parse a raw input line into a REPL action. Returns None for empty/whitespace-only input.
pub fn parse(line: &str) -> Option<ReplInput> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }

    if !trimmed.starts_with('/') {
        return Some(ReplInput::Query {
            text: trimmed.to_string(),
            flags: QueryFlags::default(),
        });
    }

    let mut parts = trimmed.splitn(3, ' ');
    let cmd = parts.next().unwrap_or("");
    let arg1 = parts.next().unwrap_or("").trim();
    let rest = parts.next().unwrap_or("").trim();

    match cmd {
        "/apply" => Some(ReplInput::Apply),
        "/agent" => Some(ReplInput::Agent),
        "/accept" | "/yes" => Some(ReplInput::Accept),
        "/clear" => Some(ReplInput::Clear),
        "/usage" => Some(ReplInput::Usage),
        "/history" | "/log" => Some(ReplInput::History),
        "/index" => Some(ReplInput::Index),
        "/help" => Some(ReplInput::Help),
        "/exit" | "/quit" | "/q" => Some(ReplInput::Exit),

        "/explain" => {
            let text = format!("{} {}", arg1, rest).trim().to_string();
            if text.is_empty() {
                eprintln!("Usage: /explain <query>");
                None
            } else {
                Some(ReplInput::Query {
                    text,
                    flags: QueryFlags { explain: true, ..Default::default() },
                })
            }
        }

        "/local" => {
            let text = format!("{} {}", arg1, rest).trim().to_string();
            if text.is_empty() {
                eprintln!("Usage: /local <query>");
                None
            } else {
                Some(ReplInput::Query {
                    text,
                    flags: QueryFlags { local: true, ..Default::default() },
                })
            }
        }

        "/cheap" => {
            let text = format!("{} {}", arg1, rest).trim().to_string();
            if text.is_empty() {
                eprintln!("Usage: /cheap <query>");
                None
            } else {
                Some(ReplInput::Query {
                    text,
                    flags: QueryFlags { cheap: true, ..Default::default() },
                })
            }
        }

        "/model" => {
            if arg1.is_empty() {
                Some(ReplInput::Models)
            } else {
                let text = rest.to_string();
                if text.is_empty() {
                    eprintln!("Usage: /model <alias> <query>  (run /model alone to list aliases)");
                    None
                } else {
                    Some(ReplInput::Query {
                        text,
                        flags: QueryFlags {
                            model: Some(arg1.to_string()),
                            ..Default::default()
                        },
                    })
                }
            }
        }

        "/build" => {
            let text = format!("{} {}", arg1, rest).trim().to_string();
            if text.is_empty() {
                eprintln!("Usage: /build <description of what to build>");
                None
            } else {
                Some(ReplInput::Build { query: text })
            }
        }

        "/persona" => {
            if arg1.is_empty() {
                Some(ReplInput::Persona { name: None })
            } else {
                Some(ReplInput::Persona { name: Some(arg1.to_string()) })
            }
        }

        "/debate" => {
            // /debate [strategy] <query>
            // strategy is optional; recognised strategies: debate, red-team, perspectives, delphi
            let known = ["debate", "red-team", "redteam", "perspectives", "perspective", "delphi"];
            let (strategy, query) = if known.contains(&arg1.to_ascii_lowercase().as_str()) {
                (arg1.to_string(), rest.to_string())
            } else {
                // arg1 is the start of the query, not a strategy name
                let full = format!("{} {}", arg1, rest).trim().to_string();
                ("debate".to_string(), full)
            };
            if query.is_empty() {
                eprintln!("Usage: /debate [strategy] <query>  (strategies: debate, red-team, perspectives, delphi)");
                None
            } else {
                Some(ReplInput::Debate { strategy, query })
            }
        }

        "/scope" => {
            if arg1.is_empty() {
                eprintln!("Usage: /scope narrow|broad");
                None
            } else {
                eprintln!("Scope set to '{}' for next query.", arg1);
                Some(ReplInput::Query {
                    text: rest.to_string(),
                    flags: QueryFlags {
                        scope: Some(arg1.to_string()),
                        ..Default::default()
                    },
                })
            }
        }

        _ => {
            eprintln!("Unknown command '{cmd}'. Type /help for available commands.");
            None
        }
    }
}

pub fn print_help() {
    println!("Available commands:");
    println!("  /agent              Toggle agentic mode (file tools, run commands)");
    println!("  /accept             Toggle auto-accept (skip write/run confirmations)");
    println!("  /apply              Apply code blocks from the last response to files");
    println!("  /clear              Clear session context (history reset)");
    println!("  /usage              Show token/cost usage for this session");
    println!("  /history            Show last 20 turns — question, provider, answer summary");
    println!("  /index              Trigger a re-index of the current directory");
    println!("  /explain <query>    Send query and show routing decision");
    println!("  /local <query>      Force local model for one query");
    println!("  /cheap <query>      Force cheapest model for one query");
    println!("  /model              List available model aliases");
    println!("  /model <alias> <q>  Override model for one query (use alias from /model list)");
    println!("  /build <desc>       Run multi-phase build pipeline (clarify→arch→plan→build→QC→test→devlog)");
    println!("  /scope narrow|broad Set scope for next query");
    println!("  /persona            List developer personas");
    println!("  /persona <name>     Activate a persona (architect/debugger/security/performance/teacher/reviewer/tester/devops)");
    println!("  /persona off        Clear active persona");
    println!("  /debate <query>     Multi-agent brainstorm using two models (default: debate strategy)");
    println!("  /debate <strategy> <query>  Brainstorm with a specific strategy (debate/red-team/perspectives/delphi)");
    println!("  @claude/@gemini/@local/@cheap  Prefix query to route to a specific backend");
    println!("  /exit               End session and show summary");
}
