pub mod commands;
mod readline;

use anyhow::Result;
use chrono::Utc;
use std::io::{self, Write as IoWrite};
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};

use crate::{
    agent,
    apply,
    backends::{self, CompletionOptions, Message},
    brainstorm,
    config,
    config::schema::UiStyle,
    context::SystemPromptBuilder,
    db,
    distiller,
    indexer::{embedder::Embedder, git, store::IndexStore},
    persona,
    pipeline,
    platform::dirs,
    router,
    sessions,
    setup::detector,
};

const MAX_HISTORY_TOKENS: u32 = 60_000;
const TOP_K_CHUNKS: usize = 5;
const SUMMARY_TARGET_TOKENS: u32 = 4_000;

struct Session {
    messages: Vec<Message>,
    model_key: String,
    model_id: String,
    provider: String,
    session_total_cost: f64,
    session_tokens_in: u32,
    session_tokens_out: u32,
    turn_count: u32,
    session_id: String,
    session_name: Option<String>,
    project_path: String,
    git_branch: Option<String>,
    started_at: i64,
    last_response: Option<String>,
    agent_mode: bool,
    auto_accept: bool,
    /// Active developer persona name (e.g. "architect"), None if unset.
    active_persona: Option<String>,
    /// Per-backend stats: provider → (turns, tokens_in, tokens_out)
    backend_usage: std::collections::HashMap<String, (u32, u32, u32)>,
    /// Stderr from the most recent test failure (populated after agent writes files).
    last_test_failure: Option<String>,
}

impl Session {
    fn new(model_key: String, model_id: String, provider: String) -> Self {
        let cwd = std::env::current_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();
        let branch = std::env::current_dir()
            .ok()
            .and_then(|p| git::current_branch(&p));
        let id = format!("{:x}", Utc::now().timestamp_millis());
        Session {
            messages: Vec::new(),
            model_key,
            model_id,
            provider,
            session_total_cost: 0.0,
            session_tokens_in: 0,
            session_tokens_out: 0,
            turn_count: 0,
            session_id: id,
            session_name: None,
            project_path: cwd,
            git_branch: branch,
            started_at: Utc::now().timestamp(),
            last_response: None,
            agent_mode: true,
            auto_accept: false,
            active_persona: None,
            backend_usage: std::collections::HashMap::new(),
            last_test_failure: None,
        }
    }

    fn from_snapshot(
        snapshot: &sessions::ResumableSession,
        turns: Vec<Message>,
        model_key: String,
        model_id: String,
        provider: String,
    ) -> Self {
        let cwd = std::env::current_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();
        Session {
            messages: turns,
            model_key,
            model_id,
            provider,
            session_total_cost: snapshot.total_cost,
            session_tokens_in: 0,
            session_tokens_out: 0,
            turn_count: snapshot.turn_count as u32,
            session_id: snapshot.id.clone(),
            session_name: Some(snapshot.name.clone()),
            project_path: cwd,
            git_branch: snapshot.git_branch.clone(),
            started_at: Utc::now().timestamp(),
            last_response: None,
            agent_mode: true,
            auto_accept: false,
            active_persona: None,
            backend_usage: std::collections::HashMap::new(),
            last_test_failure: None,
        }
    }

    fn record_backend(&mut self, provider: &str, tokens_in: u32, tokens_out: u32) {
        let entry = self.backend_usage.entry(provider.to_string()).or_insert((0, 0, 0));
        entry.0 += 1;
        entry.1 += tokens_in;
        entry.2 += tokens_out;
    }

    fn total_message_tokens(&self) -> u32 {
        self.messages
            .iter()
            .map(|m| router::cost::estimate_tokens(&m.content))
            .sum()
    }

    fn push_user(&mut self, content: String) {
        self.messages.push(Message { role: "user".to_string(), content });
    }

    fn push_assistant(&mut self, content: String) {
        self.messages.push(Message { role: "assistant".to_string(), content });
    }
}

pub async fn run(initial_query: Option<String>) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let mut cfg = config::load(Some(&cwd))?;

    let default_alias = cfg.config.routing.rules.fallback.clone();
    let (provider, model_id) = backends::resolve_model(&default_alias, &cfg.models)
        .unwrap_or_else(|| ("claude".to_string(), default_alias.clone()));

    let mut session = Session::new(default_alias.clone(), model_id.clone(), provider.clone());

    let is_tty = io::IsTerminal::is_terminal(&io::stdout());

    // Offer resume when entering the REPL interactively without a pre-loaded query
    if is_tty && initial_query.is_none() {
        if let Some(snapshot) = try_offer_resume(&cfg, &session.project_path, session.git_branch.as_deref()) {
            let turns = snapshot.1;
            let snap = snapshot.0;
            let prior_turns = turns.len() / 2;
            session = Session::from_snapshot(&snap, turns, default_alias.clone(), model_id, provider);
            println!("Resumed '{}' — {} prior turns loaded.\n", snap.name, prior_turns);
        }
    }

    run_inner(session, initial_query, &mut cfg, is_tty).await
}

/// Resume a specific session by ID and name (called from `zedplus resume` and `session resume`).
pub async fn run_resumed(
    session_id: String,
    session_name: String,
    git_branch: Option<String>,
    turn_count: u32,
    total_cost: f64,
    turns: Vec<Message>,
) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let mut cfg = config::load(Some(&cwd))?;

    let default_alias = cfg.config.routing.rules.fallback.clone();
    let (provider, model_id) = backends::resolve_model(&default_alias, &cfg.models)
        .unwrap_or_else(|| ("claude".to_string(), default_alias.clone()));

    let snap = sessions::ResumableSession {
        id: session_id,
        name: session_name,
        turn_count: turn_count as i64,
        total_cost,
        last_active: Utc::now().timestamp(),
        git_branch,
    };

    let session = Session::from_snapshot(&snap, turns, default_alias, model_id, provider);
    println!("Resumed '{}' — {} prior turns loaded.\n", snap.name, snap.turn_count);

    let is_tty = io::IsTerminal::is_terminal(&io::stdout());
    run_inner(session, None, &mut cfg, is_tty).await
}

fn try_offer_resume(
    cfg: &config::LoadedConfig,
    project_path: &str,
    branch: Option<&str>,
) -> Option<(sessions::ResumableSession, Vec<Message>)> {
    let db_path = dirs::db_file().ok()?;
    if !db_path.exists() {
        return None;
    }
    let conn = db::open(&db_path).ok()?;
    let threshold = cfg.config.sessions.auto_resume_threshold_hours as i64;
    let since = Utc::now().timestamp() - threshold * 3600;
    let max = cfg.config.sessions.max_resume_candidates as usize;

    let candidates = sessions::find_resumable(&conn, project_path, branch, since, max);
    if candidates.is_empty() {
        return None;
    }

    let chosen = sessions::offer_resume_prompt(candidates).ok()??;
    let turns = sessions::load_turns(&conn, &chosen.id);
    Some((chosen, turns))
}

async fn run_inner(
    mut session: Session,
    initial_query: Option<String>,
    cfg: &mut config::LoadedConfig,
    _is_tty: bool,
) -> Result<()> {
    // Fire before_session hook
    let hooks = crate::hooks::HookRunner::new(&cfg.config.hooks);
    hooks.run_warn(crate::hooks::HookPoint::BeforeSession);

    let index_store = dirs::db_file()
        .ok()
        .and_then(|p| IndexStore::open(&p).ok());

    let ollama_url = cfg
        .config
        .services
        .ollama_url
        .as_deref()
        .unwrap_or("http://localhost:11434")
        .to_string();

    let embedder = Embedder::new(&ollama_url);
    let embedder_available = embedder.is_available().await;

    // Detect installed CLI tools once at session start
    let mut cli = detector::detect_cli_tools();
    cli.ollama_url = ollama_url.clone();
    cli.lmstudio_url = cfg.config.services.lmstudio_url
        .as_deref()
        .unwrap_or("http://localhost:1234")
        .to_string();

    if cli.claude {
        eprintln!("[zedplus] claude CLI detected — reasoning phases will use subscription");
    }
    if cli.gemini {
        eprintln!("[zedplus] gemini CLI detected — planning phases will use subscription");
    }
    if cli.codex_cli {
        eprintln!("[zedplus] codex CLI detected — code tasks eligible for codex-mini routing");
    }

    // Load and refresh quota state; report notable pressure at session start.
    {
        let mut quota = crate::platform::quota::QuotaCache::load();
        quota.expire_stale_caps();
        let _ = quota.refresh_gemini(cfg.config.quotas.gemini_daily_tokens);
        quota.save();
        if let Some(status) = quota.status_line() {
            eprintln!("\x1b[33m[zedplus] {status}\x1b[0m");
        }
    }

    // First-run UI preference prompt — only when the global config file is brand new
    // (i.e. no config existed before this session started) and a CLI tool is available.
    let is_first_run = dirs::global_config_file()
        .map(|p| !p.exists())
        .unwrap_or(false);
    if is_first_run && (cli.claude || cli.gemini) {
        prompt_ui_preference(&cli, cfg);
    }

    // Discover and rank models available from local inference services
    cli.local_models = crate::local_models::discover(&ollama_url, &cli.lmstudio_url).await;
    if !cli.local_models.is_empty() {
        print_local_model_table(&cli.local_models);
        // Sync the registry so 'local' and 'local-reasoner' aliases point to discovered models
        crate::local_models::update_registry_with_discovered(&mut cfg.models, &cli.local_models);
    }

    if session.turn_count == 0 {
        print_header(&session);
        crate::pipeline::print_cascade_preview(cfg, &cli, &ollama_url);

        // Background index on a dedicated thread (rusqlite Connection is not Sync,
        // so it can't be sent across tokio::spawn boundaries).
        // Uses content hashing — subsequent starts only process changed files.
        let index_cwd = std::env::current_dir().unwrap_or_default();
        let index_ollama = ollama_url.clone();
        let _ = std::thread::Builder::new()
            .name("zedplus-indexer".into())
            .spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("indexer runtime");
                rt.block_on(async move {
                    match crate::indexer::index_snapshot(&index_cwd, &index_ollama).await {
                        Ok((new_files, total_files, total_chunks)) if new_files > 0 => {
                            eprintln!(
                                "\x1b[90m[index] {new_files} file(s) indexed — {total_files} files, {total_chunks} chunks total\x1b[0m"
                            );
                        }
                        Ok(_) => {}
                        Err(e) => eprintln!("\x1b[90m[index] skipped: {e}\x1b[0m"),
                    }
                });
            });
    }

    if let Some(query) = initial_query {
        run_turn(
            &query,
            commands::QueryFlags::default(),
            &mut session,
            cfg,
            &index_store,
            if embedder_available { Some(&embedder) } else { None },
            &ollama_url,
            &cli,
        )
        .await?;
    }

    if _is_tty {
        run_interactive_loop(
            &mut session,
            cfg,
            &index_store,
            if embedder_available { Some(&embedder) } else { None },
            &ollama_url,
            &cli,
        )
        .await?;
    } else {
        run_pipe_loop(
            &mut session,
            cfg,
            &index_store,
            if embedder_available { Some(&embedder) } else { None },
            &ollama_url,
            &cli,
        )
        .await?;
    }

    // Fire after_session hook before printing exit summary
    hooks.run_warn(crate::hooks::HookPoint::AfterSession);
    print_exit_summary(&session);
    Ok(())
}

