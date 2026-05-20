use anyhow::Result;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct CodeBlock {
    pub language: Option<String>,
    pub path: Option<PathBuf>,
    pub content: String,
}

/// Extract all fenced code blocks from an AI response.
/// Tries to infer file paths from preceding context and first-line comments.
pub fn extract_blocks(response: &str) -> Vec<CodeBlock> {
    let mut blocks = Vec::new();
    let lines: Vec<&str> = response.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i].trim_start();

        if !line.starts_with("```") && !line.starts_with("~~~") {
            i += 1;
            continue;
        }

        let fence = if line.starts_with("```") { "```" } else { "~~~" };
        let lang_tag = line.strip_prefix(fence).map(|s| s.trim());
        let language = lang_tag.filter(|s| !s.is_empty()).map(|s| s.to_string());

        // Look backwards for a file path hint in the preceding lines
        let path_from_context = find_preceding_path(&lines, i);

        // Gather block content
        let content_start = i + 1;
        let mut j = content_start;
        while j < lines.len()
            && !lines[j].trim_start().starts_with(fence)
        {
            j += 1;
        }
        let content_lines = &lines[content_start..j];

        // Check first line inside the block for an inline path comment
        let path_from_inline = content_lines
            .first()
            .and_then(|l| extract_inline_path(l, language.as_deref()));

        let path = path_from_context.or(path_from_inline);

        let content = content_lines.join("\n");

        if !content.trim().is_empty() {
            blocks.push(CodeBlock { language, path, content });
        }

        i = j + 1;
    }

    blocks
}

/// Inspect the 1-3 lines before the fence for a file path pattern.
fn find_preceding_path(lines: &[&str], fence_idx: usize) -> Option<PathBuf> {
    let start = fence_idx.saturating_sub(3);
    for &line in lines[start..fence_idx].iter().rev() {
        let trimmed = line.trim();
        if let Some(path) = parse_path_hint(trimmed) {
            return Some(path);
        }
    }
    None
}

/// Parse patterns like:
///   **src/main.rs**
///   `src/main.rs`
///   ### src/main.rs
///   src/main.rs:
///   File: src/main.rs
fn parse_path_hint(s: &str) -> Option<PathBuf> {
    // Strip markdown bold: **path**
    let s = if s.starts_with("**") && s.ends_with("**") && s.len() > 4 {
        &s[2..s.len() - 2]
    } else {
        s
    };
    // Strip markdown code: `path`
    let s = if s.starts_with('`') && s.ends_with('`') && s.len() > 2 {
        &s[1..s.len() - 1]
    } else {
        s
    };
    // Strip headers: ### path
    let s = s.trim_start_matches('#').trim();
    // Strip "File:" prefix
    let s = s.strip_prefix("File:").map(|r| r.trim()).unwrap_or(s);
    // Strip trailing colon
    let s = s.trim_end_matches(':');

    let s = s.trim();
    if looks_like_path(s) {
        Some(PathBuf::from(s))
    } else {
        None
    }
}

/// Extract a path from first-line comments like:
///   // src/main.rs
///   # src/config.py
///   -- src/schema.sql
fn extract_inline_path(line: &str, lang: Option<&str>) -> Option<PathBuf> {
    let trimmed = line.trim();
    let content = if trimmed.starts_with("//") {
        trimmed.strip_prefix("//").unwrap().trim()
    } else if trimmed.starts_with('#') {
        trimmed.strip_prefix('#').unwrap().trim()
    } else if trimmed.starts_with("--") {
        trimmed.strip_prefix("--").unwrap().trim()
    } else {
        return None;
    };
    // Strip "File:" prefix
    let content = content.strip_prefix("File:").map(|r| r.trim()).unwrap_or(content);
    if looks_like_path(content) {
        Some(PathBuf::from(content))
    } else {
        None
    }
}

fn looks_like_path(s: &str) -> bool {
    if s.is_empty() || s.len() > 200 {
        return false;
    }
    // Must contain at least one path separator or a known source extension
    let has_slash = s.contains('/') || s.contains('\\');
    let has_ext = matches!(
        s.rsplit_once('.').map(|(_, ext)| ext),
        Some("rs" | "py" | "js" | "ts" | "tsx" | "jsx" | "go" | "java" | "c" | "cpp"
            | "h" | "toml" | "yaml" | "yml" | "json" | "md" | "sql" | "sh" | "ps1"
            | "cs" | "rb" | "php" | "swift" | "kt" | "html" | "css" | "scss")
    );
    // No spaces (paths don't have spaces in typical code)
    let no_spaces = !s.contains(' ');
    (has_slash || has_ext) && no_spaces && !s.starts_with("http")
}

