mod agent;
mod apply;
mod backends;
mod brainstorm;
mod cli;
mod config;
mod context;
mod db;
mod distiller;
mod hooks;
mod indexer;
mod local_models;
mod persona;
mod pipeline;
mod platform;
mod repl;
mod router;
mod sessions;
mod setup;
mod shell;
mod skills;
mod tester;

use anyhow::Result;
use clap::Parser;
use cli::{Cli, Command};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_target(false)
        .without_time()
        .init();

    platform::dirs::ensure_dirs()?;

    let cli = Cli::parse();

    match cli.command {
        None => {
            repl::run(cli.query).await?;
        }

        Some(Command::Init(args)) => {
            if args.context {
                let cwd = std::env::current_dir()?;
                let path = context::project::generate(&cwd)?;
                println!("Generated: {}", path.display());
                println!("This file gives ZedPlus richer context about your project.");
                println!("Commit it to your repo or add it to .gitignore.");
            } else {
                setup::run_init(false).await?;
            }
        }

        Some(Command::Auth(args)) => {
            if let Some(provider) = args.revoke {
                cmd_auth_revoke(&provider)?;
            } else {
                setup::run_auth(args.provider).await?;
            }
        }

        Some(Command::Index(args)) => {
            let path = args.path.unwrap_or_else(|| std::env::current_dir().unwrap());
            indexer::run(path, args.reset).await?;
        }

        Some(Command::Ask(args)) => {
            cmd_ask(args).await?;
        }

        Some(Command::Search(args)) => {
            cmd_search(args).await?;
        }

        Some(Command::Resume) => {
            cmd_resume().await?;
        }

        Some(Command::Clear) => {
            println!("Session context cleared. Distillation data preserved.");
        }

        Some(Command::Usage(args)) => {
            cmd_usage(args)?;
        }

        Some(Command::Distill(args)) => {
            cmd_distill(args)?;
        }

        Some(Command::Train(args)) => {
            cmd_train(args).await?;
        }

        Some(Command::Bench(args)) => {
            cmd_bench(args).await?;
        }

        Some(Command::Model(args)) => {
            cmd_model(args)?;
        }

        Some(Command::Profile(args)) => {
            cmd_profile(args)?;
        }

        Some(Command::Config(args)) => {
            cmd_config(args)?;
        }

        Some(Command::Update(args)) => {
            cmd_update(args).await?;
        }

        Some(Command::Shell(args)) => {
            if args.install_hotkey {
                shell::integration::install_hotkey_interactive()?;
            } else if let Some(desc) = &args.description {
                shell::run(desc, args.inline).await?;
            } else {
                eprintln!("Usage: zedplus shell \"<description>\"  or  zedplus shell --install-hotkey");
            }
        }

        Some(Command::Session(args)) => {
            cmd_session(args)?;
        }

        Some(Command::Skills(args)) => {
            cmd_skills(args)?;
        }
    }

    Ok(())
}

// ── zedplus ask ──────────────────────────────────────────────────────────────

