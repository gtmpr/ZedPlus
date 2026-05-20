use crate::backends::{Backend, CompletionOptions, Message};
use anyhow::Result;
use std::collections::HashSet;

#[derive(Debug, Clone, PartialEq)]
pub enum Strategy {
    Debate,
    RedTeam,
    Perspectives,
    Delphi,
}

impl Strategy {
    pub fn from_str(s: &str) -> Self {
        match s.to_ascii_lowercase().trim_matches('-').replace('-', "_").as_str() {
            "red_team" | "redteam" => Strategy::RedTeam,
            "perspectives" | "perspective" => Strategy::Perspectives,
            "delphi" => Strategy::Delphi,
            _ => Strategy::Debate,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Strategy::Debate => "debate",
            Strategy::RedTeam => "red-team",
            Strategy::Perspectives => "perspectives",
            Strategy::Delphi => "delphi",
        }
    }

    pub fn description(&self) -> &'static str {
        match self {
            Strategy::Debate => "Model A proposes → Model B critiques → show both",
            Strategy::RedTeam => "Model A proposes → Model B stress-tests → show both",
            Strategy::Perspectives => "Both models answer independently → show both",
            Strategy::Delphi => "Iterative refinement until convergence (max 3 rounds)",
        }
    }
}

pub struct BrainstormResult {
    pub strategy: Strategy,
    pub label_a: String,
    pub response_a: String,
    pub label_b: String,
    pub response_b: String,
    pub synthesis: Option<String>,
    pub rounds: u32,
    pub converged: bool,
    pub input_tokens: u32,
    pub output_tokens: u32,
}

const CONVERGENCE_THRESHOLD: f64 = 0.62;
const MAX_DELPHI_ROUNDS: u32 = 3;

fn jaccard(a: &str, b: &str) -> f64 {
    let wa: HashSet<&str> = a.split_whitespace().collect();
    let wb: HashSet<&str> = b.split_whitespace().collect();
    let inter = wa.intersection(&wb).count();
    let union = wa.union(&wb).count();
    if union == 0 { 1.0 } else { inter as f64 / union as f64 }
}

fn opts_with_prompt(system: Option<&str>, history: &[Message], prompt: &str) -> CompletionOptions {
    let mut messages = history.to_vec();
    messages.push(Message { role: "user".to_string(), content: prompt.to_string() });
    CompletionOptions {
        model_id: String::new(),
        system: system.map(String::from),
        messages,
        max_tokens: 2048,
        use_search_grounding: false,
        use_cache: false,
        auto_accept: false,
    }
}

pub async fn run(
    query: &str,
    system: Option<&str>,
    history: &[Message],
    backend_a: &dyn Backend,
    backend_b: &dyn Backend,
) -> Result<BrainstormResult> {
    // Strategy is carried in the context of the caller; this is the shared runner
    // that handles all four strategies based on the query prefix.
    // The caller strips the strategy prefix before calling here.
    run_perspectives(query, system, history, backend_a, backend_b).await
}


pub async fn run_strategy(
    query: &str,
    system: Option<&str>,
    history: &[Message],
    backend_a: &dyn Backend,
    backend_b: &dyn Backend,
    strategy: &Strategy,
) -> Result<BrainstormResult> {
    match strategy {
        Strategy::Debate => run_debate(query, system, history, backend_a, backend_b).await,
        Strategy::RedTeam => run_red_team(query, system, history, backend_a, backend_b).await,
        Strategy::Perspectives => run_perspectives(query, system, history, backend_a, backend_b).await,
        Strategy::Delphi => run_delphi(query, system, history, backend_a, backend_b).await,
    }
}

async fn run_perspectives(
    query: &str,
    system: Option<&str>,
    history: &[Message],
    backend_a: &dyn Backend,
    backend_b: &dyn Backend,
) -> Result<BrainstormResult> {
    eprintln!("\x1b[90m[brainstorm] perspectives — querying both models independently…\x1b[0m");

    let (ra, rb) = tokio::join!(
        backend_a.complete(opts_with_prompt(system, history, query)),
        backend_b.complete(opts_with_prompt(system, history, query)),
    );

    let ra = ra.map_err(|e| anyhow::anyhow!("{e}"))?;
    let rb = rb.map_err(|e| anyhow::anyhow!("{e}"))?;

    let input_tokens = ra.input_tokens + rb.input_tokens;
    let output_tokens = ra.output_tokens + rb.output_tokens;

    let sim = jaccard(&ra.content, &rb.content);
    let converged = sim >= CONVERGENCE_THRESHOLD;

    Ok(BrainstormResult {
        strategy: Strategy::Perspectives,
        label_a: backend_a.name().to_string(),
        response_a: ra.content,
        label_b: backend_b.name().to_string(),
        response_b: rb.content,
        synthesis: if converged { Some("[Models converged on the same core answer.]".to_string()) } else { None },
        rounds: 1,
        converged,
        input_tokens,
        output_tokens,
    })
}

