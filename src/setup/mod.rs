pub mod detector;
pub mod profile;
pub mod services;

use crate::config::{self, schema::*};
use crate::config::schema::UiStyle;
use crate::context::locale::LocaleContext;
use crate::platform::{auth, secrets};
use anyhow::Result;
use crossterm::style::Stylize;
use detector::LocalLlmVerdict;
use inquire::{Confirm, Select, Text};
use reqwest::Client;
use std::io::{IsTerminal, Write as _};

pub async fn run_init(context: bool) -> Result<()> {
    if context {
        println!("ZEDPLUS.md generation — coming in Phase 12b.");
        return Ok(());
    }

    if !std::io::stdin().is_terminal() {
        anyhow::bail!(
            "zedplus init requires an interactive terminal.\n\
             Run it directly, not from a pipe or CI script."
        );
    }

    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()?;

    print_header();

    // ── Step 1/8: Locale ────────────────────────────────────────────────────
    print_step(1, 8, "Where are you based?");
    let locale = step_locale()?;

    // ── Step 2/8: CLI tool detection ─────────────────────────────────────────
    print_step(2, 8, "Detecting CLI tools");
    let cli = detector::detect_cli_tools();
    let claude_tag = if cli.claude { "claude CLI found ✓" } else { "claude CLI not found" };
    let gemini_tag = if cli.gemini { "gemini CLI found ✓" } else { "gemini CLI not found" };
    println!("  [{}] [{}]", claude_tag, gemini_tag);
    if cli.openai_cli { println!("  openai CLI found ✓"); }
    if cli.groq { println!("  groq CLI found ✓"); }
    if cli.qwen { println!("  qwen CLI found ✓"); }
    if cli.aider { println!("  aider found ✓"); }

    // ── Step 3/8: UI style mimic ─────────────────────────────────────────────
    print_step(3, 8, "Which CLI interface would you like ZedPlus to mimic?");
    let ui_style = step_ui_style(&cli)?;

    // ── Step 4/8: Services ──────────────────────────────────────────────────
    print_step(4, 8, "Which AI services do you have access to?");
    let svc = services::prompt_services(&client).await?;

    // ── Step 5/8: Use cases ─────────────────────────────────────────────────
    print_step(5, 8, "What do you primarily use AI for?");
    let user_profile = profile::prompt_use_cases()?;

    // ── Step 6/8: Routing priority ──────────────────────────────────────────
    print_step(6, 8, "What's your routing priority?");
    let priority = profile::prompt_routing_priority()?;

    // ── Step 7/8: API keys ──────────────────────────────────────────────────
    print_step(7, 8, "API key setup");
    let keys = services::configure_all_services(&svc, &client).await?;

    // ── Device scan ─────────────────────────────────────────────────────────
    println!();
    print_step_label("Device scan");
    let (device_info, llm_verdict) = detector::scan();
    display_device_verdict(&device_info, &llm_verdict);

    // ── Build routing plan ──────────────────────────────────────────────────
    let routing_rules = compute_routing_rules(&svc, &priority, &llm_verdict, &user_profile);
    let costs = crate::config::costs::default_costs();
    display_routing_plan(&routing_rules, &costs);

    // ── Step 8/8: Auto-train ─────────────────────────────────────────────────
    print_step(8, 8, "Local model auto-training");
    let training = profile::prompt_auto_train(&llm_verdict)?;

    // ── Confirm and save ────────────────────────────────────────────────────
    println!();
    let save = Confirm::new("Save configuration and store API keys?")
        .with_default(true)
        .prompt()?;

    if !save {
        println!("Aborted — nothing was saved.");
        return Ok(());
    }

    let config = Config {
        locale,
        routing: RoutingConfig {
            priority,
            rules: routing_rules,
            ..Default::default()
        },
        training,
        behavior: BehaviorConfig {
            ui_style,
            ..Default::default()
        },
        services: ServicesConfig {
            anthropic: svc.anthropic,
            google: svc.google,
            openai: svc.openai,
            ollama: svc.ollama,
            ollama_url: if svc.ollama { Some(svc.ollama_url.clone()) } else { None },
            lmstudio: svc.lmstudio,
            lmstudio_url: if svc.lmstudio { Some(svc.lmstudio_url.clone()) } else { None },
            use_cases: user_profile,
            ..Default::default()
        },
        ..Default::default()
    };

    config::write_global(&config)?;
    println!("  ✓ Config written to {}", crate::platform::dirs::global_config_file()?.display());

    for (provider, key) in &keys {
        secrets::store_secret(&secrets::api_key_name(provider), key)?;
    }
    if !keys.is_empty() {
        println!("  ✓ {} API key(s) stored in OS keychain", keys.len());
    }

    println!();
    println!("{}", "Setup complete!".green().bold());
    println!(
        "  Run {} to try it out.",
        "zedplus ask \"hello\"".cyan()
    );
    println!(
        "  Run {} to re-run this wizard.",
        "zedplus init".cyan()
    );

    Ok(())
}

