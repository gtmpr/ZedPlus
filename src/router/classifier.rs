#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TaskType {
    WebSearch,
    CodeReview,
    ComplexReasoning,
    DataAnalysis,
    Documentation,
    QuickCompletion,
    Fallback,
}

impl TaskType {
    pub fn as_str(&self) -> &'static str {
        match self {
            TaskType::WebSearch => "web_search",
            TaskType::CodeReview => "code_review",
            TaskType::ComplexReasoning => "complex_reasoning",
            TaskType::DataAnalysis => "data_analysis",
            TaskType::Documentation => "documentation",
            TaskType::QuickCompletion => "quick_completion",
            TaskType::Fallback => "fallback",
        }
    }
}

// Regex + keyword heuristics — fully implemented in Phase 5
pub fn classify(query: &str) -> TaskType {
    let q = query.to_lowercase();

    if q.contains("search") || q.contains("latest") || q.contains("news") || q.contains("current") {
        return TaskType::WebSearch;
    }
    if q.contains("review") || q.contains("audit") || q.contains("refactor") || q.contains("improve") {
        return TaskType::CodeReview;
    }
    // "explain why/how/the" → deep reasoning; bare "explain" → documentation
    if q.contains("design") || q.contains("architect") || q.contains("why")
        || (q.contains("explain") && (q.contains("why") || q.contains("trade") || q.contains("decision")))
    {
        return TaskType::ComplexReasoning;
    }
    if q.contains("analyz") || q.contains("csv") || q.contains("dataframe") || q.contains("plot") {
        return TaskType::DataAnalysis;
    }
    if q.contains("doc") || q.contains("readme") || q.contains("comment") || q.contains("docstring")
        || q.contains("explain") || q.contains("what does") || q.contains("what is")
        || q.contains("how does") || q.contains("how do")
    {
        return TaskType::Documentation;
    }
    if q.len() < 60 {
        return TaskType::QuickCompletion;
    }

    TaskType::Fallback
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn web_search_keywords() {
        assert_eq!(classify("search for rust async tutorials"), TaskType::WebSearch);
        assert_eq!(classify("what's the latest news on AI?"), TaskType::WebSearch);
        assert_eq!(classify("current status of the Rust 2024 edition"), TaskType::WebSearch);
    }

    #[test]
    fn code_review_keywords() {
        assert_eq!(classify("review this function for correctness"), TaskType::CodeReview);
        assert_eq!(classify("audit my authentication module"), TaskType::CodeReview);
        assert_eq!(classify("refactor this loop to be more idiomatic"), TaskType::CodeReview);
        assert_eq!(classify("improve the error handling in this code"), TaskType::CodeReview);
    }

    #[test]
    fn complex_reasoning_keywords() {
        assert_eq!(classify("design a distributed caching system"), TaskType::ComplexReasoning);
        assert_eq!(classify("architect the auth service"), TaskType::ComplexReasoning);
        assert_eq!(classify("why does async Rust need a runtime?"), TaskType::ComplexReasoning);
        assert_eq!(classify("explain the trade-offs between SQL and NoSQL"), TaskType::ComplexReasoning);
    }

    #[test]
    fn data_analysis_keywords() {
        assert_eq!(classify("analyze this csv file for trends"), TaskType::DataAnalysis);
        assert_eq!(classify("create a plot of the dataframe"), TaskType::DataAnalysis);
    }

    #[test]
    fn documentation_keywords() {
        assert_eq!(classify("write docstrings for this module"), TaskType::Documentation);
        assert_eq!(classify("explain what this function does"), TaskType::Documentation);
        assert_eq!(classify("what is the difference between Arc and Rc?"), TaskType::Documentation);
        assert_eq!(classify("how does the borrow checker work?"), TaskType::Documentation);
        assert_eq!(classify("generate a readme for this project"), TaskType::Documentation);
    }

    #[test]
    fn quick_completion_short_query() {
        // Short query with no keywords → QuickCompletion
        assert_eq!(classify("fix the typo"), TaskType::QuickCompletion);
        assert_eq!(classify("format this"), TaskType::QuickCompletion);
    }

    #[test]
    fn fallback_long_unclassified() {
        let long = "a".repeat(80);
        assert_eq!(classify(&long), TaskType::Fallback);
    }
}