async fn cmd_ask(args: cli::AskArgs) -> Result<()> {
    use std::io::Write as IoWrite;

    let cwd = std::env::current_dir()?;
    let mut cfg = config::load(Some(&cwd))?;

    let ollama_url = cfg
        .config
        .services
        .ollama_url
        .as_deref()
        .unwrap_or("http://localhost:11434");

    // If local mode is requested or cloud is disabled, run a quick discovery
    // to sync the registry aliases with what's actually running.
    if args.local || cfg.config.privacy.cloud_allowed == Some(false) {
        let lmstudio_url = cfg.config.services.lmstudio_url
            .as_deref()
            .unwrap_or("http://localhost:1234");
        let discovered = crate::local_models::discover(ollama_url, lmstudio_url).await;
        if !discovered.is_empty() {
            crate::local_models::update_registry_with_discovered(&mut cfg.models, &discovered);
        }
    }

    let decision = router::route(
        &args.query,
        &cfg.config,
        &cfg.models,
        &cfg.costs,
        args.model.as_deref(),
        args.local,
        args.cheap,
    );

    // Privacy gate
    let is_local = cfg.models.get(&decision.model_key).map(|m| m.is_local).unwrap_or(false);
    if !is_local {
        if let Some(false) = cfg.config.privacy.cloud_allowed {
            anyhow::bail!(
                "Cloud requests are disabled (privacy.cloud_allowed = false).\n\
                 Use --local or enable cloud in .zedplus.toml."
            );
        }
    }

    if args.explain {
        println!("  Task:       {}", decision.task_type.as_str());
        println!("  Model:      {} ({})", decision.model_id, decision.model_key);
        println!("  Provider:   {}", decision.provider);
        println!("  Reason:     {}", decision.reason);
        println!("  Architect:  {}", if decision.is_architect_mode { "YES" } else { "NO" });
        println!("  Tokens est: ~{} in + ~1024 out", decision.estimated_input_tokens);
        println!("  Cost est:   ${:.6}", decision.estimated_cost_usd);
        if let Some((alt_key, alt_cost)) = &decision.cheapest_alternative {
            println!("  Cheapest:   {alt_key} (${alt_cost:.6})");
        }
        return Ok(());
    }

    let api_key = get_api_key(&decision.provider, &cfg)?;
    let ollama_url = cfg
        .config
        .services
        .ollama_url
        .as_deref()
        .unwrap_or("http://localhost:11434");

    let backend = backends::create_backend(&decision.provider, &api_key, ollama_url);
    let system_prompt = context::SystemPromptBuilder::new(&cfg.config).build_base_system_prompt();

    // ── Architect/Editor Split Mode ───────────────────────────────────────────
    if decision.is_architect_mode && !args.agent {
        println!("\x1b[35m[architect] Planning implementation...\x1b[0m");
        let db_path = platform::dirs::db_file()?;
        let conn = db::open(&db_path)?;
        let index_store = indexer::store::IndexStore::new(conn);
        let repomap = index_store.build_repomap().unwrap_or_default();
        
        let embedder = indexer::embedder::Embedder::new(ollama_url);
        let chunks = if let Ok(emb) = embedder.embed(&args.query).await {
            index_store.similarity_search(&emb, 5).unwrap_or_default()
        } else { vec![] };
        
        let chunk_text: String = chunks.iter().map(|c| format!("File: {}\n{}", c.file_path, c.content)).collect::<Vec<_>>().join("\n\n");

        let plan = pipeline::architect::run_planning_phase(
            &args.query,
            &repomap,
            &chunk_text,
            backend.as_ref(),
            &decision.model_id
        ).await?;

        println!("\x1b[35m[editor] Applying changes...\x1b[0m");
        let editor_alias = router::rules::select_editor_alias(&cfg.config, &cfg.models);
        let (editor_prov, editor_mid) = backends::resolve_model(&editor_alias, &cfg.models)
            .or_else(|| backends::resolve_model("local", &cfg.models))
            .unwrap_or(("ollama".to_string(), "llama3.2:8b".to_string()));
        let editor_key = get_api_key(&editor_prov, &cfg).unwrap_or_default();
        let editor_backend = backends::create_backend(&editor_prov, &editor_key, ollama_url);

        let mut file_contents = Vec::new();
        for path in &plan.files_to_modify {
            if let Ok(c) = std::fs::read_to_string(cwd.join(path)) {
                file_contents.push((path.clone(), c));
            }
        }

        let diff = pipeline::architect::run_editing_phase(
            &plan,
            &file_contents,
            editor_backend.as_ref(),
            &editor_mid
        ).await?;

        println!();
        println!("{diff}");
        
        if args.apply {
            let _ = apply::apply_response(&diff, &cwd);
        }

        let _ = distiller::record(distiller::DistillEntry {
            query: args.query.clone(),
            response: diff,
            model_key: decision.model_key.clone(),
            model_id: decision.model_id.clone(),
            task_type: decision.task_type.as_str().to_string(),
            input_tokens: 0, // Simplified for this release
            output_tokens: 0,
            cost_usd: 0.0,
            cache_hit: false,
            override_model: args.model.clone(),
            is_architect_split: true,
            reward_signal: 0.0,
            edit_accepted: false,
            session_id: None,
            });
        return Ok(());
    }

    // ── Agent mode ─────────────────────────────────────────────────────────────
    if args.agent {
        match agent::run(
            &args.query,
            Some(&system_prompt),
            &[],
            backend.as_ref(),
            &decision.model_id,
            4096,
            20,
            &cwd,
            args.yes,
            ollama_url,
            std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        )
        .await
        {
            Ok((final_text, tokens_in, tokens_out, pending_reward)) => {
                println!();
                if args.apply {
                    let _ = apply::apply_response(&final_text, &cwd);
                }
                let actual_cost =
                    cfg.costs.cost_usd(&decision.model_id, tokens_in, tokens_out);
                let ask_session_id = format!("ask-{:x}", chrono::Utc::now().timestamp_millis());
                if let Ok(row_id) = distiller::record(distiller::DistillEntry {
                    query: args.query.clone(),
                    response: final_text,
                    model_key: decision.model_key.clone(),
                    model_id: decision.model_id.clone(),
                    task_type: decision.task_type.as_str().to_string(),
                    input_tokens: tokens_in,
                    output_tokens: tokens_out,
                    cost_usd: actual_cost,
                    cache_hit: false,
                    override_model: args.model.clone(),
                    is_architect_split: false,
                    reward_signal: 0.0,
                    edit_accepted: false,
                    session_id: Some(ask_session_id),
                }) {
                    if pending_reward != 0.0 {
                        let _ = distiller::update_reward(row_id, pending_reward);
                    }
                }
            }
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("Rate limit") || msg.contains("rate limit") {
                    if let Some((fl_alias, fl_prov, fl_mid)) = failover_provider(&decision.provider, &cfg) {
                        eprintln!("\x1b[33m⚠ {} rate limit — retrying with {fl_alias}\x1b[0m", decision.provider);
                        let fl_key = get_api_key(&fl_prov, &cfg).unwrap_or_default();
                        let fl_backend = backends::create_backend(&fl_prov, &fl_key, ollama_url);
                        match agent::run(&args.query, Some(&system_prompt), &[], fl_backend.as_ref(), &fl_mid, 4096, 20, &cwd, args.yes, ollama_url, std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false))).await {
                            Ok((t, _, _, _)) => { println!(); println!("{t}"); }
                            Err(e2) => eprintln!("Error after failover: {e2}"),
                        }
                    } else {
                        eprintln!("\x1b[31m⚠ Rate limit reached.\x1b[0m");
                        eprintln!("  {}", rate_limit_upgrade_url(&decision.provider));
                    }
                } else {
                    return Err(e);
                }
            }
        }
        return Ok(());
    }

    let opts = backends::CompletionOptions {
        model_id: decision.model_id.clone(),
        system: Some(system_prompt),
        messages: vec![backends::Message {
            role: "user".to_string(),
            content: args.query.clone(),
        }],
        max_tokens: 4096,
        use_search_grounding: decision.use_search_grounding,
        use_cache: decision.use_cache,
        auto_accept: false,
    };

    // Non-terminal output formats require collecting the full response before printing
    let force_no_stream = matches!(args.output.as_str(), "json" | "plain");
    let stream = !args.no_stream && !force_no_stream;

    let result = if stream {
        backend
            .complete_streaming(
                opts,
                Box::new(|token: String| {
                    print!("{token}");
                    let _ = std::io::stdout().flush();
                }),
            )
            .await
    } else {
        backend.complete(opts).await
    };

    match result {
        Ok(r) => {
            let actual_cost =
                cfg.costs.cost_usd(&decision.model_id, r.input_tokens, r.output_tokens);

            match args.output.as_str() {
                "json" => {
                    let json = serde_json::json!({
                        "query":        args.query,
                        "response":     r.content,
                        "model_key":    decision.model_key,
                        "model_id":     decision.model_id,
                        "task_type":    decision.task_type.as_str(),
                        "input_tokens": r.input_tokens,
                        "output_tokens":r.output_tokens,
                        "cost_usd":     actual_cost,
                        "cache_hit":    r.cache_hit,
                    });
                    println!("{json}");
                }
                "plain" => println!("{}", r.content),
                _ => {
                    // terminal: already streamed or not yet printed
                    if !stream { println!("{}", r.content); }
                    else { println!(); }
                }
            }

            if args.apply {
                let cwd = std::env::current_dir()?;
                let _ = apply::apply_response(&r.content, &cwd);
            }

            // --exit-code: exit 1 when response signals an error or warning
            if args.exit_code {
                let lower = r.content.to_lowercase();
                if lower.contains("error") || lower.contains("warning") || lower.contains("failed") {
                    let _ = distiller::record(distiller::DistillEntry {
                        query: args.query.clone(),
                        response: r.content,
                        model_key: decision.model_key.clone(),
                        model_id: decision.model_id.clone(),
                        task_type: decision.task_type.as_str().to_string(),
                        input_tokens: r.input_tokens,
                        output_tokens: r.output_tokens,
                        cost_usd: actual_cost,
                        cache_hit: r.cache_hit,
                        override_model: args.model.clone(),
                        is_architect_split: false,
                        reward_signal: 0.0,
                        edit_accepted: false,
                        session_id: Some(format!("ask-{:x}", chrono::Utc::now().timestamp_millis())),
                    });
                    std::process::exit(1);
                }
            }

            let _ = distiller::record(distiller::DistillEntry {
                query: args.query.clone(),
                response: r.content,
                model_key: decision.model_key.clone(),
                model_id: decision.model_id.clone(),
                task_type: decision.task_type.as_str().to_string(),
                input_tokens: r.input_tokens,
                output_tokens: r.output_tokens,
                cost_usd: actual_cost,
                cache_hit: r.cache_hit,
                override_model: args.model.clone(),
                is_architect_split: false,
                reward_signal: 0.0,
                edit_accepted: false,
                session_id: Some(format!("ask-{:x}", chrono::Utc::now().timestamp_millis())),
            });
        }
        Err(backends::BackendError::RateLimit) => {
            eprintln!("⚠ Rate limit hit — retrying with fallback model...");
            let fallback = cfg.config.routing.rules.fallback.clone();
            cmd_ask_with_fallback(args, &fallback, &cfg).await?;
        }
        Err(backends::BackendError::Timeout) => {
            eprintln!("⚠ Request timed out — retrying with fallback model...");
            let fallback = cfg.config.routing.fallback_chain.local_failure.clone();
            cmd_ask_with_fallback(args, &fallback, &cfg).await?;
        }
        Err(e) => return Err(e.into()),
    }

    Ok(())
}

