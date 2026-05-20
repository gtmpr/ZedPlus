use crate::backends::ToolDef;
use serde_json::{json, Value};
use std::io::{self, Write as IoWrite};
use std::path::{Path, PathBuf};

// ── Tool definitions ──────────────────────────────────────────────────────────

pub fn all_tools() -> Vec<ToolDef> {
    vec![
        ToolDef {
            name: "read_file",
            description: "Read the contents of a file. Returns the file content with line numbers. Hard cap: 100 lines. Pass max_lines to read fewer (e.g. 30 to skim a file).",
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "The path to the file to read (relative to cwd or absolute)"
                    },
                    "max_lines": {
                        "type": "integer",
                        "description": "Maximum lines to return (default 100, hard cap 100). Use a smaller value to save context."
                    }
                },
                "required": ["path"]
            }),
        },
        ToolDef {
            name: "write_file",
            description: "Write content to a file. Shows a diff vs existing content and asks for confirmation before writing.",
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "The path to the file to write (relative to cwd or absolute)"
                    },
                    "content": {
                        "type": "string",
                        "description": "The content to write to the file"
                    }
                },
                "required": ["path", "content"]
            }),
        },
        ToolDef {
            name: "list_dir",
            description: "List the contents of a directory as a tree. Skips .git, target, and node_modules. Default depth is 3.",
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "The path to the directory to list (relative to cwd or absolute)"
                    },
                    "depth": {
                        "type": "integer",
                        "description": "Maximum depth to recurse (default 3)"
                    }
                },
                "required": ["path"]
            }),
        },
        ToolDef {
            name: "run_command",
            description: "Run a shell command. Shows the command and asks for confirmation before running. Captures and returns stdout and stderr.",
            parameters: json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "The shell command to run"
                    }
                },
                "required": ["command"]
            }),
        },
        ToolDef {
            name: "search_files",
            description: "Search for a pattern in files (case-insensitive). Returns file:line:content format, up to 50 results.",
            parameters: json!({
                "type": "object",
                "properties": {
                    "pattern": {
                        "type": "string",
                        "description": "The search pattern (case-insensitive regex or literal text)"
                    },
                    "directory": {
                        "type": "string",
                        "description": "Directory to search in (relative to cwd or absolute). Defaults to cwd."
                    }
                },
                "required": ["pattern"]
            }),
        },
        ToolDef {
            name: "glob_files",
            description: "Find files matching a glob pattern. Supports ** prefix and extension suffix patterns like **/*.rs or src/**/*.toml.",
            parameters: json!({
                "type": "object",
                "properties": {
                    "pattern": {
                        "type": "string",
                        "description": "Glob pattern to match files (e.g. **/*.rs, src/**/*.toml, *.json)"
                    }
                },
                "required": ["pattern"]
            }),
        },
        ToolDef {
            name: "search_semantic",
            description: "Semantic search of the indexed codebase using embeddings. Returns the most relevant code chunks without reading whole files. Use this before read_file to find where something is defined. Requires the codebase to have been indexed (zedplus index).",
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Natural language description of what you're looking for, e.g. 'cascade fallback logic' or 'how errors are handled in the agent loop'"
                    },
                    "top_k": {
                        "type": "integer",
                        "description": "Number of results to return (default 5, max 10)"
                    }
                },
                "required": ["query"]
            }),
        },
        ToolDef {
            name: "git_status",
            description: "Show current git repository state: branch, staged/unstaged changes, untracked files, and last 5 commits. Use before making commits or to understand what has changed.",
            parameters: json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        },
        ToolDef {
            name: "git_commit",
            description: "Stage all changes and create a git commit. Shows a diff summary and asks for confirmation before committing.",
            parameters: json!({
                "type": "object",
                "properties": {
                    "message": {
                        "type": "string",
                        "description": "The commit message"
                    },
                    "files": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Specific files to stage. If omitted, stages all tracked changes (git add -u)."
                    }
                },
                "required": ["message"]
            }),
        },
        ToolDef {
            name: "search_skills",
            description: "Search for available skill packs (domain knowledge) by keyword. Use this if you realize you need specialized expertise in a specific language, framework, or task (e.g. 'rust', 'react', 'security'). Returns a list of matching skill names and descriptions.",
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Keyword to search for (e.g. 'python', 'sql', 'performance')"
                    }
                },
                "required": ["query"]
            }),
        },
        ToolDef {
            name: "activate_skill",
            description: "Activate a specific skill pack by name to receive specialized system instructions for that domain. Returns the skill's instructions which will be added to your system prompt.",
            parameters: json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "The exact name of the skill pack to activate (get this from search_skills)"
                    }
                },
                "required": ["name"]
            }),
        },
    ]
}

