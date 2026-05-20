pub mod architect;
pub mod selector;

use anyhow::Result;
use chrono::Utc;
use std::io::{self, Write as IoWrite};

use crate::{agent, backends::{self, CompletionOptions, Message}, config, setup::detector::CliDetection};
use selector::{BackendChoice, PhaseKind};

// ── Hard constraints embedded in every code-touching phase ────────────────────
//
// Single-model CLIs can silently drop functional code when "optimising".
// These rules are injected into all prompts that read or review code to
// make that behaviour a prompt violation rather than a silent default.

const NO_DELETE_RULE: &str = "\n\n\
    NON-NEGOTIABLE RULES — violating these is a critical failure:\n\
    1. NEVER remove, delete, overwrite, comment out, or truncate any existing \
       functional code — even if you think it is redundant or can be improved.\n\
    2. If you identify code to improve, annotate it with a comment but leave the \
       original completely intact.\n\
    3. All changes must be ADDITIVE ONLY. No subtractions.\n\
    4. If a file already exists, never rewrite it entirely — only append or \
       insert targeted additions.\n\
    Violations cause irreversible data loss and will be flagged as errors.";

// ── Context trimming ──────────────────────────────────────────────────────────
//
// CLI backends have practical limits on what we can pass via --print / -p.
// Cloud APIs also have rate limits on very large prompts.
// We trim artifact text to a safe character budget before embedding it into
// a phase prompt; the full content is always available in the devlog.

const REASONING_CTX_CHARS: usize = 12_000;
const PLANNING_CTX_CHARS:  usize = 10_000;
const BUILD_CTX_CHARS:     usize = 24_000; // build needs full arch + plan

fn trim(text: &str, max: usize) -> std::borrow::Cow<str> {
    if text.len() <= max {
        std::borrow::Cow::Borrowed(text)
    } else {
        // Cut at last newline within budget to avoid mid-line breaks
        let cut = text[..max].rfind('\n').unwrap_or(max);
        std::borrow::Cow::Owned(format!("{}\n\n[...truncated — full content in devlog...]", &text[..cut]))
    }
}

// ── Artifacts ─────────────────────────────────────────────────────────────────

pub struct Artifacts {
    pub original_query: String,
    pub clarifications: Vec<(String, String)>,
    pub architecture:   String,
    pub build_plan:     String,
    pub verified_plan:  String,
    pub build_summary:  String,
    pub qc_report:      String,
    pub arch_check:     String,
    pub test_plan:      String,
    pub devlog_path:    Option<std::path::PathBuf>,
}

impl Default for Artifacts {
    fn default() -> Self {
        Self {
            original_query: String::new(),
            clarifications: Vec::new(),
            architecture:   String::new(),
            build_plan:     String::new(),
            verified_plan:  String::new(),
            build_summary:  String::new(),
            qc_report:      String::new(),
            arch_check:     String::new(),
            test_plan:      String::new(),
            devlog_path:    None,
        }
    }
}

// ── Phase execution helpers ───────────────────────────────────────────────────

/// Tries each backend in order until one succeeds. Returns (content, label).
async fn run_phase(
    kind: PhaseKind,
    prompt: &str,
    cfg: &config::LoadedConfig,
    cli: &CliDetection,
    ollama_url: &str,
) -> Result<(String, String)> {
    let candidates = selector::cascade(kind, cfg, cli, ollama_url);
    if candidates.is_empty() {
        anyhow::bail!("No backend available for {kind:?} phase — configure an API key or install a CLI tool.");
    }

    for choice in candidates {
        let label = choice.label.clone();
        match complete_once(choice, prompt).await {
            Ok(content) => return Ok((content, label)),
            Err(e) => {
                let reason = e.to_string();
                // Only fall through on connectivity / auth failures, not on user-cancel
                if is_retryable(&reason) {
                    eprintln!("  \x1b[33m[fallback]\x1b[0m {label} failed ({reason:.80}), trying next...");
                } else {
                    return Err(e);
                }
            }
        }
    }

    anyhow::bail!("All backends exhausted for {kind:?} phase.")
}