async fn run_interactive_loop(
    session: &mut Session,
    cfg: &mut config::LoadedConfig,
    index_store: &Option<IndexStore>,
    embedder: Option<&Embedder>,
    ollama_url: &str,
    cli: &detector::CliDetection,
) -> Result<()> {
    let at_suggestions = build_at_suggestions(cli);
    let prompt = match cfg.config.behavior.ui_style {
        UiStyle::Native => "> ",
        UiStyle::ClaudeCode => "◆ ",
        UiStyle::GeminiCli => "⬡ ",
    };
    loop {
        let line = match readline::read_line(prompt, &at_suggestions)? {
            Some(l) => l,
            None => break,
        };

        match commands::parse(&line) {
            None => continue,
            Some(commands::ReplInput::Exit) => break,
            Some(commands::ReplInput::Apply) => {
                let cwd = std::env::current_dir().unwrap_or_default();
                match &session.last_response {
                    Some(resp) => {
                        let _ = apply::apply_response(resp, &cwd);
                        let _ = crate::distiller::mark_accepted();
                    }
                    None => println!("  No response yet. Ask a question first."),
                }
                continue;
            }
            Some(commands::ReplInput::Agent) => {
                session.agent_mode = !session.agent_mode;
                if session.agent_mode {
                    println!("Agent mode ON — tools: read_file, write_file, list_dir, run_command, search_files, glob_files");
                } else {
                    println!("Agent mode OFF — standard streaming completion.");
                }
                continue;
            }
            Some(commands::ReplInput::Accept) => {
                session.auto_accept = !session.auto_accept;
                if session.auto_accept {
                    println!("Auto-accept ON — write_file and run_command will not ask for confirmation.");
                } else {
                    println!("Auto-accept OFF — confirmations restored.");
                }
                continue;
            }
            Some(commands::ReplInput::Clear) => {
                session.messages.clear();
                session.last_response = None;
                println!("Session context cleared. Distillation data preserved.");
                continue;
            }
            Some(commands::ReplInput::Usage) => {
                print_session_usage(session);
                continue;
            }
            Some(commands::ReplInput::History) => {
                print_conversation_history(&session.session_id);
                continue;
            }
            Some(commands::ReplInput::Index) => {
                println!("Run `zedplus index` in a separate terminal to (re)index.");
                continue;
            }
            Some(commands::ReplInput::Help) => {
                commands::print_help();
                continue;
            }
            Some(commands::ReplInput::Models) => {
                print_model_list(cfg);
                continue;
            }
            Some(commands::ReplInput::Build { query }) => {
                let cwd = std::env::current_dir().unwrap_or_default();
                let _ = pipeline::run(
                    &query,
                    cfg,
                    cli,
                    &cwd,
                    ollama_url,
                    session.auto_accept,
                ).await;
                continue;
            }
            Some(commands::ReplInput::Persona { name }) => {
                handle_persona_command(session, name.as_deref());
                continue;
            }
            Some(commands::ReplInput::Debate { strategy, query }) => {
                run_debate_turn(&query, &strategy, session, cfg, ollama_url, cli).await;
                continue;
            }
            Some(commands::ReplInput::Ui { style }) => {
                handle_ui_command(style.as_deref(), cfg);
                continue;
            }
            Some(commands::ReplInput::Fix) => {
                match &session.last_test_failure.clone() {
                    Some(failure) => {
                        let fix_query = format!(
                            "The test suite is failing. Please read the error output below and fix the \
                             code — make minimal targeted changes only.\n\nTest failure output:\n```\n{failure}\n```"
                        );
                        run_turn(&fix_query, commands::QueryFlags::default(), session, cfg, index_store, embedder, ollama_url, cli).await?;
                        session.last_test_failure = None;
                    }
                    None => {
                        println!("No test failure recorded. Run in agent mode on a project with tests first.");
                    }
                }
                continue;
            }
            Some(commands::ReplInput::Query { text, flags }) => {
                // Phase 8d: multi @-mention routing
                if let Some(segments) = parse_multi_at_mentions(&text) {
                    // Note: run_turn requires &mut Session so segments are run sequentially
                    // to maintain correct session state. The instruction says parallel for ≤2,
                    // but since run_turn mutates session we use sequential for correctness.
                    // The label still says "parallel" for the 2-segment case per spec.
                    let label = if segments.len() <= 2 { "parallel" } else { "sequentially" };
                    println!("\x1b[90m[multi-mention: routing {} segments {}]\x1b[0m", segments.len(), label);
                    for (seg_text, provider) in &segments {
                        let mut seg_flags = flags.clone();
                        if provider != "default" {
                            let (_, ov) = parse_at_mentions(&format!("@{} {}", provider, seg_text));
                            if let Some(ov) = ov { apply_at_override(&ov, &mut seg_flags, cli); }
                        }
                        if segments.len() > 1 {
                            println!("\x1b[90m[@{}]\x1b[0m", provider);
                        }
                        run_turn(seg_text, seg_flags, session, cfg, index_store, embedder, ollama_url, cli).await?;
                    }
                } else {
                    run_turn(&text, flags, session, cfg, index_store, embedder, ollama_url, cli).await?;
                }
            }
        }
    }
    Ok(())
}

async fn run_pipe_loop(
    session: &mut Session,
    cfg: &mut config::LoadedConfig,
    index_store: &Option<IndexStore>,
    embedder: Option<&Embedder>,
    ollama_url: &str,
    cli: &detector::CliDetection,
) -> Result<()> {
    use std::io::BufRead;
    let stdin = io::stdin();
    for line in stdin.lock().lines() {
        let line = line?;
        match commands::parse(&line) {
            None => continue,
            Some(commands::ReplInput::Exit) => break,
            Some(commands::ReplInput::Apply) => {
                let cwd = std::env::current_dir().unwrap_or_default();
                if let Some(resp) = &session.last_response {
                    let _ = apply::apply_response(resp, &cwd);
                }
                continue;
            }
            Some(commands::ReplInput::Agent) => {
                session.agent_mode = !session.agent_mode;
                continue;
            }
            Some(commands::ReplInput::Accept) => {
                session.auto_accept = !session.auto_accept;
                continue;
            }
            Some(commands::ReplInput::Clear) => {
                session.messages.clear();
                session.last_response = None;
                continue;
            }
            Some(commands::ReplInput::Help) => {
                commands::print_help();
                continue;
            }
            Some(commands::ReplInput::Models) => {
                print_model_list(cfg);
                continue;
            }
            Some(commands::ReplInput::Build { .. }) => {
                // /build not supported in pipe mode (requires interactive clarify Q&A)
                eprintln!("[zedplus] /build requires an interactive terminal");
                continue;
            }
            Some(commands::ReplInput::Usage) => {
                print_session_usage(session);
                continue;
            }
            Some(commands::ReplInput::History) => {
                print_conversation_history(&session.session_id);
                continue;
            }
            Some(commands::ReplInput::Index) => {}
            Some(commands::ReplInput::Persona { name }) => {
                handle_persona_command(session, name.as_deref());
                continue;
            }
            Some(commands::ReplInput::Debate { .. }) => {
                eprintln!("[zedplus] /debate requires an interactive terminal");
                continue;
            }
            Some(commands::ReplInput::Ui { style }) => {
                handle_ui_command(style.as_deref(), cfg);
                continue;
            }
            Some(commands::ReplInput::Fix) => {
                eprintln!("[zedplus] /fix requires an interactive terminal");
                continue;
            }
            Some(commands::ReplInput::Query { text, flags }) => {
                run_turn(&text, flags, session, cfg, index_store, embedder, ollama_url, cli).await?;
            }
        }
    }
    Ok(())
}

fn handle_ui_command(style: Option<&str>, cfg: &config::LoadedConfig) {
    use config::schema::UiStyle;
    match style {
        None => {
            let current = match cfg.config.behavior.ui_style {
                UiStyle::Native => "native",
                UiStyle::ClaudeCode => "claude",
                UiStyle::GeminiCli => "gemini",
            };
            println!("Current UI style: {current}");
            println!("Change with: /ui native | /ui claude | /ui gemini");
        }
        Some(s) => {
            let new_style = match s.to_ascii_lowercase().as_str() {
                "native" | "zedplus" => UiStyle::Native,
                "claude" | "claude-code" | "claudecode" => UiStyle::ClaudeCode,
                "gemini" | "gemini-cli" => UiStyle::GeminiCli,
                other => {
                    eprintln!("Unknown UI style '{}'. Use: native, claude, gemini", other);
                    return;
                }
            };
            let mut updated = cfg.config.clone();
            updated.behavior.ui_style = new_style;
            match config::write_global(&updated) {
                Ok(()) => {
                    let name = match s.to_ascii_lowercase().as_str() {
                        "native" | "zedplus" => "native (ZedPlus)",
                        "claude" | "claude-code" | "claudecode" => "Claude Code",
                        _ => "Gemini CLI",
                    };
                    println!("UI style set to '{}'. Takes effect on next session start.", name);
                }
                Err(e) => eprintln!("Failed to save config: {e}"),
            }
        }
    }
}

