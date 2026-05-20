use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelPricing {
    pub input_per_mtok: f64,
    pub output_per_mtok: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CostsTable {
    pub models: HashMap<String, ModelPricing>,
}

impl CostsTable {
    pub fn cost_usd(&self, model_key: &str, input_tokens: u32, output_tokens: u32) -> f64 {
        if let Some(p) = self.models.get(model_key) {
            let input_cost = (input_tokens as f64 / 1_000_000.0) * p.input_per_mtok;
            let output_cost = (output_tokens as f64 / 1_000_000.0) * p.output_per_mtok;
            input_cost + output_cost
        } else {
            0.0
        }
    }
}

pub fn default_costs() -> CostsTable {
    let mut models = HashMap::new();
    let entries = [
        ("claude-haiku-4-5", 0.80, 4.00),
        ("claude-sonnet-4-6", 3.00, 15.00),
        ("claude-opus-4-7", 15.00, 75.00),
        ("gemini-flash-2-5", 0.15, 0.60),
        ("gemini-pro-2-5", 1.25, 5.00),
        ("gpt-4o-mini", 0.15, 0.60),
        ("gpt-4o", 2.50, 10.00),
        ("local", 0.0, 0.0),
    ];
    for (name, inp, out) in entries {
        models.insert(name.to_string(), ModelPricing { input_per_mtok: inp, output_per_mtok: out });
    }
    CostsTable { models }
}

pub fn load_or_default(path: &std::path::Path) -> Result<CostsTable> {
    if path.exists() {
        let raw = std::fs::read_to_string(path)?;
        Ok(toml::from_str(&raw)?)
    } else {
        Ok(default_costs())
    }
}