// ── Tool execution ────────────────────────────────────────────────────────────

/// Execute a tool call. Returns (output, is_error).
/// When `auto_accept` is true, write_file and run_command skip the confirmation prompt.
pub fn execute(name: &str, input: &Value, cwd: &Path, auto_accept: bool) -> (String, bool) {
    match name {
        "read_file" => {
            let path_str = match input["path"].as_str() {
                Some(p) => p,
                None => return ("Missing 'path' argument".to_string(), true),
            };
            let max_lines = input["max_lines"].as_u64()
                .map(|n| (n as usize).min(100))
                .unwrap_or(100);
            tool_read_file(path_str, max_lines, cwd)
        }
        "write_file" => {
            let path_str = match input["path"].as_str() {
                Some(p) => p,
                None => return ("Missing 'path' argument".to_string(), true),
            };
            let content = match input["content"].as_str() {
                Some(c) => c,
                None => return ("Missing 'content' argument".to_string(), true),
            };
            tool_write_file(path_str, content, cwd, auto_accept)
        }
        "list_dir" => {
            let path_str = match input["path"].as_str() {
                Some(p) => p,
                None => return ("Missing 'path' argument".to_string(), true),
            };
            let depth = input["depth"].as_u64().unwrap_or(3) as usize;
            tool_list_dir(path_str, depth, cwd)
        }
        "run_command" => {
            let command = match input["command"].as_str() {
                Some(c) => c,
                None => return ("Missing 'command' argument".to_string(), true),
            };
            tool_run_command(command, cwd, auto_accept)
        }
        "search_files" => {
            let pattern = match input["pattern"].as_str() {
                Some(p) => p,
                None => return ("Missing 'pattern' argument".to_string(), true),
            };
            let directory = input["directory"].as_str();
            tool_search_files(pattern, directory, cwd)
        }
        "glob_files" => {
            let pattern = match input["pattern"].as_str() {
                Some(p) => p,
                None => return ("Missing 'pattern' argument".to_string(), true),
            };
            tool_glob_files(pattern, cwd)
        }
        "git_status" => tool_git_status(cwd),
        "git_commit" => {
            let message = match input["message"].as_str() {
                Some(m) => m,
                None => return ("Missing 'message' argument".to_string(), true),
            };
            let files: Vec<&str> = input["files"]
                .as_array()
                .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
                .unwrap_or_default();
            tool_git_commit(message, &files, cwd, auto_accept)
        }
        // search_semantic is handled async in agent/mod.rs before this dispatcher
        "search_semantic" => ("search_semantic requires async context".to_string(), true),
        _ => (format!("Unknown tool: {name}"), true),
    }
}

// ── Individual tool implementations ──────────────────────────────────────────

fn resolve_path(path_str: &str, cwd: &Path) -> PathBuf {
    let p = PathBuf::from(path_str);
    if p.is_absolute() {
        p
    } else {
        cwd.join(p)
    }
}