async fn cmd_ask_with_fallback(
    args: cli::AskArgs,
    fallback_alias: &str,
    cfg: &config::LoadedConfig,
) -> Result<()> {
    use std::io::Write as IoWrite;

    let (provider, model_id) = backends::resolve_model(fallback_alias, &cfg.models)
        .unwrap_or_else(|| ("claude".to_string(), fallback_alias.to_string()));

    let api_key = get_api_key(&provider, cfg)?;
    let ollama_url = cfg
        .config
        .services
        .ollama_url
        .as_deref()
        .unwrap_or("http://localhost:11434");

    let backend = backends::create_backend(&provider, &api_key, ollama_url);
    let system_prompt = context::SystemPromptBuilder::new(&cfg.config).build_base_system_prompt();

    let opts = backends::CompletionOptions {
        model_id: model_id.clone(),
        system: Some(system_prompt),
        messages: vec![backends::Message {
            role: "user".to_string(),
            content: args.query.clone(),
        }],
        max_tokens: 4096,
        use_search_grounding: false,
        use_cache: false,
        auto_accept: false,
    };

    let stream = !args.no_stream;
    let result = if stream {
        backend
            .complete_streaming(
                opts,
                Box::new(|token: String| {
                    print!("{token}");
                    let _ = std::io::stdout().flush();
                }),
            )
            .await
    } else {
        backend.complete(opts).await
    };

    match result {
        Ok(r) => {
            if stream { println!(); } else { println!("{}", r.content); }
            let actual_cost = cfg.costs.cost_usd(&model_id, r.input_tokens, r.output_tokens);
            let _ = distiller::record(distiller::DistillEntry {
                query: args.query.clone(),
                response: r.content,
                model_key: fallback_alias.to_string(),
                model_id,
                task_type: "fallback".to_string(),
                input_tokens: r.input_tokens,
                output_tokens: r.output_tokens,
                cost_usd: actual_cost,
                cache_hit: r.cache_hit,
                override_model: args.model.clone(),
                is_architect_split: false,
                reward_signal: 0.0,
                edit_accepted: false,
                session_id: Some(format!("ask-{:x}", chrono::Utc::now().timestamp_millis())),
            });
        }
        Err(e) => return Err(e.into()),
    }
    Ok(())
}

// ── zedplus search ───────────────────────────────────────────────────────────

async fn cmd_search(args: cli::SearchArgs) -> Result<()> {
    use std::io::Write as IoWrite;

    let cwd = std::env::current_dir()?;
    let cfg = config::load(Some(&cwd))?;

    let (provider, model_id) = backends::resolve_model("gemini-flash", &cfg.models)
        .unwrap_or_else(|| ("gemini".to_string(), "gemini-2.5-flash".to_string()));

    let api_key = get_api_key(&provider, &cfg)?;
    let ollama_url = cfg
        .config
        .services
        .ollama_url
        .as_deref()
        .unwrap_or("http://localhost:11434");

    let backend = backends::create_backend(&provider, &api_key, ollama_url);
    let system_prompt = context::SystemPromptBuilder::new(&cfg.config).build_base_system_prompt();

    let opts = backends::CompletionOptions {
        model_id: model_id.clone(),
        system: Some(system_prompt),
        messages: vec![backends::Message {
            role: "user".to_string(),
            content: args.query.clone(),
        }],
        max_tokens: 4096,
        use_search_grounding: true,
        use_cache: false,
        auto_accept: false,
    };

    let stream = !args.no_stream;
    let result = if stream {
        backend
            .complete_streaming(
                opts,
                Box::new(|token: String| {
                    print!("{token}");
                    let _ = std::io::stdout().flush();
                }),
            )
            .await
    } else {
        backend.complete(opts).await
    };

    match result {
        Ok(r) => {
            if stream { println!(); } else { println!("{}", r.content); }
            let actual_cost = cfg.costs.cost_usd(&model_id, r.input_tokens, r.output_tokens);
            let _ = distiller::record(distiller::DistillEntry {
                query: args.query.clone(),
                response: r.content,
                model_key: "gemini-flash".to_string(),
                model_id,
                task_type: "web_search".to_string(),
                input_tokens: r.input_tokens,
                output_tokens: r.output_tokens,
                cost_usd: actual_cost,
                cache_hit: r.cache_hit,
                override_model: None,
                is_architect_split: false,
                reward_signal: 0.0,
                edit_accepted: false,
                session_id: None,
                });
        }
        Err(e) => return Err(e.into()),
    }
    Ok(())
}

// ── zedplus resume ───────────────────────────────────────────────────────────

async fn cmd_resume() -> Result<()> {
    let db_path = platform::dirs::db_file()?;
    if !db_path.exists() {
        println!("No sessions found. Start one with `zedplus`.");
        return Ok(());
    }

    let cwd = std::env::current_dir()?;
    let cwd_str = cwd.to_string_lossy().to_string();
    let branch = indexer::git::current_branch(&cwd);
    let conn = db::open(&db_path)?;

    // Show sessions from the last 7 days (wider window than auto-resume threshold)
    let since = chrono::Utc::now().timestamp() - 7 * 24 * 3600;
    let candidates = sessions::find_resumable(&conn, &cwd_str, branch.as_deref(), since, 10);

    let chosen = match sessions::offer_resume_prompt(candidates)? {
        Some(s) => s,
        None => {
            println!("No session selected. Starting fresh — run `zedplus` to open the REPL.");
            return Ok(());
        }
    };

    let turns = sessions::load_turns(&conn, &chosen.id);
    drop(conn);

    repl::run_resumed(
        chosen.id.clone(),
        chosen.name.clone(),
        chosen.git_branch.clone(),
        chosen.turn_count as u32,
        chosen.total_cost,
        turns,
    )
    .await
}

// ── zedplus usage ────────────────────────────────────────────────────────────

