# Changelog

All notable changes to ZedPlus are documented here.
Format: [Semantic Versioning](https://semver.org). Increment rules:
- **patch** (0.x.**y**) — bug fixes, no new commands
- **minor** (0.**x**.0) — new commands or features, backward compatible
- **major** (**x**.0.0) — breaking changes to config schema or CLI interface

---

## [0.8.1] - 2026-05-20

### Added
- **Architect/Editor two-phase routing** (`run_architect_editor_turn`): On complex `CodeReview` or `ComplexReasoning` queries (keyword match or length ≥ threshold), ZedPlus automatically runs two calls — a high-quality architect model produces a structured implementation plan, then a fast editor model implements it. Both phases stream output inline with labeled headers (`── Architect phase ──` / `── Editor phase ──`). The combined plan + implementation is stored in session context.
- **`/explain` architect/editor cost split**: When architect/editor mode would activate, `/explain` now additionally shows both model IDs and the estimated cost split (`Arch/Edit: … → …`, `Split cost: $x.xx (arch) + $x.xx (edit)`).
- Architect/editor mode is bypassed when an explicit `@mention`, `/model`, `/local`, or `/cheap` override is active.
- `is_architect_split: true` flag in distiller entries produced by architect/editor turns.

### Version bump
`0.8.0` → `0.8.1`

---

## [0.8.0] - 2026-05-20

### Added
- **`zedplus config` command**: `--show` (pretty-prints active config), `--edit` (opens in `$EDITOR`/`VISUAL`/`notepad`), `--reset` (restores defaults), `--set KEY=VALUE` (sets ~15 config keys across routing, behavior, privacy, training, brainstorm sections). Persists to global `config.toml` immediately.
- **Unit tests**: 22 new tests across `router::classifier` (7 task types), `router::cost` (token estimation and cost calculation), `router::rules` (alias selection and fallback), `local_models` (parameter extraction and model scoring). All 25 tests pass.
- **README.md**: Full quickstart guide, install instructions (source + pre-built), REPL command table, one-shot usage, config, adaptive routing, distillation, session management, and default routing table.

### Version bump
`0.7.2` → `0.8.0`

---

## [0.7.2] - 2026-05-20

### Added
- **Codex routing**: `codex-mini` (→ `codex-mini-latest`) and `gpt-4-1` (→ `gpt-4.1`) added to `models.toml` with code-focused strengths. `gpt-4.1` is quality_tier 5 with a 1M-token context window.
- **`@codex` @-mention**: routes to OpenAI with `codex-mini-latest`; auto-completes in the REPL dropdown when Codex CLI is detected.
- **Codex CLI detection**: `detect_cli_tools()` now probes for the `codex` binary; announces at REPL startup when found.
- **Codex in failover chain**: `codex-mini` tried before `gpt-4o-mini` when OpenAI is a configured fallback provider.
- **`/ui` command**: `/ui` shows the active UI style; `/ui native|claude|gemini` changes it and persists to `~/.config/zedplus/config.toml` immediately.
- **First-run UI preference prompt**: on the very first launch (no config file yet), if Claude CLI or Gemini CLI is detected the REPL prompts the user to pick a preferred UI style before entering the main loop.

### Fixed
- **Ollama model-not-found error**: the 404 error message now includes the actual model name (`ollama pull gemma4:27b`) instead of the placeholder `ollama pull <model>`.
- **`@local/<name>` not-found feedback**: when a specific local model ID isn't in the discovered list, ZedPlus now prints the full discovered list and highlights close name matches (e.g. `did you mean: gemma4:27b?`) before falling back to Ollama directly.
- **Routing no longer defaults to Claude when alias is absent**: `select_alias()` now falls back to the best available model for the task type (by quality_tier + strengths match) when the configured alias isn't in the model registry. Eliminates implicit Claude bias for users who only have Gemini or OpenAI configured.

### Costs added
- `codex-mini-latest`: $1.50 / $6.00 per MTok (input/output)
- `gpt-4.1`: $2.00 / $8.00 per MTok (input/output)

### Version bump
`0.7.0` → `0.7.2`

---

## [0.7.0] - 2026-05-20

### Added
- **Phase 8 complete — Adaptive Routing**: `zedplus profile --optimize` and `--apply` now fully implement suggestion logic (≥5 consistent overrides per task type triggers a routing change suggestion written to `.zedplus.toml`).
- **Phase 8c — Brainstorm token accounting**: All four brainstorm strategies (`debate`, `red-team`, `perspectives`, `delphi`) now accumulate `input_tokens` / `output_tokens` across every `complete()` call and add them to session totals. Delphi sums across all refinement rounds.
- **Phase 8c — Auto-debate trigger**: New `[brainstorm] auto_debate_threshold_tokens` config field (default 0 = disabled). When set, `ComplexReasoning` queries whose estimated token count exceeds the threshold automatically trigger the configured brainstorm strategy instead of a single-model response.
- **Phase 8d — Multiple @mention routing**: Queries with 2+ `@provider` mentions (e.g. `@claude explain X @gemini summarize Y`) are now split at mention boundaries and each segment is routed to the specified provider independently, with per-segment labels printed to the terminal.
- **Phase 11 — Self-update system**: `platform/update.rs` implements full self-update: GitHub Releases API check, platform-specific asset matching (Windows zip, macOS tar.gz/dmg, Linux tar.gz), download with progress bar, binary extraction and install. `zedplus update --check` prints version comparison; `zedplus update` prompts for confirmation then installs. Windows stages as `zedplus_new.exe` with instructions since the running exe cannot be replaced in place.
- **UI style mimic**: New `[behavior] ui_style` config field (`native` / `claudecode` / `geminicli`). The REPL prompt changes to `◆ ` (Claude Code) or `⬡ ` (Gemini CLI) based on config. The `zedplus init` wizard now asks which CLI to mimic (step 3/8, shown dynamically based on detected CLIs).
- **Expanded CLI detection**: `detect_cli_tools()` now probes for `openai`, `groq`, `qwen`, and `aider` binaries alongside `claude` and `gemini`. Detected tools are printed in `zedplus init` step 2.
- **New service config fields**: `[services]` block gains `groq`, `openai_cli`, `qwen` boolean fields.
- **macOS tar.gz release artifacts**: Both macOS CI jobs now produce `.tar.gz` archives (in addition to `.dmg`) for use by the self-updater. Artifacts and release uploads updated accordingly.
- **Version bump**: `0.6.8` → `0.7.0`.

### Dependencies added
- `zip = { version = "0.6", default-features = false, features = ["deflate"] }`
- `flate2 = "1"`
- `tar = "0.4"`

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