fn tool_read_file(path_str: &str, max_lines: usize, cwd: &Path) -> (String, bool) {
    let path = resolve_path(path_str, cwd);
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) => return (format!("Error reading {}: {}", path.display(), e), true),
    };

    let cap = max_lines.min(100);
    let lines: Vec<&str> = content.lines().collect();
    let truncated = lines.len() > cap;
    let shown_lines = if truncated { &lines[..cap] } else { &lines[..] };

    let mut out = String::new();
    for (i, line) in shown_lines.iter().enumerate() {
        out.push_str(&format!("{:>4}: {}\n", i + 1, line));
    }
    if truncated {
        out.push_str(&format!(
            "\n[... truncated: showing {cap} of {} lines. Use max_lines or read in sections ...]\n",
            lines.len()
        ));
    }

    (out, false)
}

fn tool_write_file(path_str: &str, content: &str, cwd: &Path, auto_accept: bool) -> (String, bool) {
    let path = resolve_path(path_str, cwd);

    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    let is_new = !path.exists();

    if is_new {
        println!("\n\x1b[33m[write_file]\x1b[0m Creating new file: {}", path.display());
    } else {
        println!("\n\x1b[33m[write_file]\x1b[0m Modifying: {}", path.display());
        print_simple_diff(&existing, content, path_str);
    }

    if auto_accept {
        println!("  \x1b[32m✓ auto-accepted\x1b[0m");
    } else if !confirm("Proceed? [Y/n]: ") {
        return ("Write cancelled by user".to_string(), false);
    }

    // Create parent directories if needed
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            return (format!("Error creating directories: {e}"), true);
        }
    }

    match std::fs::write(&path, content) {
        Ok(()) => (
            format!(
                "Successfully wrote {} bytes to {}",
                content.len(),
                path.display()
            ),
            false,
        ),
        Err(e) => (format!("Error writing {}: {}", path.display(), e), true),
    }
}

fn tool_list_dir(path_str: &str, depth: usize, cwd: &Path) -> (String, bool) {
    let path = resolve_path(path_str, cwd);
    if !path.exists() {
        return (format!("Path does not exist: {}", path.display()), true);
    }
    if !path.is_dir() {
        return (format!("Not a directory: {}", path.display()), true);
    }

    let mut out = String::new();
    out.push_str(&format!("{}/\n", path_str.trim_end_matches(['/', '\\'])));
    list_dir_recursive(&path, &mut out, 1, depth);
    (out, false)
}

const SKIP_DIRS: &[&str] = &[".git", "target", "node_modules", ".next", "dist", "__pycache__"];

fn list_dir_recursive(dir: &Path, out: &mut String, current_depth: usize, max_depth: usize) {
    if current_depth > max_depth {
        return;
    }
    let indent = "  ".repeat(current_depth);

    let mut entries: Vec<_> = match std::fs::read_dir(dir) {
        Ok(rd) => rd.filter_map(|e| e.ok()).collect(),
        Err(_) => return,
    };
    entries.sort_by_key(|e| {
        let is_file = e.path().is_file();
        let name = e.file_name().to_string_lossy().to_lowercase();
        (is_file as u8, name)
    });

    for entry in &entries {
        let file_name = entry.file_name().to_string_lossy().to_string();
        let path = entry.path();

        if path.is_dir() {
            if SKIP_DIRS.contains(&file_name.as_str()) {
                continue;
            }
            out.push_str(&format!("{}{}/\n", indent, file_name));
            list_dir_recursive(&path, out, current_depth + 1, max_depth);
        } else {
            out.push_str(&format!("{}{}\n", indent, file_name));
        }
    }
}