async fn complete_once(choice: BackendChoice, prompt: &str) -> Result<String> {
    let opts = CompletionOptions {
        model_id: choice.model_id.clone(),
        system:   None,
        messages: vec![Message { role: "user".into(), content: prompt.to_string() }],
        max_tokens: 4096,
        use_search_grounding: false,
        use_cache: false,
        auto_accept: false,
    };
    let result = choice.backend.complete(opts).await
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(result.content)
}

fn is_retryable(msg: &str) -> bool {
    msg.contains("connection refused")
        || msg.contains("not running")
        || msg.contains("No models loaded")
        || msg.contains("empty response from local model")
        || msg.contains("no tool calls")
        || msg.contains("does not support ReAct")
        || msg.contains("failed to run")
        || msg.contains("timed out")
        || msg.contains("timeout")
        || msg.contains("Authentication")
        || msg.contains("auth")
        || msg.contains("401")
        || msg.contains("403")
        || msg.contains("CLI")
}

fn phase_header(n: u8, label: &str, backend_label: &str) {
    println!("\n\x1b[1;34m── Phase {n}: {label}\x1b[0m  \x1b[90m[{backend_label}]\x1b[0m");
}

fn read_line_stdin(prompt: &str) -> Result<String> {
    print!("{prompt}");
    io::stdout().flush()?;
    let mut buf = String::new();
    io::stdin().read_line(&mut buf)?;
    Ok(buf.trim_end_matches(['\n', '\r']).to_string())
}

// ── Pipeline entry point ──────────────────────────────────────────────────────