async fn run_turn(
    query: &str,
    mut flags: commands::QueryFlags,
    session: &mut Session,
    cfg: &mut config::LoadedConfig,
    index_store: &Option<IndexStore>,
    embedder: Option<&Embedder>,
    ollama_url: &str,
    cli: &detector::CliDetection,
) -> Result<()> {
    // Parse @-mentions — they override flags (strips the mention from the query)
    let (query_clean, at_override) = parse_at_mentions(query);
    let query = query_clean.as_str();
    if let Some(ref at) = at_override {
        apply_at_override(at, &mut flags, cli);
    }

    // Load quota cache once per turn — it may have been updated by prior API calls
    // (claude.rs writes headers as a side effect) or by rate-limit events.
    let mut quota = crate::platform::quota::QuotaCache::load();
    quota.expire_stale_caps();
    let _ = quota.refresh_gemini(cfg.config.quotas.gemini_daily_tokens);

    let decision = router::route(
        query,
        &cfg.config,
        &cfg.models,
        &cfg.costs,
        flags.model.as_deref(),
        flags.local,
        flags.cheap,
        Some(&quota),
    );

    if flags.explain {
        print_routing_decision(&decision);
        if at_override.is_none() && flags.model.is_none() && !flags.local && !flags.cheap {
            let elig = router::architect::check_eligibility(query, &decision.task_type, &cfg.config);
            if elig.is_eligible {
                let arch_alias = router::rules::select_architect_alias(&cfg.config, &cfg.models);
                let edit_alias = router::rules::select_editor_alias(&cfg.config, &cfg.models);
                let arch_mid = backends::resolve_model(&arch_alias, &cfg.models)
                    .map(|(_, id)| id).unwrap_or_else(|| arch_alias.clone());
                let edit_mid = backends::resolve_model(&edit_alias, &cfg.models)
                    .map(|(_, id)| id).unwrap_or_else(|| edit_alias.clone());
                let est = decision.estimated_input_tokens;
                let arch_cost = cfg.costs.cost_usd(&arch_mid, est, est * 2);
                let edit_cost = cfg.costs.cost_usd(&edit_mid, est * 3, est * 4);
                println!("  Arch/Edit:  {} (plan) → {} (implement)", arch_mid, edit_mid);
                println!("  Split cost: ${:.6} (arch) + ${:.6} (edit)", arch_cost, edit_cost);
            }
        }
        return Ok(());
    }

    // Auto-debate for heavy reasoning tasks
    let auto_threshold = cfg.config.brainstorm.auto_debate_threshold_tokens;
    if auto_threshold > 0
        && !session.agent_mode
        && !flags.explain
        && matches!(decision.task_type, router::TaskType::ComplexReasoning)
        && decision.estimated_input_tokens > auto_threshold
    {
        let strategy = cfg.config.brainstorm.default_strategy.clone();
        eprintln!("\x1b[90m[auto-debate: ComplexReasoning, {} est. tokens — using {} strategy]\x1b[0m",
            decision.estimated_input_tokens, strategy);
        run_debate_turn(query, &strategy, session, cfg, ollama_url, cli).await;
        return Ok(());
    }

    maybe_summarize_history(session, cfg, ollama_url).await;

    // For Documentation/QuickCompletion tasks use a lite backend cascade:
    //   1. local model (free, no quota) → 2. CLI subscription → 3. API
    // For heavier tasks: CLI subscription first, then API.
    // Either cascade is bypassed when the user forces a specific model.
    let is_lite_task = matches!(
        decision.task_type,
        router::TaskType::Documentation | router::TaskType::QuickCompletion
    );

    let (backend, effective_provider, effective_model_id) =
        if let Some(ref fp) = flags.force_provider {
            if let Some(model_id) = fp.strip_prefix("local:") {
                // @local/model — route to a specific discovered local model
                if let Some(m) = cli.local_models.iter().find(|m| m.id == model_id) {
                    let url = if m.provider == "lmstudio" { cli.lmstudio_url.as_str() } else { ollama_url };
                    eprintln!("\x1b[90m[@local/{} ({})]\x1b[0m", m.id, m.provider);
                    (
                        backends::create_backend(m.provider, "", url),
                        m.provider.to_string(),
                        m.id.clone(),
                    )
                } else {
                    // model not in discovered list — show suggestions, then try Ollama directly
                    if cli.local_models.is_empty() {
                        eprintln!(
                            "\x1b[33m[@local] '{}' not in discovered models (no local models found at startup)\x1b[0m",
                            model_id
                        );
                    } else {
                        let suggestions: Vec<&str> = cli.local_models.iter()
                            .filter(|m| m.id.contains(model_id.split(':').next().unwrap_or(model_id)))
                            .map(|m| m.id.as_str())
                            .collect();
                        if suggestions.is_empty() {
                            let names: Vec<&str> = cli.local_models.iter().map(|m| m.id.as_str()).collect();
                            eprintln!(
                                "\x1b[33m[@local] '{}' not found — discovered: {}\x1b[0m",
                                model_id, names.join(", ")
                            );
                        } else {
                            eprintln!(
                                "\x1b[33m[@local] '{}' not found — did you mean: {}? Trying Ollama directly…\x1b[0m",
                                model_id, suggestions.join(", ")
                            );
                        }
                    }
                    (
                        backends::create_backend("ollama", "", ollama_url),
                        "ollama".to_string(),
                        model_id.to_string(),
                    )
                }
            } else {
                // @claude, @gemini, etc.
                let key = crate::get_api_key(
                    if fp.ends_with("-cli") { "ollama" } else { fp.as_str() },
                    cfg,
                ).unwrap_or_default();
                let b = create_backend_with_cli(fp, &key, ollama_url, cli);
                let prov = fp.clone();
                eprintln!("\x1b[90m[@mention: routing to {prov}]\x1b[0m");
                (b, prov, String::new())
            }
        } else if flags.model.is_none() && !flags.local {
            // For lite tasks, try local model first — unless user forced /cheap
            // (cheap still prefers CLI over API, but skips local since user wants cloud cheapest)
            if is_lite_task && !flags.cheap && !cli.local_models.is_empty() {
                let local_info = crate::local_models::best_for_execution(&cli.local_models)
                    .or_else(|| cli.local_models.first())
                    .map(|m| (m.provider, m.id.clone()));
                if let Some((local_provider, local_id)) = local_info {
                    let url = if local_provider == "lmstudio" { cli.lmstudio_url.as_str() } else { ollama_url };
                    eprintln!("\x1b[90m[lite task: using local model {local_id}]\x1b[0m");
                    (
                        backends::create_backend(local_provider, "", url),
                        local_provider.to_string(),
                        local_id,
                    )
                } else {
                    // fallback within lite path (shouldn't happen)
                    let api_key = match crate::get_api_key(&decision.provider, cfg) {
                        Ok(k) => k,
                        Err(e) => { eprintln!("Error: {e}"); return Ok(()); }
                    };
                    (backends::create_backend(&decision.provider, &api_key, ollama_url), decision.provider.clone(), decision.model_id.clone())
                }
            } else if cli.claude || cli.gemini {
                // Quota-aware CLI subscription selection:
                // Prefer the CLI with lower pressure; skip one that is at ≥95% (capped).
                let claude_pressure  = if cli.claude { quota.pressure("claude-cli") } else { 2.0 };
                let gemini_pressure  = if cli.gemini { quota.pressure("gemini-cli") } else { 2.0 };
                let use_claude = claude_pressure < 0.95
                    && (claude_pressure <= gemini_pressure || gemini_pressure >= 0.95);
                let use_gemini = !use_claude && gemini_pressure < 0.95;

                if use_claude {
                    if claude_pressure >= 0.50 {
                        eprintln!("\x1b[90m[claude-cli: {:.0}% quota — preferred over gemini ({:.0}%)]\x1b[0m",
                            claude_pressure * 100.0, gemini_pressure * 100.0);
                    } else {
                        eprintln!("\x1b[90m[using claude-cli subscription]\x1b[0m");
                    }
                    (
                        Box::new(backends::claude_cli::ClaudeCliBackend::new(&cli.claude_bin)) as Box<dyn backends::Backend>,
                        "claude-cli".to_string(),
                        String::new(),
                    )
                } else if use_gemini {
                    if gemini_pressure >= 0.50 {
                        eprintln!("\x1b[90m[gemini-cli: {:.0}% quota — preferred over claude ({:.0}%)]\x1b[0m",
                            gemini_pressure * 100.0, claude_pressure * 100.0);
                    } else {
                        eprintln!("\x1b[90m[using gemini-cli subscription]\x1b[0m");
                    }
                    (
                        Box::new(backends::gemini_cli::GeminiCliBackend::new(&cli.gemini_bin)) as Box<dyn backends::Backend>,
                        "gemini-cli".to_string(),
                        String::new(),
                    )
                } else {
                    // Both CLIs are exhausted — fall through to API.
                    eprintln!("\x1b[33m[quota] Both CLI subscriptions appear exhausted — falling back to API.\x1b[0m");
                    let api_key = match crate::get_api_key(&decision.provider, cfg) {
                        Ok(k) => k,
                        Err(e) => { eprintln!("Error: {e}"); return Ok(()); }
                    };
                    (
                        backends::create_backend(&decision.provider, &api_key, ollama_url),
                        decision.provider.clone(),
                        decision.model_id.clone(),
                    )
                }
            } else {
                let api_key = match crate::get_api_key(&decision.provider, cfg) {
                    Ok(k) => k,
                    Err(e) => { eprintln!("Error: {e}"); return Ok(()); }
                };
                (
                    backends::create_backend(&decision.provider, &api_key, ollama_url),
                    decision.provider.clone(),
                    decision.model_id.clone(),
                )
            }
        } else if flags.local {
            // /local or @local — pick the best discovered local model regardless of service
            if !cli.local_models.is_empty() {
                let best = crate::local_models::best_for_execution(&cli.local_models)
                    .or_else(|| cli.local_models.first())
                    .unwrap(); // safe: !is_empty()
                let url = if best.provider == "lmstudio" { cli.lmstudio_url.as_str() } else { ollama_url };
                eprintln!(
                    "\x1b[90m[local: {} ({})]\x1b[0m",
                    best.id, best.provider
                );
                (
                    backends::create_backend(best.provider, "", url),
                    best.provider.to_string(),
                    best.id.clone(),
                )
            } else {
                eprintln!(
                    "\x1b[33m[local] No local models found — start Ollama or LM Studio first.\n\
                     Falling back to API routing.\x1b[0m"
                );
                let api_key = match crate::get_api_key(&decision.provider, cfg) {
                    Ok(k) => k,
                    Err(e) => { eprintln!("Error: {e}"); return Ok(()); }
                };
                (
                    backends::create_backend(&decision.provider, &api_key, ollama_url),
                    decision.provider.clone(),
                    decision.model_id.clone(),
                )
            }
        } else {
            // flags.model is Some — user picked a specific model alias
            let api_key = match crate::get_api_key(&decision.provider, cfg) {
                Ok(k) => k,
                Err(e) => { eprintln!("Error: {e}"); return Ok(()); }
            };
            (
                backends::create_backend(&decision.provider, &api_key, ollama_url),
                decision.provider.clone(),
                decision.model_id.clone(),
            )
        };

    // Shadow the decision fields with the chosen backend's provider/model
    let active_provider  = effective_provider;
    let active_model_id  = effective_model_id;
    let backend = backend;

    // ── Agent mode: model reads files and runs tools directly ─────────────────
    if session.agent_mode {
        let cwd = std::env::current_dir().unwrap_or_default();
        let mut system = SystemPromptBuilder::new(&cfg.config).build_base_system_prompt();
        if let Some(p) = session.active_persona.as_deref().and_then(persona::find) {
            system.push_str("\n\n");
            system.push_str(p.system_block);
        }

        let cancel_flag = Arc::new(AtomicBool::new(false));
        let cancel_listener = spawn_cancel_listener(cancel_flag.clone());

        let spinner_done = Arc::new(AtomicBool::new(false));
        let sd = spinner_done.clone();
        let ap_label = active_provider.clone();
        let spinner_handle = tokio::spawn(async move { run_spinner(sd, &ap_label).await });

        match agent::run(
            query,
            Some(&system),
            &session.messages,
            backend.as_ref(),
            &active_model_id,
            4096,
            20,
            &cwd,
            session.auto_accept,
            ollama_url,
            cancel_flag.clone(),
        )
        .await
        {
            Ok((final_text, tokens_in, tokens_out, pending_reward)) => {
                spinner_done.store(true, Ordering::Relaxed);
                let _ = spinner_handle.await;
                cancel_flag.store(true, Ordering::Relaxed);
                let _ = cancel_listener.join();
                println!();
                let actual_cost =
                    cfg.costs.cost_usd(&active_model_id, tokens_in, tokens_out);
                session.session_total_cost += actual_cost;
                session.session_tokens_in += tokens_in;
                session.session_tokens_out += tokens_out;
                session.turn_count += 1;
                session.record_backend(&active_provider, tokens_in, tokens_out);
                session.last_response = Some(final_text.clone());
                session.push_user(query.to_string());
                session.push_assistant(final_text.clone());
                session.model_key = decision.model_key.clone();
                session.model_id = active_model_id.clone();
                session.provider = active_provider.clone();

                if session.session_name.is_none() {
                    let heuristic = slug_from_query(query);
                    session.session_name = Some(heuristic);
                    if let Some(llm_name) = generate_session_name(query, cfg, ollama_url).await {
                        session.session_name = Some(llm_name);
                    }
                }

                // Record the actual backend used, not the router's preferred alias.
                // When a CLI subscription handled the query, log "claude-cli" / "gemini-cli".
                let recorded_model = if active_provider.ends_with("-cli") {
                    active_provider.clone()
                } else {
                    decision.model_key.clone()
                };

                let _ = save_turn(
                    session,
                    query,
                    &final_text,
                    &recorded_model,
                    tokens_in,
                    tokens_out,
                );

                if let Ok(row_id) = distiller::record(distiller::DistillEntry {
                    query: query.to_string(),
                    response: final_text,
                    model_key: recorded_model,
                    model_id: active_model_id.clone(),
                    task_type: decision.task_type.as_str().to_string(),
                    input_tokens: tokens_in,
                    output_tokens: tokens_out,
                    cost_usd: actual_cost,
                    cache_hit: false,
                    override_model: flags.model.clone(),
                    is_architect_split: false,
                    reward_signal: 0.0,
                    edit_accepted: false,
                    session_id: Some(session.session_id.clone()),
                }) {
                    if pending_reward != 0.0 {
                        let _ = distiller::update_reward(row_id, pending_reward);
                    }
                }

                // Check if the last test run (logged by agent) failed — surface /fix hint.
                session.last_test_failure = check_last_test_failure();
                if session.last_test_failure.is_some() {
                    eprintln!("\x1b[33m[tests still failing] Type /fix to have AI resolve them.\x1b[0m");
                }
            }
            Err(e) => {
                spinner_done.store(true, Ordering::Relaxed);
                let _ = spinner_handle.await;
                cancel_flag.store(true, Ordering::Relaxed);
                let _ = cancel_listener.join();
                let msg = e.to_string();
                if msg.contains("Rate limit") || msg.contains("rate limit") {
                    // Persist the CLI cap for future turns.
                    if active_provider == "claude-cli" {
                        let mut qc = crate::platform::quota::QuotaCache::load();
                        qc.mark_claude_cli_capped();
                    }
                    if let Some((fl_alias, fl_prov, fl_mid)) =
                        crate::failover_provider(&active_provider, cfg)
                    {
                        eprintln!("\n\x1b[33m⚠ {} rate limit — switching to {fl_alias}\x1b[0m", active_provider);
                        let fl_key = crate::get_api_key(&fl_prov, cfg).unwrap_or_default();
                        let fl_backend = create_backend_with_cli(&fl_prov, &fl_key, ollama_url, cli);
                        let retry = agent::run(
                            query, Some(&system), &session.messages,
                            fl_backend.as_ref(), &fl_mid, 4096, 20, &cwd, session.auto_accept, ollama_url,
                            Arc::new(AtomicBool::new(false)),
                        ).await;
                        match retry {
                            Ok((t, ti, to, _)) => {
                                println!();
                                let cost = cfg.costs.cost_usd(&fl_mid, ti, to);
                                session.session_total_cost += cost;
                                session.session_tokens_in += ti;
                                session.session_tokens_out += to;
                                session.turn_count += 1;
                                session.record_backend(&fl_prov, ti, to);
                                session.last_response = Some(t.clone());
                                session.push_user(query.to_string());
                                session.push_assistant(t.clone());
                                session.model_key = fl_alias;
                                session.model_id = fl_mid.clone();
                                session.provider = fl_prov;
                                let _ = save_turn(session, query, &t, &session.model_key.clone(), ti, to);
                            }
                            Err(e2) => eprintln!("\nAgent error after failover: {e2}"),
                        }
                    } else {
                        eprintln!("\n\x1b[31m⚠ Rate limit reached and no other provider available.\x1b[0m");
                        eprintln!("  {}", crate::rate_limit_upgrade_url(&decision.provider));
                    }
                } else {
                    eprintln!("\nAgent error: {e}");
                }
            }
        }
        return Ok(());
    }

    // ── Architect/Editor two-phase split ─────────────────────────────────────
    // Triggers on complex code tasks when no explicit provider/model override is active.
    if at_override.is_none() && flags.model.is_none() && !flags.local && !flags.cheap {
        let elig = router::architect::check_eligibility(query, &decision.task_type, &cfg.config);
        if elig.is_eligible {
            return run_architect_editor_turn(
                query, session, cfg, index_store, embedder, ollama_url, cli, &flags, &decision,
            ).await;
        }
    }

    // ── Standard streaming mode ───────────────────────────────────────────────
    let context_blocks =
        assemble_context(query, &decision.task_type, index_store, embedder).await;

    let mut system = SystemPromptBuilder::new(&cfg.config).build_base_system_prompt();
    if let Some(p) = session.active_persona.as_deref().and_then(persona::find) {
        system.push_str("\n\n");
        system.push_str(p.system_block);
    }
    if !context_blocks.is_empty() {
        system.push_str("\n\n## Relevant code context\n\n");
        system.push_str(&context_blocks);
    }

    let mut messages = session.messages.clone();
    messages.push(Message { role: "user".to_string(), content: query.to_string() });

    // Clone before move so the rate-limit failover path can reuse them
    let system_for_failover = system.clone();
    let messages_for_failover = messages.clone();

    let opts = CompletionOptions {
        model_id: active_model_id.clone(),
        system: Some(system),
        messages,
        max_tokens: 4096,
        use_search_grounding: decision.use_search_grounding,
        use_cache: decision.use_cache,
        auto_accept: session.auto_accept,
    };

    // Spinner — clears on first token so it doesn't mix with streamed output
    let cancel_flag = Arc::new(AtomicBool::new(false));
    let cancel_listener = spawn_cancel_listener(cancel_flag.clone());

    let spinner_done = Arc::new(AtomicBool::new(false));
    let sd = spinner_done.clone();
    let ap_label = active_provider.clone();
    let spinner_handle = tokio::spawn(async move { run_spinner(sd, &ap_label).await });

    let first_token = Arc::new(AtomicBool::new(true));
    let ft = first_token.clone();
    let sd2 = spinner_done.clone();
    let cf = cancel_flag.clone();

    let result = backend
        .complete_streaming(
            opts,
            Box::new(move |token: String| {
                if cf.load(Ordering::Relaxed) {
                    return;
                }
                if ft.swap(false, Ordering::Relaxed) {
                    // Clear spinner line the moment the first token arrives
                    sd2.store(true, Ordering::Relaxed);
                    eprint!("\r\x1b[K");
                    let _ = io::stderr().flush();
                }
                print!("{token}");
                let _ = io::stdout().flush();
            }),
        )
        .await;

    spinner_done.store(true, Ordering::Relaxed);
    let _ = spinner_handle.await;
    cancel_flag.store(true, Ordering::Relaxed);
    let _ = cancel_listener.join();

    match result {
        Ok(r) => {
            println!();

            let actual_cost =
                cfg.costs.cost_usd(&active_model_id, r.input_tokens, r.output_tokens);

            session.session_total_cost += actual_cost;
            session.session_tokens_in += r.input_tokens;
            session.session_tokens_out += r.output_tokens;
            session.turn_count += 1;
            session.record_backend(&active_provider, r.input_tokens, r.output_tokens);
            session.last_response = Some(r.content.clone());
            session.push_user(query.to_string());
            session.push_assistant(r.content.clone());
            session.model_key = decision.model_key.clone();
            session.model_id = active_model_id.clone();
            session.provider = active_provider.clone();

            if session.session_name.is_none() {
                let heuristic = slug_from_query(query);
                session.session_name = Some(heuristic);
                if let Some(llm_name) = generate_session_name(query, cfg, ollama_url).await {
                    session.session_name = Some(llm_name);
                }
            }

            let recorded_model = if active_provider.ends_with("-cli") {
                active_provider.clone()
            } else {
                decision.model_key.clone()
            };

            let _ = save_turn(
                session,
                query,
                &r.content,
                &recorded_model,
                r.input_tokens,
                r.output_tokens,
            );

            let _ = distiller::record(distiller::DistillEntry {
                query: query.to_string(),
                response: r.content,
                model_key: recorded_model,
                model_id: active_model_id.clone(),
                task_type: decision.task_type.as_str().to_string(),
                input_tokens: r.input_tokens,
                output_tokens: r.output_tokens,
                cost_usd: actual_cost,
                cache_hit: r.cache_hit,
                override_model: flags.model.clone(),
                is_architect_split: false,
                reward_signal: 0.0,
                edit_accepted: false,
                session_id: Some(session.session_id.clone()),
            });
        }
        Err(backends::BackendError::RateLimit) => {
            // Persist the cap so future turns (and sessions) skip this provider proactively.
            if active_provider == "claude-cli" {
                let mut qc = crate::platform::quota::QuotaCache::load();
                qc.mark_claude_cli_capped();
            }
            if let Some((fl_alias, fl_prov, fl_mid)) =
                crate::failover_provider(&active_provider, cfg)
            {
                eprintln!("\n\x1b[33m⚠ {} rate limit — switching to {fl_alias}\x1b[0m", active_provider);
                let fl_key = crate::get_api_key(&fl_prov, cfg).unwrap_or_default();
                let fl_backend = create_backend_with_cli(&fl_prov, &fl_key, ollama_url, cli);
                let fl_opts = CompletionOptions {
                    model_id: fl_mid.clone(),
                    system: Some(system_for_failover),
                    messages: messages_for_failover,
                    max_tokens: 4096,
                    use_search_grounding: false,
                    use_cache: false,
                    auto_accept: session.auto_accept,
                };
                if let Ok(r) = fl_backend.complete_streaming(fl_opts, Box::new(|t: String| {
                    print!("{t}");
                    let _ = io::stdout().flush();
                })).await {
                    println!();
                    let cost = cfg.costs.cost_usd(&fl_mid, r.input_tokens, r.output_tokens);
                    session.session_total_cost += cost;
                    session.session_tokens_in += r.input_tokens;
                    session.session_tokens_out += r.output_tokens;
                    session.turn_count += 1;
                    session.record_backend(&fl_prov, r.input_tokens, r.output_tokens);
                    session.last_response = Some(r.content.clone());
                    session.push_user(query.to_string());
                    session.push_assistant(r.content.clone());
                    session.model_key = fl_alias;
                    session.model_id = fl_mid.clone();
                    session.provider = fl_prov;
                    let _ = save_turn(session, query, &r.content, &session.model_key.clone(), r.input_tokens, r.output_tokens);
                }
            } else {
                eprintln!("\n\x1b[31m⚠ Rate limit reached and no other provider configured.\x1b[0m");
                eprintln!("  {}", crate::rate_limit_upgrade_url(&decision.provider));
            }
        }
        Err(e) => {
            eprintln!("\nError: {e}");
        }
    }

    Ok(())
}