async fn run_debate(
    query: &str,
    system: Option<&str>,
    history: &[Message],
    backend_a: &dyn Backend,
    backend_b: &dyn Backend,
) -> Result<BrainstormResult> {
    eprintln!("\x1b[90m[brainstorm] debate — Model A proposes…\x1b[0m");

    let ra = backend_a
        .complete(opts_with_prompt(system, history, query))
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    let critique_prompt = format!(
        "Here is another model's answer to the question: \"{query}\"\n\n\
         --- Their answer ---\n{}\n--- End ---\n\n\
         Do you agree, disagree, or see important missing nuance? Be specific and direct. \
         State what is correct, what is wrong or incomplete, and what you would add or change.",
        ra.content
    );

    eprintln!("\x1b[90m[brainstorm] debate — Model B critiques…\x1b[0m");

    let rb = backend_b
        .complete(opts_with_prompt(system, history, &critique_prompt))
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    let input_tokens = ra.input_tokens + rb.input_tokens;
    let output_tokens = ra.output_tokens + rb.output_tokens;

    Ok(BrainstormResult {
        strategy: Strategy::Debate,
        label_a: format!("{} (proposal)", backend_a.name()),
        response_a: ra.content,
        label_b: format!("{} (critique)", backend_b.name()),
        response_b: rb.content,
        synthesis: None,
        rounds: 1,
        converged: false,
        input_tokens,
        output_tokens,
    })
}

async fn run_red_team(
    query: &str,
    system: Option<&str>,
    history: &[Message],
    backend_a: &dyn Backend,
    backend_b: &dyn Backend,
) -> Result<BrainstormResult> {
    eprintln!("\x1b[90m[brainstorm] red-team — Model A proposes…\x1b[0m");

    let ra = backend_a
        .complete(opts_with_prompt(system, history, query))
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    let red_team_prompt = format!(
        "A model proposed this answer to: \"{query}\"\n\n\
         --- Proposal ---\n{}\n--- End ---\n\n\
         Play devil's advocate. Identify the weaknesses, hidden risks, edge cases it misses, \
         faulty assumptions, and scenarios where this approach breaks down. Be rigorous and specific.",
        ra.content
    );

    eprintln!("\x1b[90m[brainstorm] red-team — Model B stress-tests…\x1b[0m");

    let rb = backend_b
        .complete(opts_with_prompt(system, history, &red_team_prompt))
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    let input_tokens = ra.input_tokens + rb.input_tokens;
    let output_tokens = ra.output_tokens + rb.output_tokens;

    Ok(BrainstormResult {
        strategy: Strategy::RedTeam,
        label_a: format!("{} (proposal)", backend_a.name()),
        response_a: ra.content,
        label_b: format!("{} (red-team)", backend_b.name()),
        response_b: rb.content,
        synthesis: None,
        rounds: 1,
        converged: false,
        input_tokens,
        output_tokens,
    })
}

async fn run_delphi(
    query: &str,
    system: Option<&str>,
    history: &[Message],
    backend_a: &dyn Backend,
    backend_b: &dyn Backend,
) -> Result<BrainstormResult> {
    let mut rounds = 0u32;
    let mut converged = false;
    let mut total_input_tokens: u32 = 0;
    let mut total_output_tokens: u32 = 0;

    eprintln!("\x1b[90m[brainstorm] delphi — round 1 (independent)…\x1b[0m");

    let (ra, rb) = tokio::join!(
        backend_a.complete(opts_with_prompt(system, history, query)),
        backend_b.complete(opts_with_prompt(system, history, query)),
    );
    let ra = ra.map_err(|e| anyhow::anyhow!("{e}"))?;
    let rb = rb.map_err(|e| anyhow::anyhow!("{e}"))?;
    total_input_tokens += ra.input_tokens + rb.input_tokens;
    total_output_tokens += ra.output_tokens + rb.output_tokens;
    let mut ra_text = ra.content;
    let mut rb_text = rb.content;
    rounds += 1;

    while rounds < MAX_DELPHI_ROUNDS {
        let sim = jaccard(&ra_text, &rb_text);
        if sim >= CONVERGENCE_THRESHOLD {
            converged = true;
            break;
        }

        eprintln!(
            "\x1b[90m[brainstorm] delphi — round {} (refine, similarity={:.2})…\x1b[0m",
            rounds + 1, sim
        );

        let refine_a = format!(
            "You answered: \"{query}\"\n\nYour answer: {ra_text}\n\n\
             Another model answered:\n{rb_text}\n\n\
             Considering their perspective, refine your answer. Keep what you believe is correct, \
             revise what you think they improved on, add anything still missing."
        );
        let refine_b = format!(
            "You answered: \"{query}\"\n\nYour answer: {rb_text}\n\n\
             Another model answered:\n{ra_text}\n\n\
             Considering their perspective, refine your answer. Keep what you believe is correct, \
             revise what you think they improved on, add anything still missing."
        );

        let (na, nb) = tokio::join!(
            backend_a.complete(opts_with_prompt(system, &[], &refine_a)),
            backend_b.complete(opts_with_prompt(system, &[], &refine_b)),
        );
        let na = na.map_err(|e| anyhow::anyhow!("{e}"))?;
        let nb = nb.map_err(|e| anyhow::anyhow!("{e}"))?;
        total_input_tokens += na.input_tokens + nb.input_tokens;
        total_output_tokens += na.output_tokens + nb.output_tokens;
        ra_text = na.content;
        rb_text = nb.content;
        rounds += 1;
    }

    if !converged {
        let sim = jaccard(&ra_text, &rb_text);
        converged = sim >= CONVERGENCE_THRESHOLD;
    }

    let synthesis = if converged {
        Some("[Delphi converged — models reached consensus.]".to_string())
    } else {
        Some(format!(
            "[Delphi ended after {rounds} rounds without full convergence — \
             review both perspectives and synthesize manually.]"
        ))
    };

    Ok(BrainstormResult {
        strategy: Strategy::Delphi,
        label_a: backend_a.name().to_string(),
        response_a: ra_text,
        label_b: backend_b.name().to_string(),
        response_b: rb_text,
        synthesis,
        rounds,
        converged,
        input_tokens: total_input_tokens,
        output_tokens: total_output_tokens,
    })
}
