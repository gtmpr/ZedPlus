use anyhow::Result;
use std::path::Path;

#[derive(Debug, Clone, Default)]
pub struct GitContext {
    /// Combined diff of staged + unstaged changes (git diff HEAD).
    pub diff: Option<String>,
    /// Paths of files that have changed relative to HEAD.
    pub changed_files: Vec<String>,
    /// Current branch name.
    pub branch: Option<String>,
}

impl GitContext {
    pub fn is_empty(&self) -> bool {
        self.diff.is_none() && self.changed_files.is_empty()
    }

    /// Format for injection into an AI prompt.
    pub fn to_prompt_context(&self) -> String {
        let mut parts = Vec::new();
        if let Some(branch) = &self.branch {
            parts.push(format!("Git branch: {branch}"));
        }
        if !self.changed_files.is_empty() {
            parts.push(format!("Changed files:\n{}", self.changed_files.join("\n")));
        }
        if let Some(diff) = &self.diff {
            let truncated = truncate_diff(diff, 8000);
            parts.push(format!("Git diff HEAD:\n```diff\n{truncated}\n```"));
        }
        parts.join("\n\n")
    }
}

/// Collect git context from the repository at `repo_path`.
/// Returns None if the path is not inside a git repository.
pub fn get_context(repo_path: &Path) -> Option<GitContext> {
    let repo = git2::Repository::discover(repo_path).ok()?;
    let mut ctx = GitContext::default();

    // Branch name
    ctx.branch = repo
        .head()
        .ok()
        .and_then(|h| h.shorthand().map(|s| s.to_string()));

    // Changed files (workdir vs HEAD)
    ctx.changed_files = collect_changed_files(&repo);

    // Diff text
    ctx.diff = collect_diff(&repo).ok();

    if ctx.is_empty() {
        None
    } else {
        Some(ctx)
    }
}

/// Returns true if `path` is inside a git repository.
pub fn is_git_repo(path: &Path) -> bool {
    git2::Repository::discover(path).is_ok()
}

/// Current branch name, or None.
pub fn current_branch(path: &Path) -> Option<String> {
    git2::Repository::discover(path)
        .ok()
        .and_then(|r| {
            r.head()
                .ok()
                .and_then(|h| h.shorthand().map(|s| s.to_string()))
        })
}

fn collect_changed_files(repo: &git2::Repository) -> Vec<String> {
    let mut opts = git2::StatusOptions::new();
    opts.include_untracked(false)
        .recurse_untracked_dirs(false)
        .include_ignored(false);

    repo.statuses(Some(&mut opts))
        .map(|statuses| {
            statuses
                .iter()
                .filter_map(|entry| entry.path().map(|p| p.to_string()))
                .collect()
        })
        .unwrap_or_default()
}

fn collect_diff(repo: &git2::Repository) -> Result<String> {
    // Diff workdir + index against HEAD
    let head_commit = repo.head()?.peel_to_commit()?;
    let head_tree = head_commit.tree()?;

    // Staged changes (index vs HEAD)
    let index = repo.index()?;
    let staged_diff = repo.diff_tree_to_index(Some(&head_tree), Some(&index), None)?;

    // Unstaged changes (workdir vs index)
    let workdir_diff = repo.diff_index_to_workdir(None, None)?;

    let mut output = String::new();

    collect_diff_text(&staged_diff, &mut output)?;
    collect_diff_text(&workdir_diff, &mut output)?;

    if output.is_empty() {
        Ok(String::new())
    } else {
        Ok(output)
    }
}

fn collect_diff_text(diff: &git2::Diff, out: &mut String) -> Result<()> {
    diff.print(git2::DiffFormat::Patch, |_delta, _hunk, line| {
        let prefix = match line.origin() {
            '+' => "+",
            '-' => "-",
            ' ' => " ",
            _ => "",
        };
        let content = std::str::from_utf8(line.content()).unwrap_or("");
        out.push_str(prefix);
        out.push_str(content);
        if !content.ends_with('\n') {
            out.push('\n');
        }
        true
    })?;
    Ok(())
}

fn truncate_diff(diff: &str, max_chars: usize) -> &str {
    if diff.len() <= max_chars {
        diff
    } else {
        let boundary = diff
            .char_indices()
            .map(|(i, _)| i)
            .take_while(|&i| i < max_chars)
            .last()
            .unwrap_or(max_chars);
        &diff[..boundary]
    }
}