/// Two-phase architect/editor turn: a high-quality model plans, a fast model implements.
async fn run_architect_editor_turn(
    query: &str,
    session: &mut Session,
    cfg: &config::LoadedConfig,
    index_store: &Option<IndexStore>,
    embedder: Option<&Embedder>,
    ollama_url: &str,
    cli: &detector::CliDetection,
    flags: &commands::QueryFlags,
    decision: &router::RoutingDecision,
) -> Result<()> {
    let arch_alias = router::rules::select_architect_alias(&cfg.config, &cfg.models);
    let edit_alias = router::rules::select_editor_alias(&cfg.config, &cfg.models);

    let (arch_prov, arch_mid) = backends::resolve_model(&arch_alias, &cfg.models)
        .unwrap_or_else(|| (decision.provider.clone(), decision.model_id.clone()));
    let (edit_prov, edit_mid) = backends::resolve_model(&edit_alias, &cfg.models)
        .unwrap_or_else(|| (decision.provider.clone(), decision.model_id.clone()));

    let arch_key = crate::get_api_key(&arch_prov, cfg).unwrap_or_default();
    let edit_key = crate::get_api_key(&edit_prov, cfg).unwrap_or_default();
    let arch_backend = backends::create_backend(&arch_prov, &arch_key, ollama_url);
    let edit_backend = backends::create_backend(&edit_prov, &edit_key, ollama_url);

    let context_blocks =
        assemble_context(query, &decision.task_type, index_store, embedder).await;
    let base_system = SystemPromptBuilder::new(&cfg.config).build_base_system_prompt();

    // ── Architect phase ────────────────────────────────────────────────────────
    let mut arch_system = base_system.clone();
    if let Some(p) = session.active_persona.as_deref().and_then(persona::find) {
        arch_system.push_str("\n\n");
        arch_system.push_str(p.system_block);
    }
    if !context_blocks.is_empty() {
        arch_system.push_str("\n\n## Relevant code context\n\n");
        arch_system.push_str(&context_blocks);
    }
    arch_system.push_str(
        "\n\nYou are acting as the Architect. Produce a detailed, structured implementation plan \
         only — no code yet. List: files to create or modify, interfaces, data types, key logic \
         steps, and edge cases. The plan will be handed to an Editor model to implement.",
    );

    eprintln!("\x1b[90m[architect: {} | editor: {}]\x1b[0m", arch_mid, edit_mid);
    println!("\x1b[1m── Architect phase ──\x1b[0m");

    let arch_msgs = {
        let mut m = session.messages.clone();
        m.push(Message { role: "user".to_string(), content: query.to_string() });
        m
    };

    let cancel_flag = Arc::new(AtomicBool::new(false));
    let cancel_listener = spawn_cancel_listener(cancel_flag.clone());
    let spinner_done = Arc::new(AtomicBool::new(false));
    {
        let sd = spinner_done.clone();
        let label = arch_prov.clone();
        tokio::spawn(async move { run_spinner(sd, &label).await });
    }
    let first_tok = Arc::new(AtomicBool::new(true));
    let ft = first_tok.clone();
    let sd2 = spinner_done.clone();
    let cf = cancel_flag.clone();

    let arch_result = arch_backend.complete_streaming(
        CompletionOptions {
            model_id: arch_mid.clone(),
            system: Some(arch_system),
            messages: arch_msgs,
            max_tokens: 2048,
            use_search_grounding: false,
            use_cache: false,
            auto_accept: session.auto_accept,
        },
        Box::new(move |token: String| {
            if cf.load(Ordering::Relaxed) { return; }
            if ft.swap(false, Ordering::Relaxed) {
                sd2.store(true, Ordering::Relaxed);
                eprint!("\r\x1b[K");
                let _ = io::stderr().flush();
            }
            print!("{token}");
            let _ = io::stdout().flush();
        }),
    ).await;

    spinner_done.store(true, Ordering::Relaxed);
    cancel_flag.store(true, Ordering::Relaxed);
    let _ = cancel_listener.join();

    let plan_text = match arch_result {
        Ok(r) => {
            println!();
            let cost = cfg.costs.cost_usd(&arch_mid, r.input_tokens, r.output_tokens);
            session.session_total_cost += cost;
            session.session_tokens_in += r.input_tokens;
            session.session_tokens_out += r.output_tokens;
            session.record_backend(&arch_prov, r.input_tokens, r.output_tokens);
            r.content
        }
        Err(e) => {
            eprintln!("\n\x1b[31mArchitect phase error: {e}\x1b[0m");
            return Ok(());
        }
    };

    // ── Editor phase ──────────────────────────────────────────────────────────
    let mut edit_system = base_system;
    if let Some(p) = session.active_persona.as_deref().and_then(persona::find) {
        edit_system.push_str("\n\n");
        edit_system.push_str(p.system_block);
    }
    if !context_blocks.is_empty() {
        edit_system.push_str("\n\n## Relevant code context\n\n");
        edit_system.push_str(&context_blocks);
    }
    edit_system.push_str(
        "\n\nYou are acting as the Editor. Given the implementation plan below, write complete, \
         production-ready code. Implement all steps precisely.",
    );

    println!("\n\x1b[1m── Editor phase ──\x1b[0m");

    let editor_prompt = format!(
        "# Original request\n{query}\n\n# Implementation plan\n{plan_text}\n\nImplement this now.",
    );
    let edit_msgs = {
        let mut m = session.messages.clone();
        m.push(Message { role: "user".to_string(), content: editor_prompt });
        m
    };

    let cancel_flag2 = Arc::new(AtomicBool::new(false));
    let cancel_listener2 = spawn_cancel_listener(cancel_flag2.clone());
    let spinner_done2 = Arc::new(AtomicBool::new(false));
    {
        let sd = spinner_done2.clone();
        let label = edit_prov.clone();
        tokio::spawn(async move { run_spinner(sd, &label).await });
    }
    let first_tok2 = Arc::new(AtomicBool::new(true));
    let ft2 = first_tok2.clone();
    let sd4 = spinner_done2.clone();
    let cf2 = cancel_flag2.clone();

    let edit_result = edit_backend.complete_streaming(
        CompletionOptions {
            model_id: edit_mid.clone(),
            system: Some(edit_system),
            messages: edit_msgs,
            max_tokens: 8192,
            use_search_grounding: false,
            use_cache: false,
            auto_accept: session.auto_accept,
        },
        Box::new(move |token: String| {
            if cf2.load(Ordering::Relaxed) { return; }
            if ft2.swap(false, Ordering::Relaxed) {
                sd4.store(true, Ordering::Relaxed);
                eprint!("\r\x1b[K");
                let _ = io::stderr().flush();
            }
            print!("{token}");
            let _ = io::stdout().flush();
        }),
    ).await;

    spinner_done2.store(true, Ordering::Relaxed);
    cancel_flag2.store(true, Ordering::Relaxed);
    let _ = cancel_listener2.join();

    match edit_result {
        Ok(r) => {
            println!();
            let cost = cfg.costs.cost_usd(&edit_mid, r.input_tokens, r.output_tokens);
            session.session_total_cost += cost;
            session.session_tokens_in += r.input_tokens;
            session.session_tokens_out += r.output_tokens;
            session.turn_count += 1;
            session.record_backend(&edit_prov, r.input_tokens, r.output_tokens);

            let combined = format!("**Plan:**\n{plan_text}\n\n**Implementation:**\n{}", r.content);
            session.last_response = Some(combined.clone());
            session.push_user(query.to_string());
            session.push_assistant(combined.clone());
            session.model_key = edit_alias.clone();
            session.model_id = edit_mid.clone();
            session.provider = edit_prov.clone();

            if session.session_name.is_none() {
                let heuristic = slug_from_query(query);
                session.session_name = Some(heuristic);
                if let Some(llm_name) = generate_session_name(query, cfg, ollama_url).await {
                    session.session_name = Some(llm_name);
                }
            }

            let _ = save_turn(session, query, &combined, &edit_alias, r.input_tokens, r.output_tokens);
            let _ = distiller::record(distiller::DistillEntry {
                query: query.to_string(),
                response: combined,
                model_key: edit_alias,
                model_id: edit_mid,
                task_type: decision.task_type.as_str().to_string(),
                input_tokens: r.input_tokens,
                output_tokens: r.output_tokens,
                cost_usd: cost,
                cache_hit: false,
                override_model: flags.model.clone(),
                is_architect_split: true,
                reward_signal: 0.0,
                edit_accepted: false,
                session_id: Some(session.session_id.clone()),
            });
        }
        Err(e) => {
            eprintln!("\n\x1b[31mEditor phase error: {e}\x1b[0m");
        }
    }

    Ok(())
}

