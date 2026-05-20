use anyhow::Result;
use crate::{
    backends::{self, Backend, CompletionOptions, Message},
    config,
    setup::detector::CliDetection,
};

pub struct ArchitectPlan {
    pub files_to_modify: Vec<String>,
    pub instructions: String,
}

/// Phase 1: The Architect - Generates a structured plan without code.
pub async fn run_planning_phase(
    query: &str,
    repomap: &str,
    relevant_chunks: &str,
    backend: &dyn Backend,
    model_id: &str,
) -> Result<ArchitectPlan> {
    let system_prompt = format!(
        "You are a Senior Software Architect. Your task is to analyze a feature request \
        and provide a high-level technical plan.\n\n\
        CONTEXT:\n\
        ### Repository Structure\n{}\n\n\
        ### Relevant Code Snippets\n{}\n\n\
        RULES:\n\
        1. List every file that needs to be modified or created.\n\
        2. Describe the logic changes in plain text or pseudocode.\n\
        3. DO NOT output any actual source code or diffs.\n\
        4. Output your plan in XML format using <plan><file path=\"path/to/file\">Description</file></plan> tags.",
        repomap,
        relevant_chunks
    );

    let opts = CompletionOptions {
        model_id: model_id.to_string(),
        system: Some(system_prompt),
        messages: vec![Message {
            role: "user".to_string(),
            content: query.to_string(),
        }],
        max_tokens: 2048,
        use_search_grounding: false,
        use_cache: true,
        auto_accept: false,
    };

    let result = backend.complete(opts).await?;
    let content = result.content;

    // Parse XML plan
    let mut files = Vec::new();
    let mut instructions = String::new();

    let mut remaining = content.as_str();
    while let Some(start) = remaining.find("<file path=\"") {
        let after_path = &remaining[start + 12..];
        if let Some(end_path) = after_path.find("\">") {
            let path = &after_path[..end_path];
            let after_content = &after_path[end_path + 2..];
            if let Some(end_content) = after_content.find("</file>") {
                let desc = &after_content[..end_content];
                files.push(path.to_string());
                instructions.push_str(&format!("### File: {}\n{}\n\n", path, desc));
                remaining = &after_content[end_content + 7..];
            } else { break; }
        } else { break; }
    }

    if files.is_empty() {
        // Fallback: use the whole content if XML parsing failed
        instructions = content;
    }

    Ok(ArchitectPlan {
        files_to_modify: files,
        instructions,
    })
}

/// Phase 2: The Editor - Generates diffs based on the Architect's plan.
pub async fn run_editing_phase(
    plan: &ArchitectPlan,
    file_contents: &[(String, String)],
    backend: &dyn Backend,
    model_id: &str,
) -> Result<String> {
    let mut context = String::new();
    for (path, content) in file_contents {
        context.push_str(&format!("--- FILE: {} ---\n{}\n\n", path, content));
    }

    let system_prompt = format!(
        "You are an Expert Software Engineer. Your task is to implement the following technical plan.\n\n\
        PLAN:\n{}\n\n\
        FILES TO MODIFY:\n{}\n\n\
        RULES:\n\
        1. Output ONLY a unified diff or search/replace blocks.\n\
        2. Do not explain your changes.\n\
        3. Be precise and minimal.",
        plan.instructions,
        context
    );

    let opts = CompletionOptions {
        model_id: model_id.to_string(),
        system: Some(system_prompt),
        messages: vec![Message {
            role: "user".to_string(),
            content: "Implement the plan now.".to_string(),
        }],
        max_tokens: 4096,
        use_search_grounding: false,
        use_cache: false,
        auto_accept: false,
    };

    let result = backend.complete(opts).await?;
    Ok(result.content)
}