fn cmd_usage(args: cli::UsageArgs) -> Result<()> {
    let db_path = platform::dirs::db_file()?;
    if !db_path.exists() {
        println!("No usage data yet. Run some queries first.");
        return Ok(());
    }
    let conn = db::open(&db_path)?;

    if args.today {
        let start_of_day = chrono::Utc::now()
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp();

        let mut stmt = conn.prepare(
            "SELECT model, COUNT(*), SUM(input_tokens), SUM(output_tokens), SUM(cost_usd) \
             FROM usage WHERE ts >= ?1 GROUP BY model ORDER BY SUM(cost_usd) DESC",
        )?;
        print_usage_table(&mut stmt, rusqlite::params![start_of_day], "Today's usage")?;
    } else if args.month {
        let start_of_month = {
            use chrono::Datelike;
            let now = chrono::Utc::now();
            chrono::NaiveDate::from_ymd_opt(now.year(), now.month(), 1)
                .and_then(|d| d.and_hms_opt(0, 0, 0))
                .map(|dt| dt.and_utc().timestamp())
                .unwrap_or(0)
        };
        let mut stmt = conn.prepare(
            "SELECT model, COUNT(*), SUM(input_tokens), SUM(output_tokens), SUM(cost_usd) \
             FROM usage WHERE ts >= ?1 GROUP BY model ORDER BY SUM(cost_usd) DESC",
        )?;
        print_usage_table(&mut stmt, rusqlite::params![start_of_month], "This month's usage")?;
    } else {
        // Overall summary
        let count: i64 =
            conn.query_row("SELECT COUNT(*) FROM usage", [], |r| r.get(0))?;
        let total_cost: f64 = conn
            .query_row("SELECT COALESCE(SUM(cost_usd), 0) FROM usage", [], |r| r.get(0))?;
        let total_in: i64 = conn
            .query_row("SELECT COALESCE(SUM(input_tokens), 0) FROM usage", [], |r| r.get(0))?;
        let total_out: i64 = conn
            .query_row("SELECT COALESCE(SUM(output_tokens), 0) FROM usage", [], |r| r.get(0))?;

        println!("{:<20} {:>10} {:>14} {:>15} {:>12}", "Model", "Queries", "Tokens In", "Tokens Out", "Cost USD");
        println!("{}", "-".repeat(73));

        let mut stmt = conn.prepare(
            "SELECT model, COUNT(*), SUM(input_tokens), SUM(output_tokens), SUM(cost_usd) \
             FROM usage GROUP BY model ORDER BY SUM(cost_usd) DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, f64>(4)?,
            ))
        })?;
        for row in rows.filter_map(|r| r.ok()) {
            println!(
                "{:<20} {:>10} {:>14} {:>15} {:>12}",
                row.0, row.1, row.2, row.3, format!("${:.4}", row.4)
            );
        }
        println!("{}", "-".repeat(73));
        println!(
            "{:<20} {:>10} {:>14} {:>15} {:>12}",
            "TOTAL", count, total_in, total_out, format!("${:.4}", total_cost)
        );
    }

    Ok(())
}

fn print_usage_table(
    stmt: &mut rusqlite::Statement,
    params: impl rusqlite::Params,
    title: &str,
) -> Result<()> {
    println!("{title}");
    println!("{:<20} {:>10} {:>14} {:>15} {:>12}", "Model", "Queries", "Tokens In", "Tokens Out", "Cost USD");
    println!("{}", "-".repeat(73));
    let rows = stmt.query_map(params, |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, i64>(3)?,
            row.get::<_, f64>(4)?,
        ))
    })?;
    let mut any = false;
    for row in rows.filter_map(|r| r.ok()) {
        any = true;
        println!(
            "{:<20} {:>10} {:>14} {:>15} {:>12}",
            row.0, row.1, row.2, row.3, format!("${:.4}", row.4)
        );
    }
    if !any {
        println!("  (no data)");
    }
    Ok(())
}

// ── zedplus distill ───────────────────────────────────────────────────────────

fn cmd_distill(args: cli::DistillArgs) -> Result<()> {
    use std::io::Write as IoWrite;

    let since_ts = args.since.as_deref().and_then(|s| {
        // Accept YYYY-MM-DD or a unix timestamp string
        if let Ok(ts) = s.parse::<i64>() {
            Some(ts)
        } else {
            chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d")
                .ok()
                .map(|d| d.and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp())
        }
    });

    let lines = distiller::export(
        args.task.as_deref(),
        args.model.as_deref(),
        since_ts,
        args.weighted,
    )?;

    if lines.is_empty() {
        eprintln!("No distillation data matching the given filters.");
        return Ok(());
    }

    match args.out {
        Some(path) => {
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&path)?;
            for line in &lines {
                writeln!(file, "{line}")?;
            }
            eprintln!("Wrote {} examples to {}", lines.len(), path.display());
        }
        None => {
            for line in &lines {
                println!("{line}");
            }
        }
    }

    Ok(())
}

// ── zedplus train ────────────────────────────────────────────────────────────

async fn cmd_train(args: cli::TrainArgs) -> Result<()> {
    use distiller::trainer;

    let db_path = platform::dirs::db_file()?;
    let conn = db::open(&db_path)?;
    let cwd = std::env::current_dir()?;
    let cfg = config::load(Some(&cwd))?;

    if args.status {
        let jobs = trainer::list_jobs(&conn)?;
        if jobs.is_empty() {
            println!("No training jobs yet.");
            println!("Run: zedplus train --base llama3.2:8b --lora");
            return Ok(());
        }
        println!("{:<6} {:<24} {:<6} {:>9}  {:<10}  {}", "Job", "Base Model", "Method", "Examples", "Status", "Output");
        println!("{}", "-".repeat(80));
        for job in &jobs {
            let icon = match job.status.as_str() {
                "complete" => "ok",
                "failed" => "fail",
                "running" => "run",
                _ => "?",
            };
            let output = job.output_model.as_deref()
                .map(|p| {
                    let p = std::path::Path::new(p);
                    p.file_name().and_then(|n| n.to_str()).unwrap_or(p.to_str().unwrap_or("?"))
                })
                .unwrap_or("-");
            println!(
                "{:<6} {:<24} {:<6} {:>9}  {:<10}  {}",
                job.id,
                &job.base_model[..job.base_model.len().min(23)],
                job.method,
                job.dataset_size,
                format!("[{icon}] {}", job.status),
                output,
            );
        }

        // Auto-train suggestion
        if let Ok(Some(suggestion)) = trainer::check_auto_train(&conn, &cfg.config.training) {
            println!();
            println!(
                "Tip: {} ({} new messages). Consider running:",
                suggestion.reason,
                suggestion.new_conversations
            );
            let base = if suggestion.base_model == "auto" {
                trainer::suggest_base_model(&cfg.config.training.primary_use).to_string()
            } else {
                suggestion.base_model
            };
            println!("  zedplus train --base {} --lora", base);
        }

        return Ok(());
    }

    let base_model = match args.base.as_deref() {
        Some(b) => b.to_string(),
        None => {
            // Check auto-train suggestion
            if let Ok(Some(s)) = trainer::check_auto_train(&conn, &cfg.config.training) {
                if s.base_model == "auto" {
                    trainer::suggest_base_model(&cfg.config.training.primary_use).to_string()
                } else {
                    s.base_model
                }
            } else {
                anyhow::bail!(
                    "--base <model> is required.\n  \
                     Example: zedplus train --base llama3.2:8b --lora\n  \
                     Available local models: zedplus model list"
                );
            }
        }
    };

    let method = if args.full { "full" } else { "lora" };

    // Prepare data path — export distillation data if not provided
    let data_path = if let Some(p) = args.data {
        if !p.exists() {
            anyhow::bail!("Data file not found: {}", p.display());
        }
        p
    } else {
        let train_dir = platform::dirs::train_dir()?;
        let export_path = train_dir.join("auto_export.jsonl");

        println!("No --data file provided. Exporting distillation data (recency-weighted)...");
        let lines = distiller::export(None, None, None, true)?;

        if lines.is_empty() {
            anyhow::bail!(
                "No distillation data found. Run some queries first, then retry.\n  \
                 Or provide a data file: zedplus train --base {} --data training.jsonl --lora",
                base_model
            );
        }

        use std::io::Write as IoWrite;
        let mut f = std::fs::OpenOptions::new()
            .create(true).write(true).truncate(true)
            .open(&export_path)?;
        for line in &lines {
            writeln!(f, "{line}")?;
        }
        println!("Exported {} examples to {}", lines.len(), export_path.display());
        export_path
    };

    let dataset_size = {
        let content = std::fs::read_to_string(&data_path)?;
        content.lines().filter(|l| !l.trim().is_empty()).count() as i64
    };

    let output_dir = {
        let slug = base_model.replace([':', '/', ' '], "-");
        let ts = chrono::Utc::now().format("%Y%m%d-%H%M%S");
        platform::dirs::train_dir()?.join(format!("{slug}-{method}-{ts}"))
    };
    std::fs::create_dir_all(&output_dir)?;

    let job_id = trainer::insert_job(&conn, &base_model, method, dataset_size)?;
    drop(conn);

    trainer::run_training(job_id, &base_model, &data_path, method, &output_dir, &db_path).await?;

    if args.bench {
        println!("\n── Auto-Benchmarking ───────────────────────────────────────");
        let model_name = format!("trained-{}", chrono::Utc::now().format("%Y%m%d"));
        
        // Auto-import the newly trained model
        let conn = db::open(&db_path)?;
        trainer::register_model(&conn, &model_name, "local", &output_dir.to_string_lossy(), Some(&output_dir.to_string_lossy()))?;
        println!("✓ Auto-registered as '{model_name}'");

        let bench_args = cli::BenchArgs {
            model: Some(model_name.clone()),
            baseline: Some(base_model.clone()),
            samples: 20, // Quick eval
            history: false,
        };
        
        if let Err(e) = cmd_bench(bench_args).await {
            eprintln!("⚠ Benchmark failed: {e}");
        }
    }

    Ok(())
}