fn tool_run_command(command: &str, cwd: &Path, auto_accept: bool) -> (String, bool) {
    println!("\n\x1b[33m[run_command]\x1b[0m {command}");

    if auto_accept {
        println!("  \x1b[32m✓ auto-accepted\x1b[0m");
    } else if !confirm("Proceed? [Y/n]: ") {
        return ("Command cancelled by user".to_string(), false);
    }

    let output = if cfg!(windows) {
        std::process::Command::new("cmd")
            .args(["/C", command])
            .current_dir(cwd)
            .output()
    } else {
        std::process::Command::new("sh")
            .args(["-c", command])
            .current_dir(cwd)
            .output()
    };

    match output {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout).to_string();
            let stderr = String::from_utf8_lossy(&out.stderr).to_string();
            let exit_code = out.status.code().unwrap_or(-1);

            let mut result = String::new();
            if !stdout.is_empty() {
                result.push_str("stdout:\n");
                result.push_str(&stdout);
            }
            if !stderr.is_empty() {
                if !result.is_empty() {
                    result.push('\n');
                }
                result.push_str("stderr:\n");
                result.push_str(&stderr);
            }
            if result.is_empty() {
                result.push_str("(no output)");
            }
            result.push_str(&format!("\nexit code: {exit_code}"));

            let is_error = !out.status.success();
            (result, is_error)
        }
        Err(e) => (format!("Failed to run command: {e}"), true),
    }
}

fn tool_search_files(pattern: &str, directory: Option<&str>, cwd: &Path) -> (String, bool) {
    let search_dir = match directory {
        Some(d) => resolve_path(d, cwd),
        None => cwd.to_path_buf(),
    };

    if !search_dir.exists() {
        return (
            format!("Directory does not exist: {}", search_dir.display()),
            true,
        );
    }

    // Build a case-insensitive search using lowercased pattern
    let pattern_lower = pattern.to_lowercase();

    let mut results: Vec<String> = Vec::new();
    let mut searched = 0usize;

    search_dir_for_pattern(&search_dir, &pattern_lower, &mut results, &mut searched);

    if results.is_empty() {
        return (
            format!("No matches found for '{pattern}' in {}", search_dir.display()),
            false,
        );
    }

    let truncated = results.len() > 50;
    let shown = if truncated { &results[..50] } else { &results[..] };
    let mut out = shown.join("\n");
    if truncated {
        out.push_str(&format!(
            "\n\n[... {} more matches not shown ...]",
            results.len() - 50
        ));
    }
    (out, false)
}

fn search_dir_for_pattern(
    dir: &Path,
    pattern_lower: &str,
    results: &mut Vec<String>,
    searched: &mut usize,
) {
    if results.len() >= 50 {
        return;
    }

    let entries = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(_) => return,
    };

    for entry in entries.filter_map(|e| e.ok()) {
        if results.len() >= 50 {
            break;
        }
        let path = entry.path();
        let file_name = entry.file_name().to_string_lossy().to_string();

        if path.is_dir() {
            if SKIP_DIRS.contains(&file_name.as_str()) {
                continue;
            }
            search_dir_for_pattern(&path, pattern_lower, results, searched);
        } else if path.is_file() {
            *searched += 1;
            // Skip binary-likely files
            if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                if matches!(
                    ext,
                    "exe" | "dll" | "so" | "dylib" | "bin" | "png" | "jpg" | "jpeg" | "gif"
                        | "ico" | "pdf" | "zip" | "tar" | "gz" | "wasm" | "lock"
                ) {
                    continue;
                }
            }
            if let Ok(content) = std::fs::read_to_string(&path) {
                for (line_no, line) in content.lines().enumerate() {
                    if results.len() >= 50 {
                        break;
                    }
                    if line.to_lowercase().contains(pattern_lower) {
                        results.push(format!("{}:{}:{}", path.display(), line_no + 1, line.trim()));
                    }
                }
            }
        }
    }
}

fn tool_glob_files(pattern: &str, cwd: &Path) -> (String, bool) {
    let mut matches: Vec<String> = Vec::new();
    glob_walk(pattern, cwd, cwd, &mut matches);

    if matches.is_empty() {
        return (format!("No files matched pattern '{pattern}'"), false);
    }

    matches.sort();
    (matches.join("\n"), false)
}

fn glob_walk(pattern: &str, base: &Path, dir: &Path, results: &mut Vec<String>) {
    // Normalise the pattern separators
    let pattern_norm = pattern.replace('\\', "/");

    // Parse the pattern into components
    let parts: Vec<&str> = pattern_norm.split('/').collect();

    match_pattern_parts(&parts, base, dir, results, 0);
}

