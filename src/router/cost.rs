use crate::config::costs::CostsTable;

/// Estimate token count from raw text using a 4-chars-per-token heuristic.
pub fn estimate_tokens(text: &str) -> u32 {
    (text.len() as u32 / 4).max(1)
}

pub fn estimate_cost(model_id: &str, input_tokens: u32, output_tokens: u32, costs: &CostsTable) -> f64 {
    costs.cost_usd(model_id, input_tokens, output_tokens)
}