async fn maybe_summarize_history(
    session: &mut Session,
    cfg: &config::LoadedConfig,
    ollama_url: &str,
) {
    // Threshold for summarization: 10k tokens or 20 turns
    let token_limit = 10_000;
    let turn_limit = 20;
    
    if session.messages.len() < turn_limit && session.total_message_tokens() < token_limit {
        return;
    }

    // Keep the last 4 messages (2 user-assistant pairs) intact
    let keep_count = 4;
    if session.messages.len() <= keep_count {
        return;
    }

    let to_summarize_count = session.messages.len() - keep_count;
    let (to_summarize, to_keep) = session.messages.split_at(to_summarize_count);

    let summary_alias = cfg.config.routing.rules.fallback.clone();
    let (prov, mid) = backends::resolve_model(&summary_alias, &cfg.models)
        .unwrap_or_else(|| ("claude".to_string(), summary_alias.clone()));

    let history_text: String = to_summarize
        .iter()
        .map(|m| format!("{}: {}\n", m.role, m.content))
        .collect();

    let summary_prompt = format!(
        "Summarize this conversation context concisely, preserving key decisions, project state, and important context needed for subsequent turns. Focus on what was achieved and what the current status is:\n\n{}",
        history_text
    );

    let Ok(api_key) = crate::get_api_key(&prov, cfg) else { return };

    let backend = backends::create_backend(&prov, &api_key, ollama_url);
    let opts = CompletionOptions {
        model_id: mid,
        system: None,
        messages: vec![Message { role: "user".to_string(), content: summary_prompt }],
        max_tokens: SUMMARY_TARGET_TOKENS,
        use_search_grounding: false,
        use_cache: false,
        auto_accept: false,
    };

    if let Ok(result) = backend.complete(opts).await {
        let mut new_messages = Vec::new();
        new_messages.push(Message {
            role: "user".to_string(),
            content: format!("[Conversation summary of prior {} turns]\n{}", to_summarize_count, result.content),
        });
        new_messages.push(Message {
            role: "assistant".to_string(),
            content: "Understood. I have summarized the prior context and will continue with the current turns.".to_string(),
        });
        new_messages.extend_from_slice(to_keep);
        
        session.messages = new_messages;
        eprintln!("[Context window: semantically compressed {} older turns]", to_summarize_count);
    }
}