// ── Auth subcommand ──────────────────────────────────────────────────────────

pub async fn run_auth(provider: Option<String>) -> Result<()> {
    if !std::io::stdin().is_terminal() {
        anyhow::bail!("zedplus auth requires an interactive terminal.");
    }

    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()?;

    let providers: Vec<&str> = match provider.as_deref() {
        Some(p) => vec![p],
        None => vec!["anthropic", "google", "openai"],
    };

    for p in providers {
        println!("\n── {} ─────────────────────────────", p);
        let (display_name, url) = match p {
            "anthropic" => ("Anthropic", auth::ANTHROPIC_KEYS_URL),
            "google" => ("Google AI Studio", auth::GOOGLE_AI_STUDIO_URL),
            "openai" => ("OpenAI", auth::OPENAI_KEYS_URL),
            other => {
                println!("  Unknown provider '{other}'. Supported: anthropic, google, openai");
                continue;
            }
        };

        let choice = Select::new(
            &format!("  Authenticate {}:", display_name),
            vec!["[B] Open browser → paste key", "[M] Manual entry", "[S] Skip"],
        )
        .prompt()?;

        let result = if choice.starts_with("[B]") {
            auth::browser_assist_key(&client, display_name, url).await
        } else if choice.starts_with("[S]") {
            println!("  Skipped.");
            continue;
        } else {
            auth::manual_key(&client, display_name).await
        };

        match result {
            Ok(key) => {
                secrets::store_secret(&secrets::api_key_name(p), &key)?;
                println!("  ✓ Stored in OS keychain");
            }
            Err(e) => println!("  Skipped: {e}"),
        }
    }

    Ok(())
}

// ── Wizard helpers ───────────────────────────────────────────────────────────

fn step_locale() -> Result<LocaleConfig> {
    let detected = LocaleContext::detect();
    println!(
        "  Detected: country={}, timezone={}, language={}",
        detected.country, detected.timezone, detected.language
    );

    let change = Confirm::new("Change locale settings?")
        .with_default(false)
        .prompt()?;

    if !change {
        return Ok(detected);
    }

    let country = Text::new("Country code (ISO 3166-1 alpha-2, e.g. US, GB, AU):")
        .with_initial_value(&detected.country)
        .prompt()?
        .trim()
        .to_uppercase();

    let timezone = Text::new("Timezone (IANA, e.g. America/New_York):")
        .with_initial_value(&detected.timezone)
        .prompt()?
        .trim()
        .to_string();

    let language = Text::new("Language (BCP 47, e.g. en-US):")
        .with_initial_value(&detected.language)
        .prompt()?
        .trim()
        .to_string();

    // Keep format/units/currency from detected defaults for the country
    let (date_format, units, currency) = crate::context::locale::defaults_for_country(&country);

    Ok(LocaleConfig {
        country,
        timezone,
        language,
        date_format,
        units,
        currency,
    })
}