// ── auth / session / model / config / skills ─────────────────────────────────

fn cmd_auth_revoke(provider: &str) -> Result<()> {
    use platform::secrets;
    secrets::delete_secret(&secrets::api_key_name(provider))?;
    secrets::delete_secret(&secrets::oauth_token_name(provider))?;
    secrets::delete_secret(&secrets::oauth_refresh_name(provider))?;
    println!("Credentials for '{provider}' removed from keychain.");
    Ok(())
}

fn cmd_model(args: cli::ModelArgs) -> Result<()> {
    use cli::ModelCommand;
    let registry = config::models::default_registry();
    match args.command {
        ModelCommand::List => {
            println!("{:<25} {:<10} {:>5} {:>5}  {}", "Model", "Provider", "Q", "S", "Strengths");
            println!("{}", "-".repeat(70));
            let mut models: Vec<_> = registry.models.iter().collect();
            models.sort_by_key(|(k, _)| k.as_str());
            for (key, m) in models {
                println!(
                    "{:<25} {:<10} {:>5} {:>5}  {}",
                    key, m.provider, m.quality_tier, m.speed_tier, m.strengths.join(", ")
                );
            }
        }
        ModelCommand::Add { provider, model_id } => {
            println!("Scaffold models.toml entry for {provider}/{model_id} — coming in Phase 12.");
        }
        ModelCommand::Import { source, name } => {
            let db_path = platform::dirs::db_file()?;
            let conn = db::open(&db_path)?;

            let source_path = std::path::Path::new(&source);
            let (provider, model_id, path) = if source_path.exists() {
                ("ollama".to_string(), name.clone(), Some(source.clone()))
            } else {
                // Treat as Ollama model ID (e.g. "llama3.2:8b")
                ("ollama".to_string(), source.clone(), None)
            };

            distiller::trainer::register_model(&conn, &name, &provider, &model_id, path.as_deref())?;
            println!("Model '{name}' registered.");
            println!("  Provider : {provider}");
            println!("  Model ID : {model_id}");
            if let Some(p) = &path {
                println!("  Path     : {p}");
            }
            println!("\nUse it with: zedplus ask --model {name} \"your query\"");
        }
        ModelCommand::Adapters(_) => {
            println!("Community adapters — planned for v2.");
        }
        ModelCommand::Rank => {
            cmd_model_rank()?;
        }
    }
    Ok(())
}

fn cmd_model_rank() -> Result<()> {
    let db_path = platform::dirs::db_file()?;
    if !db_path.exists() {
        println!("No usage data yet. Run some queries first.");
        println!("Reliability scores are built from real usage — test pass rates, negative signals, and override frequency.");
        return Ok(());
    }

    let conn = db::open(&db_path)?;
    let scores = router::adaptive::analyze_reliability(&conn)?;

    if scores.is_empty() {
        println!("No model usage data yet. Run some queries first.");
        return Ok(());
    }

    println!("\nModel Reliability Leaderboard");
    println!("  Scores are weighted: 50% test pass rate · 30% responsiveness · 20% routing accuracy");
    println!("{}", "─".repeat(82));
    println!(
        "{:<4}  {:<22}  {:>6}  {:>12}  {:>8}  {:>9}  {}",
        "Rank", "Model", "Score", "Tests", "Neg Sig", "Overrides", "Flags"
    );
    println!("{}", "─".repeat(82));

    for (i, s) in scores.iter().enumerate() {
        let rank = i + 1;

        let test_col = if s.tests_run > 0 {
            format!("{}/{} ({:.0}%)", s.tests_passed, s.tests_run, s.test_pass_rate * 100.0)
        } else {
            "no data".to_string()
        };

        let neg_col = format!("{:.0}%", s.negative_signal_rate * 100.0);
        let ov_col = format!("{:.0}%", s.override_frequency * 100.0);

        let score_bar = {
            let filled = (s.score * 8.0).round() as usize;
            let empty = 8usize.saturating_sub(filled);
            format!("{:.2} [{}{}]", s.score, "█".repeat(filled), "░".repeat(empty))
        };

        let flags = if s.fresh_eyes_needed {
            "\x1b[33m⚠ Fresh Eyes\x1b[0m"
        } else if s.score >= 0.8 {
            "\x1b[32m● hot\x1b[0m"
        } else if s.score < 0.4 {
            "\x1b[31m● cold\x1b[0m"
        } else {
            ""
        };

        println!(
            "{:<4}  {:<22}  {:>6}  {:>12}  {:>8}  {:>9}  {}",
            rank,
            &s.model[..s.model.len().min(22)],
            score_bar,
            test_col,
            neg_col,
            ov_col,
            flags,
        );
    }

    println!("{}", "─".repeat(82));
    println!("  ⚠ Fresh Eyes = last 2 consecutive test runs failed — router will swap this model out.");
    println!("  Negative signal = user re-asked a similar query within 30s of the response.");
    println!("  Override = user manually typed @model to bypass the router's choice.");
    println!();

    Ok(())
}

