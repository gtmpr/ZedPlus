use std::path::Path;
use tree_sitter::{Language, Node, Parser};

#[derive(Debug, Clone)]
pub struct Chunk {
    pub symbol: Option<String>,
    pub content: String,
    pub start_byte: usize,
    pub end_byte: usize,
}

/// Parse a source file and return meaningful chunks (functions, classes, etc.)
/// Falls back to whole-file chunking if no language grammar is available.
pub fn parse_file(path: &Path, content: &str) -> Vec<Chunk> {
    let Some(lang) = language_for_path(path) else {
        return whole_file_chunk(content);
    };

    let mut parser = Parser::new();
    if parser.set_language(&lang).is_err() {
        return whole_file_chunk(content);
    }

    let Some(tree) = parser.parse(content, None) else {
        return whole_file_chunk(content);
    };

    let root = tree.root_node();
    let source = content.as_bytes();
    let mut chunks = Vec::new();

    extract_chunks(&root, source, content, &mut chunks, true);

    // If tree-sitter found nothing interesting, fall back to whole-file
    if chunks.is_empty() {
        whole_file_chunk(content)
    } else {
        chunks
    }
}

fn extract_chunks(
    node: &Node,
    source: &[u8],
    content: &str,
    out: &mut Vec<Chunk>,
    top_level: bool,
) {
    let kind = node.kind();

    if is_interesting_node(kind) {
        let symbol = extract_name(node, source);
        let start = node.start_byte();
        let end = node.end_byte().min(content.len());
        let text = &content[start..end];

        // Skip trivially small nodes (e.g. empty impls)
        if text.trim().len() > 10 {
            out.extend(sliding_windows(symbol, text, start, end));
            return; // don't recurse into already-extracted nodes
        }
    }

    // Recurse into children at top level or inside container nodes
    let recurse = top_level
        || matches!(
            kind,
            "source_file"
                | "module"
                | "program"
                | "chunk"
                | "impl_item"
                | "class_body"
                | "block"
                | "export_statement"
                | "decorated_definition"
        );

    if recurse {
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i as u32) {
                extract_chunks(&child, source, content, out, false);
            }
        }
    }
}

fn is_interesting_node(kind: &str) -> bool {
    matches!(
        kind,
        // Rust
        "function_item"
            | "impl_item"
            | "struct_item"
            | "enum_item"
            | "trait_item"
            | "type_alias"
            | "mod_item"
            // Python
            | "function_definition"
            | "class_definition"
            | "decorated_definition"
            // JavaScript / TypeScript
            | "function_declaration"
            | "generator_function_declaration"
            | "class_declaration"
            | "method_definition"
            | "arrow_function"
            | "interface_declaration"
            | "type_alias_declaration"
            | "enum_declaration"
            // Go
            | "function_declaration"
            | "method_declaration"
            | "type_declaration"
    )
}

/// Extract the identifier/name from a node (first named child or "name" child).
fn extract_name(node: &Node, source: &[u8]) -> Option<String> {
    // Try "name" field first (tree-sitter convention)
    if let Some(name_node) = node.child_by_field_name("name") {
        return name_node.utf8_text(source).ok().map(|s| s.to_string());
    }
    // Try first named child that looks like an identifier
    for i in 0..node.named_child_count() {
        if let Some(child) = node.named_child(i as u32) {
            if matches!(child.kind(), "identifier" | "type_identifier" | "field_identifier") {
                return child.utf8_text(source).ok().map(|s| s.to_string());
            }
        }
    }
    None
}

const CHUNK_MAX: usize = 6_000;
const CHUNK_OVERLAP: usize = 500;

