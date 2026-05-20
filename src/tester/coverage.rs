use std::path::{Path, PathBuf};

pub struct CoverageHint {
    pub has_tests: bool,
    pub test_function_count: usize,
    pub test_files: Vec<PathBuf>,
}

/// Heuristic coverage check — scan for test annotations without running tests.
pub fn check(cwd: &Path) -> CoverageHint {
    let mut test_files = Vec::new();
    let mut count = 0usize;
    scan_dir(cwd, cwd, &mut test_files, &mut count);
    CoverageHint {
        has_tests: !test_files.is_empty() || count > 0,
        test_function_count: count,
        test_files,
    }
}

fn scan_dir(root: &Path, dir: &Path, files: &mut Vec<PathBuf>, count: &mut usize) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            if name.starts_with('.') || matches!(name, "target" | "node_modules" | "dist" | ".git") {
                continue;
            }
        }
        if path.is_dir() {
            scan_dir(root, &path, files, count);
        } else {
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if matches!(ext, "rs" | "py" | "ts" | "js" | "go") {
                let content = match std::fs::read_to_string(&path) {
                    Ok(c) => c,
                    Err(_) => continue,
                };
                let n = count_test_annotations(&content);
                if n > 0 {
                    files.push(path.strip_prefix(root).unwrap_or(&path).to_path_buf());
                    *count += n;
                }
            }
        }
    }
}

fn count_test_annotations(content: &str) -> usize {
    ["#[test]", "def test_", "it(\"", "describe(\"", "func Test", "#[cfg(test)]"]
        .iter()
        .map(|m| content.matches(m).count())
        .sum()
}