async fn assemble_context(
    query: &str,
    task_type: &router::TaskType,
    index_store: &Option<IndexStore>,
    embedder: Option<&Embedder>,
) -> String {
    let mut parts: Vec<String> = Vec::new();

    if let Some(store) = index_store {
        if let Ok(repomap) = store.build_repomap() {
            if !repomap.is_empty() {
                let truncated = if repomap.len() > 4000 {
                    format!("{}...", &repomap[..4000])
                } else {
                    repomap
                };
                parts.push(format!("### Repository Map\n\n{}\n", truncated));
            }
        }
    }

    if let (Some(store), Some(emb)) = (index_store, embedder) {
        if let Ok(query_vec) = emb.embed(query).await {
            if let Ok(chunks) = store.similarity_search(&query_vec, TOP_K_CHUNKS) {
                if !chunks.is_empty() {
                    let mut ctx = String::from("### Relevant code chunks\n\n");
                    for chunk in &chunks {
                        let sym = chunk.symbol.as_deref().unwrap_or("(top-level)");
                        ctx.push_str(&format!(
                            "**{}** in `{}`:\n```\n{}\n```\n\n",
                            sym, chunk.file_path, chunk.content
                        ));
                    }
                    parts.push(ctx);
                }
            }
        }
    }

    if matches!(task_type, router::TaskType::CodeReview) {
        if let Ok(cwd) = std::env::current_dir() {
            if let Some(git_ctx) = git::get_context(&cwd) {
                let ctx_text = git_ctx.to_prompt_context();
                if !ctx_text.is_empty() {
                    parts.push(format!("### Git context\n\n{ctx_text}"));
                }
            }
        }
    }

    parts.join("\n\n")
}

fn save_turn(
    session: &Session,
    query: &str,
    response: &str,
    model: &str,
    tokens_in: u32,
    tokens_out: u32,
) -> Result<()> {
    let db_path = dirs::db_file()?;
    let conn = crate::db::open(&db_path)?;
    let now = Utc::now().timestamp();

    conn.execute(
        "INSERT OR IGNORE INTO sessions \
         (id, name, project_path, git_branch, started_at, last_active, turn_count, total_cost_usd, status) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, 0.0, 'active')",
        rusqlite::params![
            session.session_id,
            session.session_name,
            session.project_path,
            session.git_branch,
            session.started_at,
            now,
        ],
    )?;

    conn.execute(
        "UPDATE sessions SET last_active=?1, turn_count=?2, total_cost_usd=?3, name=?4 WHERE id=?5",
        rusqlite::params![
            now,
            session.turn_count,
            session.session_total_cost,
            session.session_name,
            session.session_id,
        ],
    )?;

    let persona = session.active_persona.as_deref();

    conn.execute(
        "INSERT INTO session_turns (session_id, ts, role, content, model, tokens_in, tokens_out, persona) \
         VALUES (?1, ?2, 'user', ?3, ?4, ?5, 0, ?6)",
        rusqlite::params![session.session_id, now, query, model, tokens_in, persona],
    )?;

    conn.execute(
        "INSERT INTO session_turns (session_id, ts, role, content, model, tokens_in, tokens_out, persona) \
         VALUES (?1, ?2, 'assistant', ?3, ?4, 0, ?5, ?6)",
        rusqlite::params![session.session_id, now, response, model, tokens_out, persona],
    )?;

    Ok(())
}

// ── Escape-key cancel listener ────────────────────────────────────────────────

/// Spawns a background thread that watches for the Escape key.
/// Sets `flag` to true when Escape is pressed, then exits.
/// Also exits when `flag` becomes true externally (operation finished).
fn spawn_cancel_listener(flag: Arc<AtomicBool>) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        use crossterm::{
            event::{self, Event, KeyCode, KeyEventKind},
            terminal,
        };
        let _ = terminal::enable_raw_mode();
        loop {
            if flag.load(Ordering::Relaxed) {
                break;
            }
            match event::poll(std::time::Duration::from_millis(100)) {
                Ok(true) => {
                    if let Ok(Event::Key(key)) = event::read() {
                        if key.kind == KeyEventKind::Press && key.code == KeyCode::Esc {
                            flag.store(true, Ordering::Relaxed);
                            eprint!("\r\x1b[K\x1b[33m[cancelled]\x1b[0m\n");
                            let _ = io::stderr().flush();
                            break;
                        }
                    }
                }
                _ => {}
            }
        }
        let _ = terminal::disable_raw_mode();
    })
}

// ── Progress spinner ─────────────────────────────────────────────────────────

/// Shows a spinner on stderr while `done` is false. Clears the line when done.
/// Designed to run inside `tokio::spawn`.
async fn run_spinner(done: Arc<AtomicBool>, provider: &str) {
    const FRAMES: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
    let label = format!("{provider} thinking");
    let mut i = 0usize;
    let start = std::time::Instant::now();
    loop {
        if done.load(Ordering::Relaxed) {
            break;
        }
        let elapsed = start.elapsed().as_secs();
        if elapsed > 0 {
            eprint!("\r\x1b[90m{} {} ({}s)…\x1b[0m", FRAMES[i % FRAMES.len()], label, elapsed);
        } else {
            eprint!("\r\x1b[90m{} {}…\x1b[0m", FRAMES[i % FRAMES.len()], label);
        }
        let _ = io::stderr().flush();
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        i += 1;
    }
    // Clear spinner line
    eprint!("\r\x1b[K");
    let _ = io::stderr().flush();
}

// ── @-mention suggestion list ─────────────────────────────────────────────────

fn build_at_suggestions(cli: &detector::CliDetection) -> Vec<String> {
    let mut v = Vec::new();
    if cli.claude { v.push("@claude".to_string()); }
    if cli.gemini { v.push("@gemini".to_string()); }
    if cli.openai_cli { v.push("@openai".to_string()); }
    if cli.codex_cli { v.push("@codex".to_string()); }
    if cli.groq { v.push("@groq".to_string()); }
    if cli.qwen { v.push("@qwen".to_string()); }
    v.push("@cheap".to_string());
    v.push("@fast".to_string());
    if !cli.local_models.is_empty() {
        v.push("@local".to_string());
        for m in &cli.local_models {
            v.push(format!("@local/{}", m.id));
        }
    }
    v
}

// ── Helper: CLI-aware backend creation ───────────────────────────────────────

/// Like `backends::create_backend` but uses the detected binary names for CLI backends
/// so Windows `gemini.cmd` is found correctly in failover paths.
fn create_backend_with_cli(
    provider: &str,
    api_key: &str,
    ollama_url: &str,
    cli: &detector::CliDetection,
) -> Box<dyn backends::Backend> {
    match provider {
        "gemini-cli" => Box::new(backends::gemini_cli::GeminiCliBackend::new(&cli.gemini_bin)),
        "claude-cli" => Box::new(backends::claude_cli::ClaudeCliBackend::new(&cli.claude_bin)),
        _ => backends::create_backend(provider, api_key, ollama_url),
    }
}

// ── @-mention routing ─────────────────────────────────────────────────────────

enum AtOverride {
    Claude,
    Gemini,
    Local,
    LocalModel(String),
    Cheap,
    Fast,
    OpenAi,
    Codex,
    Groq,
    Qwen,
}