fn cmd_config(args: cli::ConfigArgs) -> Result<()> {
    if args.show {
        let cfg = config::load(Some(&std::env::current_dir()?))?;
        let c = &cfg.config;
        let sep = "─".repeat(52);
        println!("\n{sep}");
        println!("  ZedPlus Configuration");
        println!("{sep}");
        println!("  File: {}", platform::dirs::global_config_file()?.display());
        println!();
        println!("  [locale]");
        println!("    country  = {}", c.locale.country);
        println!("    timezone = {}", c.locale.timezone);
        println!("    language = {}", c.locale.language);
        println!();
        println!("  [behavior]");
        println!("    stream              = {}", c.behavior.stream);
        println!("    ui_style            = {:?}", c.behavior.ui_style);
        println!("    default_scope       = {:?}", c.behavior.default_scope);
        println!("    cost_nudge_usd      = ${}", c.behavior.cost_nudge_threshold_usd);
        println!();
        println!("  [routing]");
        println!("    priority            = {:?}", c.routing.priority);
        let r = &c.routing.rules;
        println!("    rules.quick_completion  = {}", r.quick_completion);
        println!("    rules.code_review       = {}", r.code_review);
        println!("    rules.complex_reasoning = {}", r.complex_reasoning);
        println!("    rules.data_analysis     = {}", r.data_analysis);
        println!("    rules.documentation     = {}", r.documentation);
        println!("    rules.web_search        = {}", r.web_search);
        println!("    rules.fallback          = {}", r.fallback);
        let ae = &c.routing.architect_editor;
        println!("    architect_editor.enabled        = {}", ae.enabled);
        println!("    architect_editor.architect_model = {}", ae.architect_model);
        println!("    architect_editor.editor_model   = {}", ae.editor_model);
        println!("    architect_editor.threshold_lines = {}", ae.threshold_lines);
        println!();
        println!("  [privacy]");
        println!("    cloud_allowed = {:?}", c.privacy.cloud_allowed);
        println!();
        println!("  [training]");
        println!("    auto_train     = {}", c.training.auto_train);
        println!("    lora_rank      = {}", c.training.lora_rank);
        println!("    primary_use    = {:?}", c.training.primary_use);
        println!();
        println!("  [brainstorm]");
        println!("    default_strategy       = {}", c.brainstorm.default_strategy);
        println!("    convergence_threshold  = {}", c.brainstorm.convergence_threshold);
        println!("{sep}");
        println!("  Edit: zedplus config --edit");
        println!("  Set:  zedplus config --set routing.rules.code_review=gemini-pro-2-5");
        println!("{sep}\n");
    } else if args.reset {
        config::write_global(&config::schema::Config::default())?;
        println!("Config reset to defaults.");
        println!("  {}", platform::dirs::global_config_file()?.display());
    } else if args.edit {
        let path = platform::dirs::global_config_file()?;
        if !path.exists() {
            config::write_global(&config::schema::Config::default())?;
        }
        let editor = std::env::var("EDITOR")
            .or_else(|_| std::env::var("VISUAL"))
            .unwrap_or_else(|_| if cfg!(windows) { "notepad".to_string() } else { "vi".to_string() });
        std::process::Command::new(&editor).arg(&path).status()?;
    } else if let Some(kv) = args.set {
        let (key, value) = kv.split_once('=')
            .ok_or_else(|| anyhow::anyhow!("Expected KEY=VALUE format, e.g. routing.rules.code_review=gemini-pro-2-5"))?;
        let cwd = std::env::current_dir()?;
        let mut cfg = config::load(Some(&cwd))?.config;
        apply_config_set(&mut cfg, key.trim(), value.trim())?;
        config::write_global(&cfg)?;
        println!("Set {} = {}", key.trim(), value.trim());
        println!("  Saved to {}", platform::dirs::global_config_file()?.display());
    } else {
        println!("Usage:");
        println!("  zedplus config --show                        Show all settings");
        println!("  zedplus config --edit                        Open config in $EDITOR");
        println!("  zedplus config --reset                       Reset to defaults");
        println!("  zedplus config --set KEY=VALUE               Change a setting");
        println!();
        println!("Settable keys:");
        println!("  routing.rules.code_review / complex_reasoning / data_analysis");
        println!("  routing.rules.documentation / web_search / quick_completion / fallback");
        println!("  routing.priority            balanced | quality | cost | localfirst");
        println!("  routing.architect_editor.enabled            true | false");
        println!("  routing.architect_editor.threshold_lines    <number>");
        println!("  behavior.stream             true | false");
        println!("  behavior.ui_style           native | claudecode | geminicli");
        println!("  privacy.cloud_allowed       true | false");
        println!("  training.auto_train         true | false");
        println!("  brainstorm.convergence_threshold  <0.0–1.0>");
    }
    Ok(())
}

fn apply_config_set(cfg: &mut config::schema::Config, key: &str, value: &str) -> Result<()> {
    use config::schema::{RoutingPriority, UiStyle};
    match key {
        "routing.rules.quick_completion"  => cfg.routing.rules.quick_completion  = value.to_string(),
        "routing.rules.code_review"       => cfg.routing.rules.code_review       = value.to_string(),
        "routing.rules.complex_reasoning" => cfg.routing.rules.complex_reasoning = value.to_string(),
        "routing.rules.data_analysis"     => cfg.routing.rules.data_analysis     = value.to_string(),
        "routing.rules.documentation"     => cfg.routing.rules.documentation     = value.to_string(),
        "routing.rules.web_search"        => cfg.routing.rules.web_search        = value.to_string(),
        "routing.rules.fallback"          => cfg.routing.rules.fallback          = value.to_string(),
        "routing.priority" => {
            cfg.routing.priority = match value {
                "balanced"   => RoutingPriority::Balanced,
                "quality"    => RoutingPriority::Quality,
                "cost"       => RoutingPriority::Cost,
                "localfirst" => RoutingPriority::LocalFirst,
                _ => anyhow::bail!("routing.priority must be: balanced | quality | cost | localfirst"),
            };
        }
        "routing.architect_editor.enabled" => {
            cfg.routing.architect_editor.enabled = value.parse()
                .map_err(|_| anyhow::anyhow!("Expected true or false"))?;
        }
        "routing.architect_editor.threshold_lines" => {
            cfg.routing.architect_editor.threshold_lines = value.parse()
                .map_err(|_| anyhow::anyhow!("Expected an integer"))?;
        }
        "routing.architect_editor.architect_model" => cfg.routing.architect_editor.architect_model = value.to_string(),
        "routing.architect_editor.editor_model"    => cfg.routing.architect_editor.editor_model    = value.to_string(),
        "behavior.stream" => {
            cfg.behavior.stream = value.parse()
                .map_err(|_| anyhow::anyhow!("Expected true or false"))?;
        }
        "behavior.ui_style" => {
            cfg.behavior.ui_style = match value {
                "native"      => UiStyle::Native,
                "claudecode"  => UiStyle::ClaudeCode,
                "geminicli"   => UiStyle::GeminiCli,
                _ => anyhow::bail!("ui_style must be: native | claudecode | geminicli"),
            };
        }
        "privacy.cloud_allowed" => {
            cfg.privacy.cloud_allowed = Some(value.parse()
                .map_err(|_| anyhow::anyhow!("Expected true or false"))?);
        }
        "training.auto_train" => {
            cfg.training.auto_train = value.parse()
                .map_err(|_| anyhow::anyhow!("Expected true or false"))?;
        }
        "brainstorm.convergence_threshold" => {
            cfg.brainstorm.convergence_threshold = value.parse()
                .map_err(|_| anyhow::anyhow!("Expected a decimal between 0.0 and 1.0"))?;
        }
        other => anyhow::bail!(
            "Unknown config key '{}'. Run 'zedplus config' to see settable keys.", other
        ),
    }
    Ok(())
}

