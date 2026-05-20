use anyhow::Result;
use std::collections::HashMap;
use std::path::Path;

/// Walk a project directory and collect a summary for ZEDPLUS.md generation.
pub struct ProjectSummary {
    pub name: String,
    pub description: Option<String>,
    pub lang_lines: HashMap<String, usize>,
    pub key_files: Vec<String>,
    pub readme_excerpt: Option<String>,
    pub changelog_excerpt: Option<String>,
    pub tech_stack: Vec<String>,
}

impl ProjectSummary {
    pub fn scan(dir: &Path) -> Self {
        let name = dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("project")
            .to_string();

        let mut lang_lines: HashMap<String, usize> = HashMap::new();
        let mut key_files = Vec::new();
        let mut tech_stack = Vec::new();

        // Walk files (non-recursive for root, then src/)
        walk_for_stats(dir, dir, &mut lang_lines, &mut key_files, 0);

        // Detect tech stack from root manifest files
        if dir.join("Cargo.toml").exists() {
            tech_stack.push("Rust / Cargo".to_string());
            key_files.push("Cargo.toml".to_string());
        }
        if dir.join("package.json").exists() {
            tech_stack.push("Node.js / npm".to_string());
            key_files.push("package.json".to_string());
        }
        if dir.join("pyproject.toml").exists() || dir.join("setup.py").exists() {
            tech_stack.push("Python".to_string());
        }
        if dir.join("go.mod").exists() {
            tech_stack.push("Go".to_string());
            key_files.push("go.mod".to_string());
        }
        if dir.join("pom.xml").exists() {
            tech_stack.push("Java / Maven".to_string());
        }
        if dir.join("docker-compose.yml").exists() || dir.join("Dockerfile").exists() {
            tech_stack.push("Docker".to_string());
        }
        if dir.join(".github").exists() {
            tech_stack.push("GitHub Actions CI".to_string());
        }

        // Read README excerpt
        let readme_excerpt = read_excerpt(dir, "README.md", 30);

        // Read CHANGELOG excerpt
        let changelog_excerpt = read_excerpt(dir, "CHANGELOG.md", 15);

        // Description from Cargo.toml
        let description = read_cargo_description(dir);

        key_files.dedup();
        key_files.retain(|f| f != "ZEDPLUS.md");

        ProjectSummary {
            name,
            description,
            lang_lines,
            key_files,
            readme_excerpt,
            changelog_excerpt,
            tech_stack,
        }
    }

    /// Generate a ZEDPLUS.md without AI (structural summary only).
    pub fn render(&self, dir: &Path) -> String {
        let mut out = String::new();

        out.push_str(&format!("# ZEDPLUS.md — {}\n\n", self.name));
        out.push_str("_Auto-generated project context for ZedPlus AI routing._\n\n");

        if let Some(desc) = &self.description {
            out.push_str(&format!("## Overview\n\n{desc}\n\n"));
        }

        if !self.tech_stack.is_empty() {
            out.push_str("## Tech Stack\n\n");
            for item in &self.tech_stack {
                out.push_str(&format!("- {item}\n"));
            }
            out.push('\n');
        }

        // Language breakdown
        if !self.lang_lines.is_empty() {
            out.push_str("## Language Breakdown\n\n");
            let mut langs: Vec<_> = self.lang_lines.iter().collect();
            langs.sort_by(|a, b| b.1.cmp(a.1));
            let total: usize = langs.iter().map(|(_, n)| *n).sum();
            for (lang, lines) in langs.iter().take(10) {
                let pct = if total > 0 {
                    *lines * 100 / total
                } else {
                    0
                };
                out.push_str(&format!("- **{lang}**: {lines} lines ({pct}%)\n"));
            }
            out.push('\n');
        }

        // Key files
        if !self.key_files.is_empty() {
            out.push_str("## Key Files\n\n");
            for f in self.key_files.iter().take(20) {
                out.push_str(&format!("- `{f}`\n"));
            }
            out.push('\n');
        }

        // README excerpt
        if let Some(readme) = &self.readme_excerpt {
            out.push_str("## README Excerpt\n\n");
            out.push_str(readme);
            out.push_str("\n\n");
        }

        // Changelog excerpt
        if let Some(cl) = &self.changelog_excerpt {
            out.push_str("## Recent Changes\n\n");
            out.push_str(cl);
            out.push('\n');
        }

        // Directory structure (top-level dirs)
        let top_dirs = list_top_dirs(dir);
        if !top_dirs.is_empty() {
            out.push_str("## Directory Structure\n\n```\n");
            for d in &top_dirs {
                out.push_str(&format!("{d}/\n"));
            }
            out.push_str("```\n");
        }

        out
    }
}