pub async fn run(
    query: &str,
    cfg: &config::LoadedConfig,
    cli: &CliDetection,
    cwd: &std::path::Path,
    ollama_url: &str,
    auto_accept: bool,
) -> Result<Artifacts> {
    let mut art = Artifacts { original_query: query.to_string(), ..Default::default() };

    // Print resolved cascade before starting — user can see which models will be used
    println!("\n\x1b[1mZedPlus Build Pipeline\x1b[0m — {query}");
    print_cascade_preview(cfg, cli, ollama_url);

    // ── Phase 0: Clarify ──────────────────────────────────────────────────
    phase_header(0, "Clarify requirements", "reasoning model");

    let clarify_prompt = format!(
        "You are a software architect. The user wants to build:\n\n{query}\n\n\
         Generate exactly 5 numbered questions that uncover the most critical unknowns: \
         scope, technology preferences, deployment target, must-have vs nice-to-have, \
         and key constraints. Output ONLY the 5 questions, one per line, nothing else."
    );
    let (questions_raw, r_label) = run_phase(PhaseKind::Reasoning, &clarify_prompt, cfg, cli, ollama_url).await?;
    println!("\x1b[90m[{r_label}]\x1b[0m\n");
    println!("{questions_raw}");

    println!("\n\x1b[33mAnswer each question (Enter to skip):\x1b[0m\n");
    for q in questions_raw.lines().filter(|l| !l.trim().is_empty()) {
        println!("{q}");
        let ans = read_line_stdin("> ")?;
        if !ans.trim().is_empty() {
            art.clarifications.push((q.trim().to_string(), ans));
        }
    }

    // ── Phase 1: Architecture ─────────────────────────────────────────────
    phase_header(1, "Architecture design", "reasoning model");

    let qa_block = if art.clarifications.is_empty() {
        "(no clarifications provided)".to_string()
    } else {
        art.clarifications.iter()
            .map(|(q, a)| format!("Q: {q}\nA: {a}"))
            .collect::<Vec<_>>()
            .join("\n\n")
    };

    let arch_prompt = format!(
        "You are a software architect.{NO_DELETE_RULE}\n\n\
         PROJECT: {query}\n\n\
         CLARIFICATIONS:\n{qa_block}\n\n\
         Produce a Markdown architecture document with these sections:\n\
         ## System Overview\n\
         ## Technology Stack (with rationale)\n\
         ## Component Breakdown\n\
         ## Directory and File Structure\n\
         ## Data Flow\n\
         ## Phased Build Order\n\
         ## Key Risks and Mitigations\n\
         Be specific and actionable."
    );
    let (arch, r_label) = run_phase(PhaseKind::Reasoning, &arch_prompt, cfg, cli, ollama_url).await?;
    println!("\x1b[90m[{r_label}]\x1b[0m");
    println!("{arch}");
    art.architecture = arch;

    // ── Phase 2: Build Plan ───────────────────────────────────────────────
    phase_header(2, "Detailed build plan", "planning model");

    let plan_prompt = format!(
        "ARCHITECTURE:\n{arch}\n\n\
         Create a numbered step-by-step build plan. Each step must include:\n\
         - File(s) to create or modify\n\
         - Key content or logic\n\
         - Dependencies (what must exist first)\n\
         Keep it concrete — a coding agent will follow this plan exactly.",
        arch = trim(&art.architecture, PLANNING_CTX_CHARS)
    );
    let (plan, l_label) = run_phase(PhaseKind::Planning, &plan_prompt, cfg, cli, ollama_url).await?;
    println!("\x1b[90m[{l_label}]\x1b[0m");
    println!("{plan}");
    art.build_plan = plan;

    // ── Phase 3: Verify Plan ──────────────────────────────────────────────
    phase_header(3, "Plan verification + risk check", "reasoning model");

    let verify_prompt = format!(
        "ARCHITECTURE:\n{arch}\n\nPROPOSED BUILD PLAN:\n{plan}\n\n\
         Review the build plan for:\n\
         1. Missing files or components\n\
         2. Dependency order violations\n\
         3. Technical risks or anti-patterns\n\
         4. Improvements\n\n\
         Then output the FINAL VERIFIED BUILD PLAN incorporating all fixes. \
         A coding agent will execute this plan directly — be explicit and complete.{no_del}",
        arch = trim(&art.architecture, REASONING_CTX_CHARS),
        plan = trim(&art.build_plan, REASONING_CTX_CHARS),
        no_del = NO_DELETE_RULE
    );
    let (verified, r_label) = run_phase(PhaseKind::Reasoning, &verify_prompt, cfg, cli, ollama_url).await?;
    println!("\x1b[90m[{r_label}]\x1b[0m");
    println!("{verified}");
    art.verified_plan = verified;

    // ── Phase 4: Build ────────────────────────────────────────────────────
    // Tries execution backends in order: local → best API.
    // Each is attempted; connection errors cause fallthrough; other errors stop.
    phase_header(4, "Build (agentic tool loop)", "execution model");

    let build_system = format!(
        "You are a coding agent executing a build plan step by step.{NO_DELETE_RULE}\n\n\
         ADDITIONAL BUILD RULES:\n\
         - Read existing files before modifying them\n\
         - Never delete or overwrite a file without first reading its current content\n\
         - If a file exists, modify it minimally — do not rewrite it entirely\n\
         - After writing each file, confirm it exists with a read\n\
         - CONTEXT BUDGET: read_file returns at most 100 lines. \
           Only read files you are about to modify. \
           Do NOT read every file in the project upfront — read one file, write it, then move on.\n\n\
         ARCHITECTURE:\n{arch}\n\nVERIFIED BUILD PLAN:\n{plan}",
        arch = trim(&art.architecture, BUILD_CTX_CHARS),
        plan = trim(&art.verified_plan, BUILD_CTX_CHARS),
    );

    art.build_summary = run_build_with_fallback(
        &build_system,
        cfg,
        cli,
        ollama_url,
        cwd,
        auto_accept,
    ).await;

    // ── Phase 5: Quality Check ────────────────────────────────────────────
    phase_header(5, "Quality check", "planning model");

    let file_tree = list_files(cwd);
    let qc_prompt = format!(
        "VERIFIED BUILD PLAN:\n{plan}\n\nFILES ON DISK:\n{tree}\n\n\
         BUILD SUMMARY:\n{summary}\n\n\
         Produce a QC Report. For each build plan step: Pass ✓ or Fail ✗.\n\
         Then list:\n\
         - Missing or incomplete items\n\
         - Obvious bugs or missing error handling\n\
         - What was done well{no_del}",
        plan    = trim(&art.verified_plan, PLANNING_CTX_CHARS),
        tree    = file_tree,
        summary = trim(&art.build_summary, 2_000),
        no_del  = NO_DELETE_RULE
    );
    let (qc, l_label) = run_phase(PhaseKind::Planning, &qc_prompt, cfg, cli, ollama_url).await?;
    println!("\x1b[90m[{l_label}]\x1b[0m");
    println!("{qc}");
    art.qc_report = qc;

    // ── Phase 6: Architecture Compliance ──────────────────────────────────
    phase_header(6, "Architecture compliance", "reasoning model");

    let archcheck_prompt = format!(
        "ORIGINAL ARCHITECTURE:\n{arch}\n\nQC REPORT:\n{qc}\n\n\
         Evaluate compliance:\n\
         1. Requirements met ✓ (list each)\n\
         2. Requirements NOT met ✗ (with explanation)\n\
         3. Blockers that must be fixed before the project can ship\n\
         4. Specific testing requirements for sign-off{no_del}",
        arch   = trim(&art.architecture, REASONING_CTX_CHARS),
        qc     = trim(&art.qc_report, REASONING_CTX_CHARS),
        no_del = NO_DELETE_RULE
    );
    let (archcheck, r_label) = run_phase(PhaseKind::Reasoning, &archcheck_prompt, cfg, cli, ollama_url).await?;
    println!("\x1b[90m[{r_label}]\x1b[0m");
    println!("{archcheck}");
    art.arch_check = archcheck;

    // ── Phase 7: Test Plan ────────────────────────────────────────────────
    phase_header(7, "Test plan", "planning model");

    let test_prompt = format!(
        "ARCHITECTURE COMPLIANCE REPORT:\n{check}\n\n\
         Produce a structured Test Plan:\n\
         1. Numbered test checklist (what to verify, expected outcome)\n\
         2. Shell commands to run existing test suites (if applicable)\n\
         3. Manual verification steps\n\
         Mark each item [ ] so it can be checked off as tests pass.",
        check = trim(&art.arch_check, PLANNING_CTX_CHARS)
    );
    let (test_plan, l_label) = run_phase(PhaseKind::Planning, &test_prompt, cfg, cli, ollama_url).await?;
    println!("\x1b[90m[{l_label}]\x1b[0m");
    println!("{test_plan}");
    art.test_plan = test_plan;

    // ── Phase 8: DevLog ───────────────────────────────────────────────────
    phase_header(8, "Writing devlog", "filesystem");
    art.devlog_path = Some(write_devlog(&art, cwd)?);
    if let Some(p) = &art.devlog_path {
        println!("\x1b[32m✓ DevLog:\x1b[0m {}", p.display());
    }

    println!("\n\x1b[1;32mBuild pipeline complete.\x1b[0m");
    Ok(art)
}