fn cmd_profile(args: cli::ProfileArgs) -> Result<()> {
    if !args.optimize {
        println!("Use `zedplus profile --optimize` to analyse usage patterns and get routing suggestions.");
        println!("Use `zedplus profile --optimize --apply` to write the suggestions to .zedplus.toml.");
        return Ok(());
    }

    let db_path = platform::dirs::db_file()?;
    if !db_path.exists() {
        println!("No usage data yet. Run some queries first.");
        return Ok(());
    }

    let cwd = std::env::current_dir()?;
    let conn = db::open(&db_path)?;
    let cfg = config::load(Some(&cwd))?;

    let suggestions = router::adaptive::analyze(&conn, &cfg.config.routing.rules, 5)?;

    if suggestions.is_empty() {
        println!("No routing changes suggested yet.");
        println!("(Need at least 5 consistent overrides per task type to suggest a change.)");
        return Ok(());
    }

    println!("Suggested routing changes based on your usage patterns:");
    println!("{}", "-".repeat(72));
    for s in &suggestions {
        println!("{}", s.diff_line());
    }
    println!("{}", "-".repeat(72));

    if args.apply {
        // Apply to project config (.zedplus.toml) if it exists, else global
        let project_config_path = cwd.join(".zedplus.toml");
        let mut project_cfg = if project_config_path.exists() {
            let raw = std::fs::read_to_string(&project_config_path)?;
            toml::from_str::<config::schema::Config>(&raw)?
        } else {
            config::schema::Config::default()
        };

        let changed = router::adaptive::apply(&mut project_cfg.routing.rules, &suggestions);

        let raw = toml::to_string_pretty(&project_cfg)?;
        std::fs::write(&project_config_path, raw)?;

        println!("\nApplied to .zedplus.toml:");
        for line in &changed {
            println!("  {line}");
        }
    } else {
        println!("\nRun with --apply to write these changes to .zedplus.toml.");
    }

    Ok(())
}

fn cmd_session(args: cli::SessionArgs) -> Result<()> {
    use cli::SessionCommand;

    let db_path = platform::dirs::db_file()?;

    match args.command {
        SessionCommand::List { all } => {
            if !db_path.exists() {
                println!("No sessions yet.");
                return Ok(());
            }
            let conn = db::open(&db_path)?;
            let project_path = if all {
                None
            } else {
                Some(std::env::current_dir()?.to_string_lossy().to_string())
            };
            let list = sessions::list_sessions(&conn, project_path.as_deref(), 30)?;

            println!("{:<30} {:>6} {:>10}  {:<16}  {}", "Name", "Turns", "Cost", "Last active", "Branch");
            println!("{}", "-".repeat(72));

            if list.is_empty() {
                println!("  (no sessions)");
            } else {
                for s in &list {
                    let branch = s.git_branch.as_deref().unwrap_or("-");
                    let ts = chrono::DateTime::from_timestamp(s.last_active, 0)
                        .map(|dt| dt.format("%m-%d %H:%M").to_string())
                        .unwrap_or_default();
                    println!(
                        "{:<30} {:>6} {:>10}  {:<16}  {}",
                        s.name, s.turn_count, format!("${:.4}", s.total_cost), ts, branch
                    );
                }
            }
        }

        SessionCommand::Resume { name } => {
            if !db_path.exists() {
                println!("No sessions found.");
                return Ok(());
            }
            let conn = db::open(&db_path)?;
            // Find by name
            let found: Option<sessions::ResumableSession> = {
                let all = sessions::list_sessions(&conn, None, 200)?;
                all.into_iter().find(|s| s.name == name)
            };
            match found {
                None => println!("Session '{name}' not found. Use `zedplus session list`."),
                Some(s) => {
                    let turns = sessions::load_turns(&conn, &s.id);
                    drop(conn);
                    // Need to run async — caller is sync, so we block
                    tokio::task::block_in_place(|| {
                        tokio::runtime::Handle::current().block_on(repl::run_resumed(
                            s.id.clone(),
                            s.name.clone(),
                            s.git_branch.clone(),
                            s.turn_count as u32,
                            s.total_cost,
                            turns,
                        ))
                    })?;
                }
            }
        }

        SessionCommand::Rename { old, new } => {
            if !db_path.exists() { println!("No sessions found."); return Ok(()); }
            let conn = db::open(&db_path)?;
            let n = conn.execute(
                "UPDATE sessions SET name = ?1 WHERE name = ?2",
                rusqlite::params![new, old],
            )?;
            if n == 0 {
                println!("Session '{old}' not found.");
            } else {
                println!("Renamed '{old}' → '{new}'.");
            }
        }

        SessionCommand::Archive { name } => {
            if !db_path.exists() { println!("No sessions found."); return Ok(()); }
            let conn = db::open(&db_path)?;
            let n = conn.execute(
                "UPDATE sessions SET status = 'archived' WHERE name = ?1",
                rusqlite::params![name],
            )?;
            if n == 0 {
                println!("Session '{name}' not found.");
            } else {
                println!("Session '{name}' archived.");
            }
        }
    }

    Ok(())
}

fn cmd_skills(args: cli::SkillsArgs) -> Result<()> {
    use cli::SkillsCommand;
    match args.command {
        SkillsCommand::List => println!("Skill packs — coming in Phase 15."),
        SkillsCommand::Install { name } => println!("Install '{name}' — coming in Phase 15."),
        SkillsCommand::Suggest => println!("Skill suggestions — coming in Phase 15."),
        SkillsCommand::Create { name } => println!("Create skill '{name}' — coming in Phase 15."),
    }
    Ok(())
}

// ── Phase 10: zedplus bench ───────────────────────────────────────────────────

async fn cmd_bench(args: cli::BenchArgs) -> Result<()> {
    use distiller::bench;
    use chrono::Utc;

    let db_path = platform::dirs::db_file()?;
    let conn = db::open(&db_path)?;
    let cfg = config::load(Some(&std::env::current_dir()?))?;

    let model_alias = args.model.as_deref().unwrap_or("claude-haiku");
    let baseline_alias = args.baseline.as_deref();

    // --history: show stored results only
    if args.history {
        let scores = bench::load_last_results(&conn, model_alias, args.samples);
        let baseline_scores = baseline_alias
            .map(|b| bench::load_last_results(&conn, b, args.samples));
        bench::print_summary(model_alias, &scores, baseline_alias, baseline_scores.as_deref());
        return Ok(());
    }

    let distill_dir = platform::dirs::distill_dir()?;
    let entries = bench::load_entries(&distill_dir, args.samples);

    if entries.is_empty() {
        println!("No distillation data found. Run some queries first (`zedplus ask ...`).");
        println!("Each query-response pair is saved automatically for benchmarking.");
        return Ok(());
    }

    println!("Benchmarking '{model_alias}' on {} samples...", entries.len());
    println!("(Sends each stored query to the model and scores against the gold response.)");
    println!();

    let (provider, model_id) = backends::resolve_model(model_alias, &cfg.models)
        .ok_or_else(|| anyhow::anyhow!("Unknown model alias: {model_alias}"))?;
    let api_key = get_api_key(&provider, &cfg)?;
    let ollama_url = cfg.config.services.ollama_url.as_deref().unwrap_or("http://localhost:11434");
    let backend = backends::create_backend(&provider, &api_key, ollama_url);

    let ts = Utc::now().timestamp();
    let mut scores: Vec<bench::BenchScore> = Vec::new();
    let embedder = indexer::embedder::Embedder::new(ollama_url);
    let embed_available = embedder.is_available().await;

    for (i, entry) in entries.iter().enumerate() {
        print!("  [{:>3}/{}] {} … ", i + 1, entries.len(), &entry.query.chars().take(40).collect::<String>());
        use std::io::Write as _;
        std::io::stdout().flush()?;

        let opts = backends::CompletionOptions {
            model_id: model_id.clone(),
            system: None,
            messages: vec![backends::Message {
                role: "user".to_string(),
                content: entry.query.clone(),
            }],
            max_tokens: 1024,
            use_search_grounding: false,
            use_cache: false,
            auto_accept: false,
        };

        match backend.complete(opts).await {
            Ok(r) => {
                let f1 = bench::token_f1(&entry.gold, &r.content);
                let lr = bench::length_ratio(&entry.gold, &r.content);
                let fmt = bench::check_format(&entry.gold, &r.content);
                
                let sem = if embed_available {
                    if let (Ok(g_v), Ok(p_v)) = (embedder.embed(&entry.gold).await, embedder.embed(&r.content).await) {
                        indexer::embedder::cosine_similarity(&g_v, &p_v)
                    } else { 0.0 }
                } else { 0.0 };

                let score = bench::BenchScore {
                    example_id: entry.id.clone(),
                    task_type: entry.task_type.clone(),
                    token_f1: f1,
                    semantic_sim: sem,
                    length_ratio: lr,
                    format_correct: fmt,
                };
                println!("F1={:.3} Sem={:.3}", f1, sem);
                bench::save_result(&conn, model_alias, baseline_alias, &score, ts)?;
                scores.push(score);
            }
            Err(e) => {
                println!("error: {e}");
            }
        }
    }

    // Load baseline scores if requested
    let baseline_scores = baseline_alias
        .map(|b| bench::load_last_results(&conn, b, args.samples));

    bench::print_summary(model_alias, &scores, baseline_alias, baseline_scores.as_deref());
    Ok(())
}