fn step_ui_style(cli: &detector::CliDetection) -> Result<UiStyle> {
    let mut options: Vec<(&str, UiStyle)> = vec![("ZedPlus native", UiStyle::Native)];
    if cli.claude {
        options.push(("Claude Code style", UiStyle::ClaudeCode));
    }
    if cli.gemini {
        options.push(("Gemini CLI style", UiStyle::GeminiCli));
    }
    if cli.claude && cli.gemini {
        options.push(("Both (auto-switch per provider)", UiStyle::Native));
    }

    let labels: Vec<&str> = options.iter().map(|(l, _)| *l).collect();
    let choice = Select::new("Choose UI style:", labels).prompt()?;
    let style = options
        .into_iter()
        .find(|(l, _)| *l == choice)
        .map(|(_, s)| s)
        .unwrap_or(UiStyle::Native);
    Ok(style)
}

fn compute_routing_rules(
    svc: &services::SelectedServices,
    priority: &RoutingPriority,
    verdict: &LocalLlmVerdict,
    _use_cases: &[String],
) -> RoutingRules {
    let has_local = (svc.ollama || svc.lmstudio) && !matches!(verdict, LocalLlmVerdict::Disabled { .. });
    let local = if !has_local {
        "claude-haiku"
    } else if svc.lmstudio && !svc.ollama {
        "lmstudio"   // LM Studio only — Ollama not installed
    } else {
        "local"      // Ollama (possibly alongside LM Studio)
    };

    // Quick completion: local if available (free), otherwise cheapest cloud
    let quick = if has_local { "local" } else { cheapest_available(svc) };

    let code_review = match priority {
        RoutingPriority::Quality => best_code_model(svc),
        RoutingPriority::Cost => cheapest_available(svc),
        RoutingPriority::LocalFirst if has_local => "local",
        _ => {
            if svc.anthropic { "claude-sonnet" }
            else if svc.openai { "gpt-4o" }
            else if svc.google { "gemini-pro" }
            else { "local" }
        }
    };

    let data_analysis = if svc.google { "gemini-pro" }
        else if svc.anthropic { "claude-sonnet" }
        else { local };

    let documentation = if svc.anthropic { "claude-haiku" }
        else if svc.google { "gemini-flash" }
        else { local };

    let web_search = if svc.google { "gemini-flash" }
        else if svc.anthropic { "claude-sonnet" }
        else { local };

    let fallback = if svc.anthropic { "claude-haiku" }
        else if svc.google { "gemini-flash" }
        else if svc.openai { "gpt-4o-mini" }
        else { "local" };

    RoutingRules {
        web_search: web_search.to_string(),
        quick_completion: quick.to_string(),
        code_review: code_review.to_string(),
        complex_reasoning: code_review.to_string(),
        data_analysis: data_analysis.to_string(),
        documentation: documentation.to_string(),
        fallback: fallback.to_string(),
    }
}

fn best_code_model(svc: &services::SelectedServices) -> &'static str {
    if svc.anthropic { "claude-sonnet" }
    else if svc.openai { "gpt-4o" }
    else if svc.google { "gemini-pro" }
    else { "local" }
}

fn cheapest_available(svc: &services::SelectedServices) -> &'static str {
    if svc.google { "gemini-flash" }
    else if svc.anthropic { "claude-haiku" }
    else if svc.openai { "gpt-4o-mini" }
    else { "local" }
}