// ── Build phase with execution-backend fallback ───────────────────────────────

async fn run_build_with_fallback(
    system: &str,
    cfg: &config::LoadedConfig,
    cli: &CliDetection,
    ollama_url: &str,
    cwd: &std::path::Path,
    _auto_accept: bool,
) -> String {
    // Pipeline build phase is always autonomous — user authorized the full build via /build.
    let auto_accept = true;
    let candidates = selector::cascade(PhaseKind::Execution, cfg, cli, ollama_url);

    if candidates.is_empty() {
        return "Build skipped — no execution backend available. \
                Configure an API key or start Ollama.".to_string();
    }

    for choice in candidates {
        let label = choice.label.clone();
        println!("\x1b[90m[{label}]\x1b[0m");

        match agent::run(
            "Execute the verified build plan — create all required files.",
            Some(system),
            &[],
            choice.backend.as_ref(),
            &choice.model_id,
            8192,
            40,
            cwd,
            auto_accept,
            ollama_url,
            std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        ).await {
            Ok((summary, _, _, _)) => return summary,
            Err(e) => {
                let msg = e.to_string();
                if is_retryable(&msg) {
                    eprintln!("  \x1b[33m[fallback]\x1b[0m {label} unavailable ({msg:.80}), trying next...");
                } else {
                    // Non-retryable (e.g. tool error): report and stop
                    return format!("Build error ({label}): {msg}");
                }
            }
        }
    }

    "Build could not complete — all execution backends exhausted.".to_string()
}