// ── Phase 11: zedplus update ──────────────────────────────────────────────────

async fn cmd_update(args: cli::UpdateArgs) -> Result<()> {
    use platform::update;
    use std::io::Write as _;

    const CURRENT: &str = env!("CARGO_PKG_VERSION");

    print!("Checking for updates (current: v{CURRENT})...");
    std::io::stdout().flush()?;

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .user_agent("zedplus-updater")
        .build()?;

    let release = match update::fetch_latest(&client).await {
        Ok(r) => r,
        Err(e) => {
            println!(" offline or unreachable ({e}).");
            println!("  Current version: v{CURRENT}");
            return Ok(());
        }
    };

    if release.version.is_empty() {
        println!(" unable to parse release tag.");
        return Ok(());
    }

    if release.version == CURRENT {
        println!(" v{CURRENT} is up to date.");
        return Ok(());
    }

    println!(" v{} available!", release.version);

    if args.check {
        println!();
        println!("  Run `zedplus update` to download and install.");
        return Ok(());
    }

    // Confirm before updating
    println!();
    print!("  Install v{}? [y/N] ", release.version);
    std::io::stdout().flush()?;
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    if !input.trim().eq_ignore_ascii_case("y") {
        println!("  Aborted.");
        return Ok(());
    }

    match update::perform_update(&client, &release).await {
        Ok(installed) => {
            println!("  Update installed to: {}", installed.display());
            #[cfg(windows)]
            {
                println!();
                println!("  On Windows the running binary cannot be replaced while it is running.");
                println!("  The new binary has been staged as:");
                println!("    {}", installed.display());
                println!();
                println!("  To complete the update, close this terminal and run:");
                println!("    Move-Item -Force zedplus_new.exe zedplus.exe");
                println!("  (or re-run the installer: .\\install.ps1)");
            }
        }
        Err(e) => {
            eprintln!("  Update failed: {e}");
            eprintln!("  You can update manually from: https://github.com/{}/releases", update::REPO);
        }
    }

    Ok(())
}

// ── shared helpers ────────────────────────────────────────────────────────────

pub fn get_api_key(provider: &str, cfg: &config::LoadedConfig) -> Result<String> {
    use platform::secrets;

    // Local providers need no key
    if matches!(provider, "ollama" | "lmstudio") {
        return Ok(String::new());
    }

    // Normalize registry provider names ("claude", "gemini") to keychain canonical names
    let canonical = match provider {
        "claude" => "anthropic",
        "gemini" => "google",
        p => p,
    };

    let env_var = match canonical {
        "anthropic" => "ZEDPLUS_API_KEY_ANTHROPIC",
        "google" => "ZEDPLUS_API_KEY_GOOGLE",
        "openai" => "ZEDPLUS_API_KEY_OPENAI",
        _ => "ZEDPLUS_API_KEY_ANTHROPIC",
    };

    if let Ok(key) = std::env::var(env_var) {
        if !key.is_empty() {
            return Ok(key);
        }
    }

    let keychain_name = secrets::api_key_name(canonical);
    if let Some(key) = secrets::get_secret(&keychain_name)? {
        return Ok(key);
    }

    if canonical == "google" {
        if let Some(token) = secrets::get_secret(&secrets::oauth_token_name(canonical))? {
            return Ok(token);
        }
    }

    anyhow::bail!(
        "No API key found for '{provider}'.\n  \
         Run `zedplus auth --provider {canonical}` to authenticate.\n  \
         Or set the {env_var} environment variable."
    )
}

/// Return the next available provider to failover to when `current_provider` is rate-limited.
/// Returns (alias, provider, model_id) or None if no alternative is configured.
pub fn failover_provider(
    current_provider: &str,
    cfg: &config::LoadedConfig,
) -> Option<(String, String, String)> {
    let current = match current_provider {
        "claude" | "anthropic" | "claude-cli" => "anthropic",
        "gemini" | "google" | "gemini-cli" => "google",
        "openai" => "openai",
        _ => "",
    };

    // Prefer CLI subscriptions first — they have no per-token cost and separate
    // rate limits from the API. Only skip if the current failure IS the CLI.
    if current != "anthropic" {
        if which_binary("claude") {
            return Some(("claude-cli".to_string(), "claude-cli".to_string(), String::new()));
        }
    }
    if current != "google" {
        if which_binary("gemini") {
            return Some(("gemini-cli".to_string(), "gemini-cli".to_string(), String::new()));
        }
    }

    // API fallbacks
    let candidates = [
        ("gemini-flash", "google"),
        ("claude-haiku", "anthropic"),
        ("codex-mini", "openai"),
        ("gpt-4o-mini", "openai"),
        ("lmstudio", "lmstudio"),
        ("local", "ollama"),
    ];
    for (alias, key_prov) in candidates {
        if key_prov == current { continue; }
        if let Ok(key) = get_api_key(key_prov, cfg) {
            if !key.is_empty() || matches!(key_prov, "ollama" | "lmstudio") {
                if let Some((prov, mid)) = backends::resolve_model(alias, &cfg.models) {
                    return Some((alias.to_string(), prov, mid));
                }
            }
        }
    }
    None
}

/// Returns true if `name` is found as an executable on PATH.
fn which_binary(name: &str) -> bool {
    std::process::Command::new(if cfg!(windows) { "where" } else { "which" })
        .arg(name)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Upgrade / usage URL for a rate-limited provider.
pub fn rate_limit_upgrade_url(provider: &str) -> String {
    match provider {
        "claude" | "anthropic" => "Manage usage: https://console.anthropic.com/settings/limits".to_string(),
        "gemini" | "google" => "Manage usage: https://aistudio.google.com/apikey".to_string(),
        "openai" => "Manage usage: https://platform.openai.com/usage".to_string(),
        _ => String::new(),
    }
}