/// Split `text` into overlapping windows of at most `CHUNK_MAX` bytes.
/// Returns a single chunk when the text fits; otherwise emits multiple
/// overlapping windows so no content is lost.
fn sliding_windows(symbol: Option<String>, text: &str, base_start: usize, base_end: usize) -> Vec<Chunk> {
    if text.len() <= CHUNK_MAX {
        return vec![Chunk {
            symbol,
            content: text.to_string(),
            start_byte: base_start,
            end_byte: base_end,
        }];
    }

    let mut chunks = Vec::new();
    let mut offset = 0usize;
    let mut window_idx = 0usize;
    let step = CHUNK_MAX.saturating_sub(CHUNK_OVERLAP);

    loop {
        let end = find_char_boundary(text, (offset + CHUNK_MAX).min(text.len()));
        let win_symbol = if window_idx == 0 {
            symbol.clone()
        } else {
            Some(symbol.as_deref()
                .map(|s| format!("{s}[{window_idx}]"))
                .unwrap_or_else(|| format!("[window {window_idx}]")))
        };
        chunks.push(Chunk {
            symbol: win_symbol,
            content: text[offset..end].to_string(),
            start_byte: base_start + offset,
            end_byte: base_start + end,
        });
        if end >= text.len() {
            break;
        }
        let next = find_char_boundary(text, (offset + step).min(text.len()));
        if next <= offset {
            break;
        }
        offset = next;
        window_idx += 1;
    }

    chunks
}

fn whole_file_chunk(content: &str) -> Vec<Chunk> {
    if content.trim().is_empty() {
        return vec![];
    }
    sliding_windows(None, content, 0, content.len())
}

fn find_char_boundary(s: &str, mut idx: usize) -> usize {
    while idx > 0 && !s.is_char_boundary(idx) {
        idx -= 1;
    }
    idx
}

fn language_for_path(path: &Path) -> Option<Language> {
    match path.extension().and_then(|e| e.to_str()) {
        Some("rs") => Some(tree_sitter_rust::LANGUAGE.into()),
        Some("js" | "mjs" | "cjs" | "jsx") => Some(tree_sitter_javascript::LANGUAGE.into()),
        Some("ts") => Some(tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()),
        Some("tsx") => Some(tree_sitter_typescript::LANGUAGE_TSX.into()),
        Some("py" | "pyw") => Some(tree_sitter_python::LANGUAGE.into()),
        Some("go") => Some(tree_sitter_go::LANGUAGE.into()),
        _ => None,
    }
}

/// File extensions we bother indexing.
pub fn is_indexable(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("rs" | "js" | "mjs" | "cjs" | "jsx" | "ts" | "tsx" | "py" | "pyw" | "go"
            | "c" | "cpp" | "h" | "hpp" | "java" | "kt" | "swift" | "rb" | "php"
            | "sh" | "bash" | "zsh" | "fish" | "toml" | "yaml" | "yml" | "json"
            | "md" | "txt" | "sql")
    )
}

/// Files we never index: lockfiles, sourcemaps, minified bundles, and other
/// machine-generated artifacts that are large and semantically useless for search.
pub fn should_skip_file(path: &Path) -> bool {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    matches!(
        name,
        "package-lock.json" | "yarn.lock" | "pnpm-lock.yaml"
            | "npm-shrinkwrap.json" | "Cargo.lock" | "composer.lock"
            | "Gemfile.lock" | "poetry.lock" | "go.sum" | "go.work.sum"
            | "mix.lock" | "packages.lock.json" | "paket.lock"
    ) || name.ends_with(".min.js")
      || name.ends_with(".min.css")
      || name.ends_with(".map")
      || name.ends_with(".bundle.js")
}

/// Directories we always skip.
pub fn should_skip_dir(name: &str) -> bool {
    matches!(
        name,
        "target" | "node_modules" | ".git" | "__pycache__" | ".venv" | "venv"
            | "dist" | "build" | ".next" | ".nuxt" | "annotated" | "vendor" | "coverage"
            | ".pytest_cache" | ".mypy_cache" | ".ruff_cache" | ".claude" | ".zedplus"
    )
}
