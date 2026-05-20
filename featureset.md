# 🤖 ZedPlus Feature Set (v0.6.6)

ZedPlus is a smart AI routing CLI designed for local-first development, distillation, and autonomous model improvement. This document outlines the core features, their usage, and current limitations.

---

## 🏎️ 1. Smart Routing & Provider Management
ZedPlus automatically routes your queries to the most appropriate AI model based on the task type (e.g., coding, docs, web search).

- **How it works:** It uses a heuristic classifier to determine if you need a "Complex Reasoning" model (Claude/Gemini) or a "Quick Completion" model (Local Llama).
- **Usage:**
  - `zedplus ask "How do I use this?"`: Routes automatically.
  - `zedplus ask "..." --model claude`: Force a specific provider.
  - `@local`, `@cheap`, `@fast`: Inline mentions in the REPL to override routing on the fly.
- **Limitations:** Heuristic classification is currently keyword-based; complex multi-intent queries may occasionally route to the default fallback.

## 📂 2. Repository Mapping (Repomap)
A high-level architectural "skeleton" of your project is injected into every coding-related query.

- **How it works:** ZedPlus scans your indexed files and extracts top-level symbols (functions, classes, structs). It provides the AI with a map of your project structure so it doesn't "hallucinate" file paths.
- **Benefit:** Dramatically improves the accuracy of multi-file edits and "diff-only" responses.
- **Limitations:** Large projects have their repomap truncated at 4000 characters to preserve the model's context window.

## 🧠 3. Closed-Loop Local Training (Phase 9)
ZedPlus captures high-quality interactions from expensive cloud models and uses them to train your local models.

- **How it works:**
  - **Distillation:** Every query is saved to a monthly `.jsonl` file.
  - **Training:** Uses `unsloth` or `axolotl` (via Docker or Venv) to perform LoRA fine-tuning.
  - **Significance Heuristics:** ZedPlus auto-suggests training when it detects a "Significant Session" (e.g., >$1.00 cost or many files written).
- **Usage:** `zedplus train --base llama3 --bench`
- **Limitations:** Requires Docker or a specific Python environment. Fine-tuning is resource-intensive and requires a modern GPU (NVIDIA 8GB+ VRAM recommended).

## 📊 4. Empirical Benchmarking (Phase 10)
Verify if your trained local model is actually better than the baseline.

- **How it works:** Runs your new model against "gold standard" history and scores it on:
  - **Token F1:** Lexical overlap.
  - **Semantic Similarity:** Intent matching using embeddings.
  - **Format Accuracy:** Validates `<tool_call>` XML tags for agentic tasks.
- **Usage:** `zedplus bench --model my-new-model --baseline llama3`
- **Limitations:** Benchmarking requires an active Ollama connection for embedding comparisons.

## 🛠️ 5. Autonomous Agentic Mode
The AI can move beyond text and actually interact with your computer.

- **Tools:** `read_file`, `write_file`, `list_dir`, `run_command`, `search_semantic`, `git_status`, and `git_commit`.
- **Usage:** `/agent` toggle in the REPL or `zedplus ask "..." --agent`.
- **Limitations:** The agent currently lacks a secure "sandbox"; it runs commands with the same permissions as your user. **Use with caution.**

## ⌨️ 6. Advanced REPL
A custom-built interactive line editor optimized for developer workflows.

- **Navigation:** Full support for `Left/Right` arrows, `Home/End`, `Delete`, and character insertion.
- **Thinking Indicator:** A live spinner with a timer (e.g., `thinking (12s)…`) so you know if a CLI backend is hung or working.
- **Autocomplete:** Slash commands (`/help`, `/usage`, `/clear`) and `@mentions` have dropdown autocomplete.
- **Limitations:** Does not yet support multi-line input (Shift+Enter); use `zedplus ask` for large pasted blocks.

## 💾 7. Session Persistence
Your work is never lost. Sessions are automatically saved and organized by project and git branch.

- **Usage:** `zedplus resume` to pick up where you left off.
- **Naming:** ZedPlus uses a cheap AI call to generate a human-readable slug for your session (e.g., "fix-auth-bug").
- **Limitations:** Old sessions are preserved indefinitely in SQLite; use `zedplus session list` to manage them.

---

## 🏛️ 8. Competitive Strategy & Lessons Learned

ZedPlus doesn't exist in a vacuum. We actively study and integrate the best patterns from the industry to ensure we are the most robust "Local-First" agent.

### 1. From [Aider](https://aider.chat/) (The Benchmark)
*   **The Lesson:** The "Architect/Editor" pattern is the gold standard for accuracy.
*   **ZedPlus Implementation:** We've adopted the **Repomap** strategy (Phase 7c) and are moving toward a dual-model pipeline where a "Smart" model plans and a "Fast" model executes the diff (Phase 12c).

### 2. From [Cursor](https://www.cursor.com/) (The Experience)
*   **The Lesson:** Context is everything. "Context Pinning" allows users to force specific files into the AI's short-term memory.
*   **ZedPlus Implementation:** We are working on a `@pin` command for the REPL to ensure critical files are never dropped from the context window during complex tasks.

### 3. From [OpenDevin / Devin](https://github.com/OpenDevin/OpenDevin) (The Autonomous Agents)
*   **The Lesson:** Security is paramount. Autonomous agents need a "playpen."
*   **ZedPlus Implementation:** Future releases will prioritize **Docker Sandboxing** for the `run_command` tool to protect the user's host system from destructive AI mistakes.

### 4. From [LiteLLM](https://github.com/BerriAI/litellm) (The Routing Standard)
*   **The Lesson:** Failover must be invisible. If an API is down, the tool should instantly "virtualize" a fallback.
*   **ZedPlus Implementation:** Our `router/mod.rs` implements a robust **Fallback Chain** that moves from Cloud API → CLI Subscriptions → Local LLMs seamlessly.

### 5. From [Mysti](https://github.com/DeepMyst/Mysti) (The Multi-Agent Peer)
*   **The Lesson:** Multi-model debate can solve "hallucination" by forcing models to cross-check each other.
*   **ZedPlus Implementation:** We've built the `/debate` and `/brainstorm` commands (Phase 8c) to allow models to collaborate on high-complexity reasoning tasks.

## 📊 9. Empirical Model Ranking (Dynamic Leaderboard)
ZedPlus doesn't just trust marketing benchmarks; it ranks models based on their actual performance in *your* codebase.

- **How it works:** ZedPlus calculates a **Reliability Score (0.0 - 1.0)** using live signals:
    - **Test Pass Rate:** Code-modifying tasks that result in a green test suite increase a model's rank.
    - **User "Negative Signals":** If a user re-asks a similar query within 30 seconds of a model's response, that model receives a penalty.
    - **Override Frequency:** Manual overrides (e.g., typing `@gemini` when the router picked Claude) are tracked as "missed expectations" for the default model.
- **Usage:** `zedplus model rank`
- **Benefits:**
    - **Self-Optimizing:** Over time, the router learns which models are "hot" or "cold" for specific types of bugs in your project.
    - **Anti-Loop:** If a model fails a test twice, ZedPlus uses the leaderboard to swap in the next-best model for a "Fresh Eyes" perspective.

---

*Ready to build your next big idea?* 🚀
