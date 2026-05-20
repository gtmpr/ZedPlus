// Phase 13c: Skill Packs
//
// Domain-specific routing and context injection.
//
// Skill packs are TOML files that customize:
// - Routing overrides for specific file patterns
// - System prompt injections (domain knowledge)
// - Recommended models for this domain
// - Context injection preferences
// 
// Built-in packs: rust-developer, python-ml, typescript-react, go-backend, etc.
// Users can create custom packs in ~/.config/zedplus/skills/

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillPack {
    pub name: String,
    pub description: String,
    pub version: String,
    
    /// Patterns that trigger this skill (e.g., ["*.rs", "src/**/*.rs"])
    #[serde(default)]
    pub file_patterns: Vec<String>,
    
    /// Routing overrides for this domain
    #[serde(default)]
    pub routing_overrides: RoutingOverrides,
    
    /// System prompt injection (domain-specific knowledge)
    #[serde(default)]
    pub system_prompt_injection: Option<String>,
    
    /// Models recommended for this domain
    #[serde(default)]
    pub recommended_models: Vec<String>,
    
    /// Context always included for files matching patterns
    #[serde(default)]
    pub always_include: Vec<PathBuf>,
    
    /// Keywords that trigger this skill in queries
    #[serde(default)]
    pub keywords: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RoutingOverrides {
    pub code_review: Option<String>,
    pub quick_completion: Option<String>,
    pub documentation: Option<String>,
    pub data_analysis: Option<String>,
    pub complex_reasoning: Option<String>,
}

impl SkillPack {
    /// Load a skill pack from a TOML file
    pub async fn load(path: impl AsRef<Path>) -> Result<Self> {
        let content = tokio::fs::read_to_string(path).await?;
        let pack = toml::from_str(&content)?;
        Ok(pack)
    }
    
    /// Save a skill pack to disk
    pub async fn save(&self, path: impl AsRef<Path>) -> Result<()> {
        let content = toml::to_string_pretty(self)?;
        tokio::fs::write(path, content).await?;
        Ok(())
    }
    
    /// Check if a file matches this pack's patterns (e.g. "*.rs", "src/**")
    pub fn matches(&self, file_path: &Path) -> bool {
        let path_str = file_path.to_string_lossy();
        self.file_patterns.iter().any(|pattern| {
            if let Some(ext) = pattern.strip_prefix("*.") {
                path_str.ends_with(&format!(".{ext}"))
            } else {
                path_str.contains(pattern.trim_start_matches('*').trim_end_matches('*'))
            }
        })
    }
    
    /// Check if a query contains keywords that trigger this pack
    pub fn matches_query(&self, query: &str) -> bool {
        let query_lower = query.to_lowercase();
        self.keywords
            .iter()
            .any(|kw| query_lower.contains(&kw.to_lowercase()))
    }
}

/// Registry of installed skill packs
#[derive(Debug)]
pub struct SkillRegistry {
    packs: HashMap<String, SkillPack>,
    skill_dir: PathBuf,
}

impl SkillRegistry {
    pub fn new(skill_dir: PathBuf) -> Self {
        SkillRegistry {
            packs: HashMap::new(),
            skill_dir,
        }
    }
    
    /// Load all skill packs from the skill directory
    pub async fn load_all(&mut self) -> Result<()> {
        if !self.skill_dir.exists() {
            tokio::fs::create_dir_all(&self.skill_dir).await?;
            return Ok(());
        }
        
        let mut entries = tokio::fs::read_dir(&self.skill_dir).await?;
        
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("toml") {
                if let Ok(pack) = SkillPack::load(&path).await {
                    self.packs.insert(pack.name.clone(), pack);
                }
            }
        }
        
        Ok(())
    }
    
    /// Get a pack by name
    pub fn get(&self, name: &str) -> Option<&SkillPack> {
        self.packs.get(name)
    }
    
    /// List all packs
    pub fn list(&self) -> Vec<&SkillPack> {
        self.packs.values().collect()
    }
    
    /// Register a new pack
    pub fn register(&mut self, pack: SkillPack) -> Result<()> {
        self.packs.insert(pack.name.clone(), pack);
        Ok(())
    }
    
    /// Find packs that match a file
    pub fn find_by_file(&self, file_path: &Path) -> Vec<&SkillPack> {
        self.packs
            .values()
            .filter(|p| p.matches(file_path))
            .collect()
    }
    
    /// Find packs that match a query
    pub fn find_by_query(&self, query: &str) -> Vec<&SkillPack> {
        self.packs
            .values()
            .filter(|p| p.matches_query(query))
            .collect()
    }
}

/// Suggestions for skill packs based on usage patterns
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillSuggestion {
    pub pack_name: String,
    pub reason: String,
    pub confidence: f32,
}

pub struct SkillSuggester;

