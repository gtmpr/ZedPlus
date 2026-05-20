use crate::config::costs::CostsTable;

/// Estimate token count from raw text using a 4-chars-per-token heuristic.
pub fn estimate_tokens(text: &str) -> u32 {
    (text.len() as u32 / 4).max(1)
}

pub fn estimate_cost(model_id: &str, input_tokens: u32, output_tokens: u32, costs: &CostsTable) -> f64 {
    costs.cost_usd(model_id, input_tokens, output_tokens)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::costs::{CostsTable, ModelPricing};
    use std::collections::HashMap;

    fn make_costs() -> CostsTable {
        let mut models = HashMap::new();
        models.insert("test-model".to_string(), ModelPricing {
            input_per_mtok: 1.0,
            output_per_mtok: 4.0,
        });
        CostsTable { models }
    }

    #[test]
    fn estimate_tokens_basic() {
        assert_eq!(estimate_tokens(""), 1);          // min 1
        assert_eq!(estimate_tokens("abcd"), 1);      // 4 chars = 1 token
        assert_eq!(estimate_tokens("a".repeat(400).as_str()), 100);
    }

    #[test]
    fn estimate_cost_known_model() {
        let costs = make_costs();
        // 1M input tokens × $1/MTok + 1M output tokens × $4/MTok = $5
        let c = estimate_cost("test-model", 1_000_000, 1_000_000, &costs);
        assert!((c - 5.0).abs() < 1e-9, "expected $5.00, got {c}");
    }

    #[test]
    fn estimate_cost_unknown_model_is_zero() {
        let costs = make_costs();
        let c = estimate_cost("no-such-model", 1_000, 1_000, &costs);
        assert_eq!(c, 0.0);
    }

    #[test]
    fn estimate_cost_partial_million() {
        let costs = make_costs();
        // 500k input @ $1/MTok = $0.50; 250k output @ $4/MTok = $1.00 → $1.50
        let c = estimate_cost("test-model", 500_000, 250_000, &costs);
        assert!((c - 1.5).abs() < 1e-9, "expected $1.50, got {c}");
    }
}