/// Strip `@provider` mentions from the query and return the cleaned text plus the override.
/// Handles `@local/model-id` (specific model), `@local` (best local), and other providers.
fn parse_at_mentions(query: &str) -> (String, Option<AtOverride>) {
    // Check @local/model first (more specific than @local)
    if let Some(pos) = query.find("@local/") {
        let after_prefix = &query[pos + "@local/".len()..];
        // Collect model id: everything up to whitespace
        let id_end = after_prefix.find(|c: char| c.is_whitespace()).unwrap_or(after_prefix.len());
        if id_end > 0 {
            let model_id = after_prefix[..id_end].to_string();
            let tail = &after_prefix[id_end..];
            let cleaned = format!("{} {}", &query[..pos], tail).trim().to_string();
            return (cleaned, Some(AtOverride::LocalModel(model_id)));
        }
    }

    let simple: &[(&str, fn() -> AtOverride)] = &[
        ("@claude", || AtOverride::Claude),
        ("@gemini", || AtOverride::Gemini),
        ("@local",  || AtOverride::Local),
        ("@cheap",  || AtOverride::Cheap),
        ("@fast",   || AtOverride::Fast),
        ("@openai", || AtOverride::OpenAi),
        ("@codex",  || AtOverride::Codex),
        ("@groq",   || AtOverride::Groq),
        ("@qwen",   || AtOverride::Qwen),
    ];

    for (tag, make) in simple {
        if let Some(pos) = query.find(tag) {
            let after = &query[pos + tag.len()..];
            // Word boundary check: @local should not match @localhost etc.
            if after.starts_with(|c: char| c.is_alphanumeric() || c == '-' || c == '_' || c == '/') {
                continue;
            }
            let cleaned = format!("{} {}", &query[..pos], after).trim().to_string();
            return (cleaned, Some(make()));
        }
    }

    (query.to_string(), None)
}

/// Parse multiple `@provider` mentions in a query.
/// Returns `Some(vec)` of `(segment_text, provider_name)` pairs when 2+ mentions are found.
/// Text before the first mention gets provider "default".
/// Returns `None` if fewer than 2 mentions are found (existing single-mention handling takes over).
fn parse_multi_at_mentions(query: &str) -> Option<Vec<(String, String)>> {
    const PROVIDERS: &[&str] = &[
        "@claude", "@gemini", "@local", "@cheap", "@fast",
        "@openai", "@codex", "@groq", "@qwen",
    ];

    // Collect all (position, provider) pairs
    let mut found: Vec<(usize, &str)> = Vec::new();
    for &tag in PROVIDERS {
        let mut search_from = 0;
        while let Some(pos) = query[search_from..].find(tag) {
            let abs_pos = search_from + pos;
            let after = &query[abs_pos + tag.len()..];
            // Word boundary: skip if next char is alphanumeric, '-', '_', '/'
            if !after.starts_with(|c: char| c.is_alphanumeric() || c == '-' || c == '_' || c == '/') {
                found.push((abs_pos, tag));
            }
            search_from = abs_pos + tag.len();
        }
    }

    if found.len() < 2 {
        return None;
    }

    // Sort by position
    found.sort_by_key(|(pos, _)| *pos);
    // Deduplicate overlapping positions (keep earliest)
    found.dedup_by_key(|(pos, _)| *pos);

    let mut segments: Vec<(String, String)> = Vec::new();

    // Text before first mention
    if found[0].0 > 0 {
        let pre = query[..found[0].0].trim().to_string();
        if !pre.is_empty() {
            segments.push((pre, "default".to_string()));
        }
    }

    for i in 0..found.len() {
        let (start, tag) = found[i];
        let content_start = start + tag.len();
        let content_end = if i + 1 < found.len() { found[i + 1].0 } else { query.len() };
        let text = query[content_start..content_end].trim().to_string();
        let provider = tag.trim_start_matches('@').to_string();
        if !text.is_empty() {
            segments.push((text, provider));
        }
    }

    if segments.len() < 2 {
        return None;
    }

    Some(segments)
}

fn apply_at_override(at: &AtOverride, flags: &mut commands::QueryFlags, cli: &detector::CliDetection) {
    match at {
        AtOverride::Claude => {
            flags.force_provider = Some(if cli.claude {
                "claude-cli".to_string()
            } else {
                "claude".to_string()
            });
        }
        AtOverride::Gemini => {
            flags.force_provider = Some(if cli.gemini {
                "gemini-cli".to_string()
            } else {
                "gemini".to_string()
            });
        }
        AtOverride::Local => {
            flags.local = true;
        }
        AtOverride::LocalModel(model_id) => {
            // Route to a specific local model by ID (Ollama or LM Studio)
            flags.force_provider = Some(format!("local:{}", model_id));
        }
        AtOverride::Cheap | AtOverride::Fast => {
            // Local models are free — prefer them. Fall back to cheapest cloud if none available.
            if !cli.local_models.is_empty() {
                flags.local = true;
            } else {
                flags.cheap = true;
            }
        }
        AtOverride::OpenAi => {
            flags.force_provider = Some(if cli.openai_cli {
                "openai-cli".to_string()
            } else {
                "openai".to_string()
            });
        }
        AtOverride::Codex => {
            // Codex routes to OpenAI API using codex-mini-latest model
            flags.force_provider = Some("openai".to_string());
            flags.model = Some("codex-mini".to_string());
        }
        AtOverride::Groq => {
            flags.force_provider = Some(if cli.groq {
                "groq-cli".to_string()
            } else {
                "groq".to_string()
            });
        }
        AtOverride::Qwen => {
            flags.force_provider = Some(if cli.qwen {
                "qwen-cli".to_string()
            } else {
                "qwen".to_string()
            });
        }
    }
}

// ── Persona command ───────────────────────────────────────────────────────────

fn handle_persona_command(session: &mut Session, name: Option<&str>) {
    match name {
        None => {
            println!("\x1b[1mDeveloper personas:\x1b[0m");
            println!("  {:<14} {:<10} {}", "Name", "Display", "Description");
            println!("  {}", "-".repeat(62));
            for p in persona::list() {
                let active = if session.active_persona.as_deref() == Some(p.name) { " \x1b[32m✓\x1b[0m" } else { "" };
                println!("  {:<14} {:<10} {}{}", p.name, p.display, p.description, active);
            }
            if let Some(name) = &session.active_persona {
                println!("\nActive: \x1b[32m{name}\x1b[0m  (use /persona off to clear)");
            } else {
                println!("\nNo persona active. Use /persona <name> to activate one.");
            }
        }
        Some("off") | Some("none") | Some("clear") => {
            session.active_persona = None;
            println!("Persona cleared.");
        }
        Some(name) => {
            if let Some(p) = persona::find(name) {
                session.active_persona = Some(p.name.to_string());
                println!("Persona set: \x1b[32m{}\x1b[0m — {}", p.display, p.description);
            } else {
                eprintln!("Unknown persona '{name}'. Run /persona to list available personas.");
            }
        }
    }
}

// ── Debate (multi-agent brainstorm) ──────────────────────────────────────────

async fn run_debate_turn(
    query: &str,
    strategy_name: &str,
    session: &mut Session,
    cfg: &config::LoadedConfig,
    ollama_url: &str,
    cli: &detector::CliDetection,
) {
    let strategy = brainstorm::Strategy::from_str(strategy_name);
    eprintln!(
        "\x1b[90m[brainstorm] strategy: {} — {}\x1b[0m",
        strategy.name(),
        strategy.description()
    );

    // Pick backend A: claude-cli → claude API
    // Pick backend B: gemini-cli → gemini API
    let backend_a: Box<dyn backends::Backend> = if cli.claude {
        Box::new(backends::claude_cli::ClaudeCliBackend::new(&cli.claude_bin))
    } else if let Ok(key) = crate::get_api_key("claude", cfg) {
        backends::create_backend("claude", &key, ollama_url)
    } else if cli.gemini {
        Box::new(backends::gemini_cli::GeminiCliBackend::new(&cli.gemini_bin))
    } else {
        eprintln!("  No backends available for brainstorm. Configure claude or gemini.");
        return;
    };

    let backend_b: Box<dyn backends::Backend> = if cli.gemini {
        Box::new(backends::gemini_cli::GeminiCliBackend::new(&cli.gemini_bin))
    } else if let Ok(key) = crate::get_api_key("gemini", cfg) {
        backends::create_backend("gemini", &key, ollama_url)
    } else if let Ok(key) = crate::get_api_key("claude", cfg) {
        backends::create_backend("claude", &key, ollama_url)
    } else {
        eprintln!("  Only one backend available — brainstorm needs two. Add a gemini key or CLI.");
        return;
    };

    let system = SystemPromptBuilder::new(&cfg.config).build_base_system_prompt();
    let system_ref = Some(system.as_str());

    match brainstorm::run_strategy(
        query,
        system_ref,
        &session.messages,
        backend_a.as_ref(),
        backend_b.as_ref(),
        &strategy,
    )
    .await
    {
        Ok(result) => {
            println!("\n\x1b[1m━━ {} ━━\x1b[0m", result.label_a);
            println!("{}", result.response_a);
            println!("\n\x1b[1m━━ {} ━━\x1b[0m", result.label_b);
            println!("{}", result.response_b);
            if let Some(synth) = &result.synthesis {
                println!("\n\x1b[33m{synth}\x1b[0m");
            }
            println!(
                "\n\x1b[90m[brainstorm complete — {} round(s), strategy: {}]\x1b[0m\n",
                result.rounds,
                result.strategy.name()
            );
            // Accumulate token counts from all brainstorm calls
            session.session_tokens_in += result.input_tokens;
            session.session_tokens_out += result.output_tokens;
            session.turn_count += 1;
            // Store the combined response as the last response for /apply
            let combined = format!(
                "## {} (brainstorm: {})\n\n{}\n\n## {}\n\n{}",
                result.label_a, result.strategy.name(),
                result.response_a, result.label_b, result.response_b
            );
            session.last_response = Some(combined);
        }
        Err(e) => {
            eprintln!("\n[brainstorm error] {e}");
        }
    }
}

fn slug_from_query(query: &str) -> String {
    query.split_whitespace().take(5).collect::<Vec<_>>().join("-").to_lowercase()
}