fn display_device_verdict(info: &detector::DeviceInfo, verdict: &LocalLlmVerdict) {
    println!(
        "  RAM: {:.0} GB  |  CPU cores: {}{}",
        info.total_ram_gb,
        info.cpu_count,
        info.vram_gb
            .map(|v| format!("  |  GPU VRAM: {v:.0} GB"))
            .unwrap_or_default()
    );

    match verdict {
        LocalLlmVerdict::Disabled { reason } => {
            println!("  ⚠  Local LLM: disabled — {reason}");
        }
        LocalLlmVerdict::CpuOnly { max_size } => {
            println!("  → Local LLM: CPU-only, up to {max_size}");
        }
        LocalLlmVerdict::GpuSmall { max_size }
        | LocalLlmVerdict::GpuMedium { max_size }
        | LocalLlmVerdict::GpuLarge { max_size } => {
            let suggested = verdict.suggested_model().unwrap_or("llama3.2:8b");
            println!("  → Local LLM: GPU, up to {max_size}");
            println!("  → Suggested: pull `ollama pull {suggested}` for free completions");
        }
    }
}

fn display_routing_plan(rules: &RoutingRules, costs: &crate::config::costs::CostsTable) {
    println!("\n  Routing plan:");
    let entries = [
        ("web search      ", &rules.web_search),
        ("code review     ", &rules.code_review),
        ("data analysis   ", &rules.data_analysis),
        ("documentation   ", &rules.documentation),
        ("quick completion", &rules.quick_completion),
        ("fallback        ", &rules.fallback),
    ];
    for (label, model) in &entries {
        let free = model.as_str() == "local";
        let tag = if free { "  ← free".cyan().to_string() } else { String::new() };
        println!("    {label}  →  {}{}", model.as_str().bold(), tag);
    }

    let monthly = estimate_monthly_cost(rules, costs);
    println!("\n  Estimated cost at ~200 queries/day: ~${monthly:.0}–${:.0}/month",
        monthly * 1.4);
}

/// Rough monthly cost estimate based on a 200 queries/day mix.
fn estimate_monthly_cost(rules: &RoutingRules, costs: &crate::config::costs::CostsTable) -> f64 {
    // Assumed task distribution at 200 queries/day
    let daily_mix = [
        (&rules.web_search,       30u32, 500u32,  200u32),  // 30 queries
        (&rules.code_review,      40u32, 2000u32, 800u32),
        (&rules.data_analysis,    20u32, 1500u32, 600u32),
        (&rules.documentation,    30u32, 800u32,  400u32),
        (&rules.quick_completion, 50u32, 200u32,  150u32),
        (&rules.fallback,         30u32, 500u32,  200u32),
    ];

    let daily_cost: f64 = daily_mix.iter().map(|(model, count, inp, out)| {
        let key = resolve_model_alias(model.as_str());
        costs.cost_usd(key, inp * count, out * count)
    }).sum();

    daily_cost * 30.0
}

fn resolve_model_alias(alias: &str) -> &'static str {
    match alias {
        "claude-haiku"      => "claude-haiku-4-5",
        "claude-sonnet"     => "claude-sonnet-4-6",
        "claude-opus"       => "claude-opus-4-7",
        "gemini-flash"      => "gemini-flash-3-1",
        "gemini-flash-2-5"  => "gemini-flash-2-5",
        "gemini-flash-3-1"  => "gemini-flash-3-1",
        "gemini-pro"        => "gemini-pro-3-1",
        "gemini-pro-2-5"    => "gemini-pro-2-5",
        "gemini-pro-3-1"    => "gemini-pro-3-1",
        "local" | "lmstudio" => "local",
        _ => "local",
    }
}

// ── UI helpers ───────────────────────────────────────────────────────────────

fn print_header() {
    println!();
    println!("{}", "╭─ ZedPlus Setup ───────────────────────────────────────────────╮".bold());
    println!("{}", "│  Let's configure your AI routing in ~2 minutes.               │".bold());
    println!("{}", "╰───────────────────────────────────────────────────────────────╯".bold());
    println!();
}

fn print_step(n: u8, total: u8, title: &str) {
    println!();
    println!("{}", format!("Step {n}/{total}  ─  {title}").bold().cyan());
}

fn print_step_label(label: &str) {
    println!("{}", format!("──  {label}  ──────────────────────────────────────────").dim());
}