impl SkillSuggester {
    /// Suggest skill packs based on:
    /// - File types in the project
    /// - Task types performed
    /// - Query keywords
    pub fn suggest(
        file_extensions: &[&str],
        recent_task_types: &[&str],
        _recent_queries: &[&str],
        available_packs: &[&SkillPack],
    ) -> Vec<SkillSuggestion> {
        let mut suggestions = Vec::new();
        
        for pack in available_packs {
            let mut confidence: f32 = 0.0;
            let mut reasons = Vec::new();
            
            // Match file extensions
            for ext in file_extensions {
                for pattern in &pack.file_patterns {
                    if pattern.contains(ext) {
                        confidence += 0.3;
                        reasons.push(format!("Matches .{} files", ext));
                        break;
                    }
                }
            }
            
            // Match task types
            for task_type in recent_task_types {
                if pack.keywords.iter().any(|kw| kw.contains(task_type)) {
                    confidence += 0.2;
                    reasons.push(format!("Suitable for {} tasks", task_type));
                }
            }
            
            if confidence > 0.0 && confidence <= 1.0 {
                suggestions.push(SkillSuggestion {
                    pack_name: pack.name.clone(),
                    reason: reasons.join("; "),
                    confidence: confidence.min(1.0),
                });
            }
        }
        
        suggestions.sort_by(|a, b| b.confidence.partial_cmp(&a.confidence).unwrap());
        suggestions
    }
}

/// Built-in skill pack templates
pub fn builtin_packs() -> HashMap<String, SkillPack> {
    let mut packs = HashMap::new();
    
    packs.insert(
        "rust-developer".to_string(),
        SkillPack {
            name: "rust-developer".to_string(),
            description: "Rust development with Cargo, testing, and safety".to_string(),
            version: "1.0".to_string(),
            file_patterns: vec!["*.rs".to_string(), "Cargo.toml".to_string()],
            routing_overrides: RoutingOverrides {
                code_review: Some("claude-opus".to_string()),
                quick_completion: Some("local".to_string()),
                documentation: Some("claude-haiku".to_string()),
                ..Default::default()
            },
            system_prompt_injection: Some(
                "You are an expert Rust developer. Prioritize memory safety, \
                 idiomatic Rust patterns, and performance. Run `cargo test` \
                 mentally before suggesting changes."
                    .to_string(),
            ),
            recommended_models: vec![
                "claude-opus".to_string(),
                "claude-sonnet".to_string(),
            ],
            always_include: vec![],
            keywords: vec!["rust".to_string(), "cargo".to_string(), "trait".to_string()],
        },
    );
    
    packs.insert(
        "python-ml".to_string(),
        SkillPack {
            name: "python-ml".to_string(),
            description: "Machine learning with Python (PyTorch, TensorFlow, etc.)".to_string(),
            version: "1.0".to_string(),
            file_patterns: vec!["*.py".to_string(), "requirements*.txt".to_string()],
            routing_overrides: RoutingOverrides {
                data_analysis: Some("claude-sonnet".to_string()),
                complex_reasoning: Some("claude-opus".to_string()),
                ..Default::default()
            },
            system_prompt_injection: Some(
                "You are an expert machine learning engineer. Focus on \
                 reproducibility, numerical stability, and best practices. \
                 Always include proper documentation and type hints."
                    .to_string(),
            ),
            recommended_models: vec![
                "claude-opus".to_string(),
                "claude-sonnet".to_string(),
            ],
            always_include: vec![],
            keywords: vec![
                "python".to_string(),
                "ml".to_string(),
                "neural".to_string(),
                "pytorch".to_string(),
            ],
        },
    );
    
    packs.insert(
        "typescript-react".to_string(),
        SkillPack {
            name: "typescript-react".to_string(),
            description: "TypeScript/React frontend development".to_string(),
            version: "1.0".to_string(),
            file_patterns: vec!["*.tsx".to_string(), "*.ts".to_string(), "package.json".to_string()],
            routing_overrides: RoutingOverrides {
                quick_completion: Some("gpt-4o-mini".to_string()),
                documentation: Some("claude-haiku".to_string()),
                ..Default::default()
            },
            system_prompt_injection: Some(
                "You are an expert TypeScript/React developer. Follow React best \
                 practices, use proper TypeScript typing, and ensure accessibility."
                    .to_string(),
            ),
            recommended_models: vec![
                "gpt-4o".to_string(),
                "claude-sonnet".to_string(),
            ],
            always_include: vec![],
            keywords: vec!["react".to_string(), "typescript".to_string()],
        },
    );
    
    packs
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_skill_pack_file_matching() {
        let pack = SkillPack {
            name: "rust".to_string(),
            description: "Rust skill".to_string(),
            version: "1.0".to_string(),
            file_patterns: vec!["*.rs".to_string()],
            routing_overrides: Default::default(),
            system_prompt_injection: None,
            recommended_models: vec![],
            always_include: vec![],
            keywords: vec![],
        };
        
        assert!(pack.matches(Path::new("main.rs")));
        assert!(!pack.matches(Path::new("main.py")));
    }
    
    #[test]
    fn test_skill_suggestion() {
        let packs = builtin_packs();
        let rust_pack = packs.get("rust-developer").unwrap();
        let suggestions = SkillSuggester::suggest(
            &["rs"],
            &["code_review"],
            &[],
            &[rust_pack],
        );
        
        assert!(!suggestions.is_empty());
        assert!(suggestions[0].confidence > 0.0);
    }
}