fn walk_for_stats(
    root: &Path,
    dir: &Path,
    lang_lines: &mut HashMap<String, usize>,
    key_files: &mut Vec<String>,
    depth: usize,
) {
    if depth > 4 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else { return; };

    for entry in entries.flatten() {
        let path = entry.path();
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();

        // Skip hidden dirs and common build artifacts
        if name.starts_with('.') || matches!(name.as_str(), "target" | "node_modules" | "__pycache__" | "dist" | ".git") {
            continue;
        }

        if path.is_dir() {
            walk_for_stats(root, &path, lang_lines, key_files, depth + 1);
        } else if path.is_file() {
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_string();
            let lang = lang_for_ext(&ext);
            if let Some(lang) = lang {
                if let Ok(content) = std::fs::read_to_string(&path) {
                    *lang_lines.entry(lang.to_string()).or_default() +=
                        content.lines().count();
                }
            }
            // Track key source files
            if matches!(ext.as_str(), "rs" | "py" | "js" | "ts" | "go" | "java" | "c" | "cpp")
                && depth <= 2
            {
                if let Ok(rel) = path.strip_prefix(root) {
                    key_files.push(rel.to_string_lossy().replace('\\', "/"));
                }
            }
        }
    }
}

fn lang_for_ext(ext: &str) -> Option<&'static str> {
    match ext {
        "rs" => Some("Rust"),
        "py" => Some("Python"),
        "js" | "mjs" | "cjs" => Some("JavaScript"),
        "ts" | "tsx" => Some("TypeScript"),
        "go" => Some("Go"),
        "java" => Some("Java"),
        "c" | "h" => Some("C"),
        "cpp" | "cc" | "cxx" | "hpp" => Some("C++"),
        "cs" => Some("C#"),
        "rb" => Some("Ruby"),
        "php" => Some("PHP"),
        "swift" => Some("Swift"),
        "kt" | "kts" => Some("Kotlin"),
        "sql" => Some("SQL"),
        "sh" | "bash" => Some("Shell"),
        "ps1" | "psm1" => Some("PowerShell"),
        "html" | "htm" => Some("HTML"),
        "css" | "scss" | "sass" => Some("CSS"),
        "toml" => Some("TOML"),
        "yaml" | "yml" => Some("YAML"),
        "json" => Some("JSON"),
        "md" | "mdx" => Some("Markdown"),
        _ => None,
    }
}

fn read_excerpt(dir: &Path, filename: &str, max_lines: usize) -> Option<String> {
    let path = dir.join(filename);
    if !path.exists() {
        return None;
    }
    let content = std::fs::read_to_string(&path).ok()?;
    let excerpt: Vec<&str> = content.lines().take(max_lines).collect();
    Some(excerpt.join("\n"))
}

fn read_cargo_description(dir: &Path) -> Option<String> {
    let path = dir.join("Cargo.toml");
    if !path.exists() {
        return None;
    }
    let content = std::fs::read_to_string(&path).ok()?;
    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("description") {
            let val = rest.trim_start_matches([' ', '=', '"']).trim_end_matches('"');
            if !val.is_empty() {
                return Some(val.to_string());
            }
        }
    }
    None
}

fn list_top_dirs(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else { return vec![]; };
    let mut dirs: Vec<String> = entries
        .flatten()
        .filter(|e| {
            let p = e.path();
            let n = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            p.is_dir() && !n.starts_with('.') && n != "target" && n != "node_modules"
        })
        .map(|e| {
            e.path()
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string()
        })
        .collect();
    dirs.sort();
    dirs
}

/// Generate and write ZEDPLUS.md to the project directory.
pub fn generate(dir: &Path) -> Result<std::path::PathBuf> {
    let summary = ProjectSummary::scan(dir);
    let content = summary.render(dir);
    let path = dir.join("ZEDPLUS.md");
    std::fs::write(&path, &content)?;
    Ok(path)
}