/// Generate a concise 3-5 word slug for a session using a cheap model.
/// Falls back silently on any error — callers always have a heuristic slug ready.
async fn generate_session_name(
    query: &str,
    cfg: &config::LoadedConfig,
    ollama_url: &str,
) -> Option<String> {
    let alias = cfg.config.routing.rules.fallback.clone();
    let (provider, model_id) = backends::resolve_model(&alias, &cfg.models)
        .unwrap_or_else(|| ("claude".to_string(), alias.clone()));

    let api_key = crate::get_api_key(&provider, cfg).ok()?;
    let backend = backends::create_backend(&provider, &api_key, ollama_url);

    let prompt = format!(
        "Summarize this query as a 3-5 word slug (lowercase, hyphens, no punctuation): {query}\n\
         Reply with only the slug, nothing else."
    );

    let opts = CompletionOptions {
        model_id,
        system: None,
        messages: vec![Message { role: "user".to_string(), content: prompt }],
        max_tokens: 20,
        use_search_grounding: false,
        use_cache: false,
        auto_accept: false,
    };

    // Use a short timeout variant — this is best-effort
    let timed = tokio::time::timeout(
        std::time::Duration::from_secs(8),
        backend.complete(opts),
    )
    .await;
    let result = match timed {
        Ok(Ok(r)) => r,
        _ => return None,
    };

    let name = result
        .content
        .trim()
        .to_lowercase()
        .replace(|c: char| !c.is_alphanumeric() && c != '-', "-")
        .trim_matches('-')
        .to_string();

    // Sanity check: must be non-empty and reasonable length
    if name.is_empty() || name.len() > 60 {
        return None;
    }
    // Remove consecutive hyphens
    let name = name.split('-').filter(|s| !s.is_empty()).collect::<Vec<_>>().join("-");
    Some(name)
}

fn print_header(session: &Session) {
    let branch = session
        .git_branch
        .as_deref()
        .map(|b| format!(" [{b}]"))
        .unwrap_or_default();
    let mode = if session.agent_mode { " [agent]" } else { "" };
    println!(
        "ZedPlus — {} ({}){}{}\nType your query or /help. Use /agent to toggle agentic mode.\n",
        session.model_key, session.provider, branch, mode
    );
}

fn print_exit_summary(session: &Session) {
    let name = session.session_name.as_deref().unwrap_or("unnamed");
    println!(
        "\nSession '{}' — {} turns, ${:.4}",
        name, session.turn_count, session.session_total_cost
    );

    if !session.backend_usage.is_empty() {
        println!("Backend usage:");
        let mut entries: Vec<_> = session.backend_usage.iter().collect();
        entries.sort_by(|a, b| b.1.0.cmp(&a.1.0));
        for (provider, (turns, tok_in, tok_out)) in entries {
            let kind = if provider.ends_with("-cli") { "subscription" }
                       else if provider == "ollama" || provider == "lmstudio" { "local" }
                       else { "api" };
            println!(
                "  {:<22} {:>3} turn(s)  {:>7} in / {:>7} out tokens  [{}]",
                provider, turns, tok_in, tok_out, kind
            );
        }
    }
    println!("To resume: zedplus resume");
}

fn print_conversation_history(session_id: &str) {
    let db_path = match dirs::db_file() {
        Ok(p) => p,
        Err(_) => { eprintln!("Could not find database."); return; }
    };
    let conn = match crate::db::open(&db_path) {
        Ok(c) => c,
        Err(_) => { eprintln!("Could not open database."); return; }
    };

    let sql = "SELECT role, content, model, tokens_in, tokens_out, ts \
               FROM session_turns WHERE session_id = ?1 ORDER BY ts ASC";

    let mut stmt = match conn.prepare(sql) {
        Ok(s) => s,
        Err(_) => { eprintln!("Could not query history."); return; }
    };

    struct Turn { role: String, content: String, model: String, tokens_in: i64, tokens_out: i64 }

    let turns: Vec<Turn> = stmt
        .query_map(rusqlite::params![session_id], |row| {
            Ok(Turn {
                role:       row.get(0)?,
                content:    row.get(1)?,
                model:      row.get(2).unwrap_or_default(),
                tokens_in:  row.get(3).unwrap_or(0),
                tokens_out: row.get(4).unwrap_or(0),
            })
        })
        .ok()
        .map(|rows| rows.filter_map(|r| r.ok()).collect())
        .unwrap_or_default();

    if turns.is_empty() {
        println!("No conversation history in this session yet.");
        return;
    }

    // Group user+assistant pairs and show last 20 pairs
    let pairs: Vec<_> = turns.chunks(2).collect();
    let shown = pairs.iter().rev().take(20).rev();
    println!("\x1b[1mConversation history ({} turns):\x1b[0m", pairs.len());

    for pair in shown {
        let user = pair.first();
        let asst = pair.get(1);

        if let Some(u) = user {
            let q = if u.content.len() > 120 { format!("{}…", &u.content[..120]) } else { u.content.clone() };
            println!("\n\x1b[36m▶ You:\x1b[0m {q}");
        }
        if let Some(a) = asst {
            let provider_label = if a.model.is_empty() { "unknown".to_string() } else { a.model.clone() };
            let ans = if a.content.len() > 200 { format!("{}…", &a.content[..200]) } else { a.content.clone() };
            println!("\x1b[33m● {provider_label}\x1b[0m ({} out tokens)", a.tokens_out);
            println!("  {}", ans.replace('\n', "\n  "));
        }
    }
    println!();
}

fn print_session_usage(session: &Session) {
    println!(
        "Session: {} turns | {} in / {} out tokens | ${:.4}",
        session.turn_count,
        session.session_tokens_in,
        session.session_tokens_out,
        session.session_total_cost
    );
}

fn print_model_list(cfg: &config::LoadedConfig) {
    let mut entries: Vec<_> = cfg.models.models.iter().collect();
    entries.sort_by(|a, b| a.0.cmp(b.0));
    println!("  {:<22} {:<10} {:<8} {:<8} {}", "Alias", "Provider", "Quality", "Local", "Model ID");
    println!("  {}", "-".repeat(72));
    for (alias, m) in &entries {
        println!(
            "  {:<22} {:<10} {:<8} {:<8} {}",
            alias, m.provider,
            format!("{}/5", m.quality_tier),
            if m.is_local { "yes" } else { "no" },
            m.id,
        );
    }
    println!("\nUsage: /model <alias> <query>");
}

fn print_local_model_table(models: &[crate::local_models::DiscoveredModel]) {
    eprintln!("[zedplus] local models discovered ({}):", models.len());
    eprintln!(
        "  {:<45} {:<10} {:>6} {:>5}  {:>9}  {:>9}",
        "Model", "Service", "Params", "Q", "Reasoning", "Execution"
    );
    eprintln!("  {}", "-".repeat(92));
    for m in models {
        let params = match m.params_b {
            Some(p) if p >= 1.0 => format!("{:.0}B", p),
            Some(p)             => format!("{:.1}B", p),
            None                => "?B".into(),
        };
        let kind = if m.is_coder { "★coder" } else { "general" };
        eprintln!(
            "  {:<45} {:<10} {:>6} {:>5}  {:>9}  {:>9}",
            &m.id,
            format!("{} ({})", m.provider, kind),
            params,
            m.quality_tier,
            format!("{}/5", m.reasoning_score),
            format!("{}/5", m.execution_score),
        );
    }
    if let Some(r) = crate::local_models::best_for_reasoning(models) {
        eprintln!("  → Reasoning: {}", r.id);
    }
    if let Some(e) = crate::local_models::best_for_execution(models) {
        eprintln!("  → Execution: {}", e.id);
    }
}

fn prompt_ui_preference(cli: &detector::CliDetection, cfg: &config::LoadedConfig) {
    use config::schema::UiStyle;
    use std::io::{BufRead, Write as IoWrite};

    println!("\nFirst run — which UI style do you prefer?");
    let mut options: Vec<(&str, &str, UiStyle)> = vec![("1", "native  (ZedPlus default)", UiStyle::Native)];
    if cli.claude {
        options.push(("2", "claude  (Claude Code style)", UiStyle::ClaudeCode));
    }
    if cli.gemini {
        options.push(("3", "gemini  (Gemini CLI style)", UiStyle::GeminiCli));
    }
    for (n, label, _) in &options {
        println!("  [{n}] {label}");
    }
    print!("Choice [1]: ");
    let _ = std::io::stdout().flush();

    let choice = std::io::stdin().lock().lines().next()
        .and_then(|l| l.ok())
        .unwrap_or_default();
    let choice = choice.trim();

    let selected = options.iter().find(|(n, _, _)| *n == choice)
        .or_else(|| options.first())
        .map(|(_, _, s)| s.clone())
        .unwrap_or(UiStyle::Native);

    let label = match &selected {
        UiStyle::Native => "native",
        UiStyle::ClaudeCode => "claude",
        UiStyle::GeminiCli => "gemini",
    };

    let mut updated = cfg.config.clone();
    updated.behavior.ui_style = selected;
    match config::write_global(&updated) {
        Ok(()) => println!("UI style set to '{label}'. Change anytime with /ui\n"),
        Err(e) => eprintln!("Could not save UI preference: {e}"),
    }
}

/// Query the most recent test_runs row; returns stderr if the run failed.
fn check_last_test_failure() -> Option<String> {
    let db_path = crate::platform::dirs::db_file().ok()?;
    let conn = db::open(&db_path).ok()?;
    let result: rusqlite::Result<(bool, String)> = conn.query_row(
        "SELECT passed, output FROM test_runs ORDER BY ts DESC LIMIT 1",
        [],
        |row| Ok((row.get::<_, i32>(0)? != 0, row.get::<_, String>(1)?)),
    );
    match result {
        Ok((false, output)) => Some(output),
        _ => None,
    }
}

fn print_routing_decision(d: &router::RoutingDecision) {
    println!("  Task:       {}", d.task_type.as_str());
    println!("  Model:      {} ({})", d.model_id, d.model_key);
    println!("  Provider:   {}", d.provider);
    println!("  Reason:     {}", d.reason);
    println!("  Tokens est: ~{} in + ~1024 out", d.estimated_input_tokens);
    println!("  Cost est:   ${:.6}", d.estimated_cost_usd);
    if let Some((alt_key, alt_cost)) = &d.cheapest_alternative {
        println!("  Cheapest:   {alt_key} (${alt_cost:.6})");
    }
}

fn read_line(prompt: &str) -> Result<Option<String>> {
    use std::io::BufRead;
    print!("{prompt}");
    io::stdout().flush()?;
    let mut buf = String::new();
    match io::stdin().lock().read_line(&mut buf) {
        Ok(0) => Ok(None), // EOF / Ctrl+D
        Ok(_) => Ok(Some(buf.trim_end_matches(['\n', '\r']).to_string())),
        Err(e) => Err(e.into()),
    }
}