fn match_pattern_parts(
    parts: &[&str],
    base: &Path,
    current: &Path,
    results: &mut Vec<String>,
    part_idx: usize,
) {
    if part_idx >= parts.len() {
        return;
    }

    let segment = parts[part_idx];
    let is_last = part_idx == parts.len() - 1;

    if segment == "**" {
        // Match zero or more directories
        // First try matching the rest from this level (zero directories)
        if part_idx + 1 < parts.len() {
            match_pattern_parts(parts, base, current, results, part_idx + 1);
        }
        // Then descend into subdirectories
        if let Ok(entries) = std::fs::read_dir(current) {
            for entry in entries.filter_map(|e| e.ok()) {
                let path = entry.path();
                let name = entry.file_name().to_string_lossy().to_string();
                if SKIP_DIRS.contains(&name.as_str()) {
                    continue;
                }
                if path.is_dir() {
                    match_pattern_parts(parts, base, &path, results, part_idx);
                }
            }
        }
    } else if is_last {
        // Last segment — match files
        if let Ok(entries) = std::fs::read_dir(current) {
            for entry in entries.filter_map(|e| e.ok()) {
                let path = entry.path();
                let name = entry.file_name().to_string_lossy().to_string();
                if path.is_file() && glob_match_segment(segment, &name) {
                    // Return path relative to base
                    if let Ok(rel) = path.strip_prefix(base) {
                        results.push(rel.to_string_lossy().replace('\\', "/"));
                    } else {
                        results.push(path.to_string_lossy().replace('\\', "/"));
                    }
                }
            }
        }
    } else {
        // Intermediate segment — must match directories
        if let Ok(entries) = std::fs::read_dir(current) {
            for entry in entries.filter_map(|e| e.ok()) {
                let path = entry.path();
                let name = entry.file_name().to_string_lossy().to_string();
                if SKIP_DIRS.contains(&name.as_str()) {
                    continue;
                }
                if path.is_dir() && glob_match_segment(segment, &name) {
                    match_pattern_parts(parts, base, &path, results, part_idx + 1);
                }
            }
        }
    }
}

