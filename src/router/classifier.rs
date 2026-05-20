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
