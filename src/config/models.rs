use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelCapabilities {
    pub provider: String,
    pub id: String,
    pub context_window: u32,
    #[serde(default)]
    pub supports_search_grounding: bool,
    #[serde(default)]
    pub supports_vision: bool,
    #[serde(default)]
    pub supports_pdf: bool,
    #[serde(default)]
    pub supports_cache: bool,
    #[serde(default)]
    pub supports_reasoning: bool,
    pub quality_tier: u8,
    pub speed_tier: u8,
    #[serde(default)]
    pub strengths: Vec<String>,
    #[serde(default)]
    pub weaknesses: Vec<String>,
    #[serde(default)]
    pub is_local: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModelRegistry {
    pub models: HashMap<String, ModelCapabilities>,
}

impl ModelRegistry {
    pub fn get(&self, key: &str) -> Option<&ModelCapabilities> {
        self.models.get(key)
    }

    pub fn models_with_strength(&self, strength: &str) -> Vec<(&String, &ModelCapabilities)> {
        self.models
            .iter()
            .filter(|(_, m)| m.strengths.iter().any(|s| s == strength))
            .collect()
    }
}

pub fn default_registry() -> ModelRegistry {
    let raw = include_str!("../../assets/models.toml");
    toml::from_str(raw).expect("bundled models.toml is valid")
}

pub fn load_or_default(path: &std::path::Path) -> Result<ModelRegistry> {
    if path.exists() {
        let raw = std::fs::read_to_string(path)?;
        Ok(toml::from_str(&raw)?)
    } else {
        Ok(default_registry())
    }
}