// ── Startup preview ───────────────────────────────────────────────────────────

pub fn print_cascade_preview(
    cfg: &config::LoadedConfig,
    cli: &CliDetection,
    ollama_url: &str,
) {
    let r_label = selector::cascade(PhaseKind::Reasoning, cfg, cli, ollama_url)
        .into_iter().next().map(|c| c.label).unwrap_or_else(|| "none".into());
    let p_label = selector::cascade(PhaseKind::Planning, cfg, cli, ollama_url)
        .into_iter().next().map(|c| c.label).unwrap_or_else(|| "none".into());
    let e_label = selector::cascade(PhaseKind::Execution, cfg, cli, ollama_url)
        .into_iter().next().map(|c| c.label).unwrap_or_else(|| "none".into());
    println!("  Reasoning  → {r_label}");
    println!("  Planning   → {p_label}");
    println!("  Execution  → {e_label}");
    println!();
}

// ── DevLog ────────────────────────────────────────────────────────────────────

fn write_devlog(art: &Artifacts, cwd: &std::path::Path) -> Result<std::path::PathBuf> {
    let dir = cwd.join(".zedplus").join("devlogs");
    std::fs::create_dir_all(&dir)?;

    let ts  = Utc::now().format("%Y%m%d_%H%M%S");
    let slug = art.original_query.split_whitespace().take(5)
        .collect::<Vec<_>>().join("-").to_lowercase()
        .replace(|c: char| !c.is_alphanumeric() && c != '-', "");
    let path = dir.join(format!("{ts}_{slug}.md"));

    let mut doc = String::new();
    doc.push_str(&format!("# Dev Log: {}\n\n", art.original_query));
    doc.push_str(&format!("**Generated:** {}\n\n", Utc::now().format("%Y-%m-%d %H:%M:%S UTC")));
    doc.push_str("---\n\n");

    section(&mut doc, "Original Request", &art.original_query);

    if !art.clarifications.is_empty() {
        doc.push_str("## Clarifications\n\n");
        for (q, a) in &art.clarifications {
            doc.push_str(&format!("**Q:** {q}\n\n**A:** {a}\n\n"));
        }
    }

    section(&mut doc, "Architecture",             &art.architecture);
    section(&mut doc, "Build Plan",               &art.build_plan);
    section(&mut doc, "Verified Build Plan",      &art.verified_plan);
    section(&mut doc, "Build Summary",            &art.build_summary);
    section(&mut doc, "QC Report",                &art.qc_report);
    section(&mut doc, "Architecture Compliance",  &art.arch_check);
    section(&mut doc, "Test Plan",                &art.test_plan);

    doc.push_str("---\n\n");
    doc.push_str("*ZedPlus build pipeline devlog. Consult for maintenance, \
                  debugging, and future feature additions.*\n");

    std::fs::write(&path, &doc)?;
    Ok(path)
}

fn section(doc: &mut String, title: &str, body: &str) {
    doc.push_str(&format!("## {title}\n\n{}\n\n", body.trim()));
}

// ── File tree ─────────────────────────────────────────────────────────────────

fn list_files(cwd: &std::path::Path) -> String {
    let mut out = String::new();
    list_recursive(cwd, cwd, &mut out, 0);
    if out.is_empty() { "(none)".into() } else { out }
}

fn list_recursive(base: &std::path::Path, dir: &std::path::Path, out: &mut String, depth: usize) {
    const SKIP: &[&str] = &[".git", "target", "node_modules", ".next", "__pycache__", ".zedplus"];
    if depth > 4 { return; }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = entries.filter_map(|e| e.ok()).collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') && depth > 0 { continue; }
        if SKIP.contains(&name.as_str()) { continue; }
        let rel = path.strip_prefix(base)
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_else(|_| name.clone());
        if path.is_dir() {
            out.push_str(&format!("{rel}/\n"));
            list_recursive(base, &path, out, depth + 1);
        } else {
            out.push_str(&format!("{rel}\n"));
        }
    }
}
