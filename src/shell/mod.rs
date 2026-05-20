pub mod integration;

use anyhow::Result;
use std::io::Write as IoWrite;

/// Generate a shell command from a natural-language description, confirm with the user,
/// then execute it. If `inline` is true, print the command and return without prompting.
pub async fn run(description: &str, inline: bool) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let cfg = crate::config::load(Some(&cwd))?;

    let os_hint = if cfg!(windows) {
        "Windows (PowerShell or cmd)"
    } else if cfg!(target_os = "macos") {
        "macOS (bash/zsh)"
    } else {
        "Linux (bash)"
    };

    let system = format!(
        "You are a shell command generator for {os_hint}. \
         Output ONLY the shell command — no explanation, no markdown fences, no extra text."
    );
    let prompt = format!(
        "Generate a single shell command that does: {description}"
    );

    // Use cheapest model — shell commands are simple completions
    let decision = crate::router::route(
        &prompt,
        &cfg.config,
        &cfg.models,
        &cfg.costs,
        None,
        false,
        true, // cheap=true
        None,
    );
    let api_key = crate::get_api_key(&decision.provider, &cfg).unwrap_or_default();
    let ollama_url = cfg
        .config
        .services
        .ollama_url
        .as_deref()
        .unwrap_or("http://localhost:11434");
    let backend = crate::backends::create_backend(&decision.provider, &api_key, ollama_url);

    let opts = crate::backends::CompletionOptions {
        model_id: decision.model_id.clone(),
        system: Some(system),
        messages: vec![crate::backends::Message {
            role: "user".to_string(),
            content: prompt,
        }],
        max_tokens: 256,
        use_search_grounding: false,
        use_cache: false,
        auto_accept: false,
    };

    let response = backend.complete(opts).await?;
    let raw = response.content.trim().to_string();

    // Strip markdown fences the model sometimes adds despite instructions
    let command = raw
        .trim_start_matches("```powershell")
        .trim_start_matches("```bash")
        .trim_start_matches("```sh")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim()
        .to_string();

    if inline {
        println!("{command}");
        return Ok(());
    }

    // Interactive confirm flow
    println!("\n  \x1b[1m{command}\x1b[0m");
    print!("  Run this? [Y/n/e(dit)] ");
    std::io::stdout().flush()?;

    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    let choice = input.trim().to_lowercase();

    if choice == "n" || choice == "no" {
        println!("Cancelled.");
        return Ok(());
    }

    let command = if choice == "e" || choice == "edit" {
        print!("  Command: ");
        std::io::stdout().flush()?;
        let mut edited = String::new();
        std::io::stdin().read_line(&mut edited)?;
        edited.trim().to_string()
    } else {
        command
    };

    println!("\x1b[90m$ {command}\x1b[0m");

    let status = if cfg!(windows) {
        std::process::Command::new("cmd")
            .args(["/C", &command])
            .status()?
    } else {
        std::process::Command::new("sh")
            .args(["-c", &command])
            .status()?
    };

    if !status.success() {
        eprintln!("\x1b[31mCommand exited with code {:?}\x1b[0m", status.code());
        std::process::exit(status.code().unwrap_or(1));
    }

    Ok(())
}
