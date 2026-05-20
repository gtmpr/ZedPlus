# Changelog

All notable changes to ZedPlus are documented here.
Format: [Semantic Versioning](https://semver.org). Increment rules:
- **patch** (0.x.**y**) — bug fixes, no new commands
- **minor** (0.**x**.0) — new commands or features, backward compatible
- **major** (**x**.0.0) — breaking changes to config schema or CLI interface

---

## [0.1.1] - 2026-05-17

### Fixed
- **REPL double-typing** — replaced crossterm raw-mode line reader with `stdin().read_line()`. Raw mode toggling between each prompt caused Windows to echo characters twice (once via console ENABLE_ECHO_INPUT, once via our manual echo). Plain stdin read uses the OS line editor and has no echo conflict.
- **`zedplus auth` missing Skip option** — `[S] Skip` now appears alongside `[B] Browser` and `[M] Manual` for each provider in both `zedplus auth` and `zedplus init`.
- **`cargo install` not replacing binary** — `--force` flag is now required when reinstalling from source at the same version; documented in build workflow.

---

## [0.1.0] - 2026-05-16

### Added
- **Phase 1** — Foundation: CLI skeleton (`clap`), config system (`config.toml` + `.zedplus.toml` merge), SQLite schema (files, chunks, usage, sessions, train_jobs, bench_results), platform dirs/secrets (`keyring`)
- **Phase 1b** — Locale/time: system prompt prefix with current date, time, timezone, country, language
- **Phase 2** — Setup wizard: `zedplus init` (6-step wizard with device scan, routing plan preview, cost estimate), `zedplus auth` (browser-assist or manual key entry per provider)
- **Phase 3** — Code indexer: tree-sitter parser (Rust, JS/TS, Python, Go), Ollama `nomic-embed-text` embedder, SQLite chunk store with cosine similarity search, `notify` file watcher with 500ms debounce, git context (`git diff HEAD`)
- **Phase 4** — AI backends: Claude (Anthropic Messages API + SSE + prompt caching), Gemini (Google AI + Search grounding), OpenAI (Chat Completions + SSE), Ollama (local `/api/generate` streaming)
- **Phase 5** — Smart router: `router::route()` → `RoutingDecision`; task classifier (7 task types via regex/keyword heuristics); cost estimator; `--explain` flag shows task, model, reason, tokens, cost, cheapest alternative
- **Phase 6** — Distiller: Alpaca-format JSONL append (monthly files `YYYY-MM.jsonl`); `usage` SQLite row per call; negative signal detection (re-ask within 30s); `zedplus distill` export with `--task/--model/--since/--weighted` filters; `zedplus usage --today/--month`
- **Phase 7** — REPL: interactive loop with session context (`Vec<Message>`), slash commands (`/clear /usage /index /help /exit /explain /local /cheap /model /scope`), history summarization at 60k tokens, similarity search + git diff context injection, streaming output
- **Phase 7b** — Session persistence: `sessions` + `session_turns` SQLite tables; auto-resume prompt on TTY startup (inquire Confirm/Select); LLM session naming via fallback model (8s timeout, heuristic fallback); `zedplus resume`, `zedplus session list/rename/archive`
- **Phase 8** — Adaptive routing: `router::adaptive::analyze()` detects override patterns in usage table (≥5 consistent overrides per task type); `zedplus profile --optimize [--apply]` suggests and writes routing changes to `.zedplus.toml`
- **Phase 9** — Local model training: `distiller::trainer` module; `detect_trainer()` probes for Unsloth/Axolotl Python modules; `zedplus train --base <model> [--lora|--full] [--data file]` generates Python training script and streams output; `zedplus train --status` shows job history with auto-train suggestion; `zedplus model import <path|id> --name X` registers model in `model_registry`
