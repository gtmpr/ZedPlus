use std::path::{Path, PathBuf};

/// Walk upward from `start` looking for a `ZEDPLUS.md` file.
pub fn find(start: &Path) -> Option<PathBuf> {
    let mut dir = start;
    loop {
        let candidate = dir.join("ZEDPLUS.md");
        if candidate.exists() {
            return Some(candidate);
        }
        match dir.parent() {
            Some(p) => dir = p,
            None => return None,
        }
    }
}

/// Load the nearest `ZEDPLUS.md` content, or None if not found.
pub fn load(start: &Path) -> Option<String> {
    let path = find(start)?;
    std::fs::read_to_string(path).ok()
}