/// Simple glob segment matching: supports `*` as a wildcard, otherwise literal.
fn glob_match_segment(pattern: &str, name: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    // Pattern like "*.rs" — prefix * + suffix
    if pattern.starts_with('*') && !pattern[1..].contains('*') {
        let suffix = &pattern[1..];
        return name.ends_with(suffix);
    }
    // Pattern like "foo*" — prefix + suffix *
    if pattern.ends_with('*') && !pattern[..pattern.len() - 1].contains('*') {
        let prefix = &pattern[..pattern.len() - 1];
        return name.starts_with(prefix);
    }
    // Pattern like "foo*.rs" — prefix + * + suffix
    if let Some(star_pos) = pattern.find('*') {
        let prefix = &pattern[..star_pos];
        let suffix = &pattern[star_pos + 1..];
        if !suffix.contains('*') {
            return name.starts_with(prefix) && name.ends_with(suffix);
        }
    }
    // Literal match
    pattern == name
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Prompt the user for a y/n confirmation. Default is Yes.
fn confirm(prompt: &str) -> bool {
    print!("{prompt}");
    let _ = io::stdout().flush();

    let mut line = String::new();
    match io::stdin().read_line(&mut line) {
        Ok(_) => {
            let trimmed = line.trim().to_lowercase();
            trimmed.is_empty() || trimmed == "y" || trimmed == "yes"
        }
        Err(_) => false,
    }
}

fn tool_git_status(cwd: &Path) -> (String, bool) {
    let run = |args: &[&str]| -> String {
        std::process::Command::new("git")
            .args(args)
            .current_dir(cwd)
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default()
    };

    let branch   = run(&["branch", "--show-current"]);
    let status   = run(&["status", "--short"]);
    let log      = run(&["log", "--oneline", "-5"]);
    let stash    = run(&["stash", "list"]);

    let mut out = String::new();
    out.push_str(&format!("Branch: {branch}\n\n"));
    if status.is_empty() {
        out.push_str("Working tree: clean\n");
    } else {
        out.push_str("Changes:\n");
        out.push_str(&status);
        out.push('\n');
    }
    out.push_str("\nRecent commits:\n");
    out.push_str(if log.is_empty() { "(no commits yet)" } else { &log });
    if !stash.is_empty() {
        out.push_str("\n\nStash:\n");
        out.push_str(&stash);
    }
    (out, false)
}

fn tool_git_commit(message: &str, files: &[&str], cwd: &Path, auto_accept: bool) -> (String, bool) {
    // Stage specified files or all tracked changes
    let stage_args: Vec<&str> = if files.is_empty() {
        vec!["add", "-u"]
    } else {
        let mut args = vec!["add", "--"];
        args.extend_from_slice(files);
        args
    };

    // Show what will be committed
    let diff_stat = std::process::Command::new("git")
        .args(["diff", "--cached", "--stat"])
        .current_dir(cwd)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();

    // Stage first to show accurate diff
    let stage_output = std::process::Command::new("git")
        .args(&stage_args)
        .current_dir(cwd)
        .output();
    if let Err(e) = stage_output {
        return (format!("git add failed: {e}"), true);
    }

    let diff_stat_after = std::process::Command::new("git")
        .args(["diff", "--cached", "--stat"])
        .current_dir(cwd)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();

    let staged_info = if diff_stat_after.is_empty() { "nothing staged".to_string() } else { diff_stat_after };
    println!("\n\x1b[33mCommit: {message}\x1b[0m");
    println!("Staged:\n{staged_info}");

    if staged_info == "nothing staged" {
        return ("Nothing to commit — working tree clean".to_string(), false);
    }

    if !auto_accept && !confirm("Proceed with commit? [Y/n] ") {
        // Unstage changes we just staged
        let _ = std::process::Command::new("git").args(["reset", "HEAD"]).current_dir(cwd).output();
        return ("Commit cancelled".to_string(), false);
    }

    let commit = std::process::Command::new("git")
        .args(["commit", "-m", message])
        .current_dir(cwd)
        .output();

    match commit {
        Ok(o) if o.status.success() => {
            let out = String::from_utf8_lossy(&o.stdout).trim().to_string();
            (format!("Committed:\n{out}"), false)
        }
        Ok(o) => {
            let err = String::from_utf8_lossy(&o.stderr).trim().to_string();
            (format!("git commit failed: {err}"), true)
        }
        Err(e) => (format!("git commit error: {e}"), true),
    }
}

/// Print a simple line-based diff between old and new content.
fn print_simple_diff(old: &str, new: &str, label: &str) {
    println!("  --- {label} (existing)");
    println!("  +++ {label} (new)");

    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();

    // Very simple diff: show removed lines (-) and added lines (+)
    let max = old_lines.len().max(new_lines.len());
    let context = 3usize;
    let mut shown_context = false;
    let mut last_diff_line: Option<usize> = None;

    let mut changes: Vec<(usize, Option<&str>, Option<&str>)> = Vec::new();
    for i in 0..max {
        let old_line = old_lines.get(i).copied();
        let new_line = new_lines.get(i).copied();
        if old_line != new_line {
            changes.push((i, old_line, new_line));
        }
    }

    if changes.is_empty() {
        println!("  (no textual differences)");
        return;
    }

    // Only show first 30 changed lines to avoid flooding
    let shown = changes.len().min(30);
    for (i, old_line, new_line) in &changes[..shown] {
        println!("  @@ line {} @@", i + 1);
        if let Some(line) = old_line {
            println!("  \x1b[31m- {line}\x1b[0m");
        }
        if let Some(line) = new_line {
            println!("  \x1b[32m+ {line}\x1b[0m");
        }
    }

    let _ = (shown_context, last_diff_line, context);

    if changes.len() > 30 {
        println!("  ... ({} more changed lines)", changes.len() - 30);
    }
}