/// Return a simple line-diff display string: lines only in `new` are shown as `+ …`.
pub fn diff_summary(old: &str, new: &str) -> String {
    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();

    let added: Vec<&&str> = new_lines
        .iter()
        .filter(|l| !old_lines.contains(l))
        .collect();
    let removed: Vec<&&str> = old_lines
        .iter()
        .filter(|l| !new_lines.contains(l))
        .collect();

    let mut out = String::new();
    for l in &removed {
        out.push_str(&format!("  - {}\n", l));
    }
    for l in &added {
        out.push_str(&format!("  + {}\n", l));
    }
    out
}

#[derive(Debug)]
pub struct ApplyResult {
    pub path: PathBuf,
    pub created: bool,
}

/// Interactively apply a single code block.
/// Returns Ok(Some(result)) if applied, Ok(None) if skipped.
pub fn apply_interactive(block: &CodeBlock, cwd: &Path) -> Result<Option<ApplyResult>> {
    let path = match &block.path {
        Some(p) => p.clone(),
        None => {
            println!(
                "  ⚠  No file path detected in this code block (language: {}).",
                block.language.as_deref().unwrap_or("unknown")
            );
            println!("  Enter a relative path to write, or press Enter to skip:");
            let mut input = String::new();
            std::io::stdin().read_line(&mut input)?;
            let trimmed = input.trim();
            if trimmed.is_empty() {
                return Ok(None);
            }
            PathBuf::from(trimmed)
        }
    };

    let abs_path = if path.is_absolute() {
        path.clone()
    } else {
        cwd.join(&path)
    };

    let exists = abs_path.exists();
    println!("\n  File: {}", path.display());

    if exists {
        let existing = std::fs::read_to_string(&abs_path).unwrap_or_default();
        let diff = diff_summary(&existing, &block.content);
        if diff.is_empty() {
            println!("  (no changes)");
            return Ok(None);
        }
        println!("  Changes:");
        for line in diff.lines().take(20) {
            println!("{line}");
        }
        if diff.lines().count() > 20 {
            println!("  … ({} lines total)", block.content.lines().count());
        }
    } else {
        println!(
            "  (new file — {} lines)",
            block.content.lines().count()
        );
    }

    print!("  Apply? [y/N] ");
    use std::io::Write as _;
    std::io::stdout().flush()?;
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    if !input.trim().eq_ignore_ascii_case("y") {
        println!("  Skipped.");
        return Ok(None);
    }

    if let Some(parent) = abs_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&abs_path, &block.content)?;
    println!("  ✓ Written: {}", path.display());

    Ok(Some(ApplyResult { path, created: !exists }))
}

/// Extract blocks from a response and interactively apply each one.
/// Returns the number of files written.
pub fn apply_response(response: &str, cwd: &Path) -> Result<usize> {
    // Fire before_apply_change hook — load config to get hook command
    if let Ok(cfg) = crate::config::load(Some(cwd)) {
        let hooks = crate::hooks::HookRunner::new(&cfg.config.hooks);
        hooks.run_warn(crate::hooks::HookPoint::BeforeApplyChange);
    }

    let blocks = extract_blocks(response);

    // Filter to blocks that have a path or where language makes sense to apply
    let applicable: Vec<_> = blocks
        .iter()
        .filter(|b| {
            b.path.is_some()
                || matches!(
                    b.language.as_deref(),
                    Some(
                        "rust" | "python" | "javascript" | "typescript" | "go" | "java"
                            | "c" | "cpp" | "toml" | "yaml" | "json" | "sql" | "sh"
                            | "bash" | "powershell" | "css" | "html"
                    )
                )
        })
        .collect();

    if applicable.is_empty() {
        println!("  No applicable code blocks found in the last response.");
        return Ok(0);
    }

    println!(
        "  Found {} code block(s) to apply.",
        applicable.len()
    );

    let mut written = 0;
    for block in applicable {
        match apply_interactive(block, cwd)? {
            Some(_) => written += 1,
            None => {}
        }
    }

    if written > 0 {
        println!("\n  Applied {written} file(s).");
        if let Ok(cfg) = crate::config::load(Some(cwd)) {
            let hooks = crate::hooks::HookRunner::new(&cfg.config.hooks);
            hooks.run_warn(crate::hooks::HookPoint::AfterApplyChange);
        }
    }
    Ok(written)
}
