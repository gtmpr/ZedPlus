# ZedPlus Build Rollout Plan

Phased build order — each phase delivers a runnable, testable increment. Later phases depend on earlier ones. Time estimates assume a single focused developer; adjust for team size.

---

## ZedPlus vs Mysti — Competitive Analysis

[Mysti](https://github.com/DeepMyst/Mysti) is the closest public project to ZedPlus. TypeScript VS Code extension, 1k+ stars, Apache 2.0. Core idea: instead of one model answering, multiple models collaborate, debate, and synthesize.

### Where Mysti wins
| Mysti capability | ZedPlus status | Action |
|---|---|---|
| Multi-agent debate (4 strategies: Debate/Red-Team/Perspectives/Delphi) | ✅ Built | Phase 8c complete |
| Convergence detection (Jaccard similarity, auto-exits when models agree) | ✅ Built | Part of Phase 8c |
| 8 developer personas (Architect, Debugger, Security, Perf, Teacher, Reviewer, Tester, DevOps) | ✅ Built | Phase 8b complete |
| @-mention inline routing (`@claude`, `@gemini`, `@local`, `@cheap`, `@fast`) | ✅ Built | Phase 8d complete |
| Autonomous mode with 3 safety levels (Conservative/Balanced/Aggressive) | Partial (`/accept` only) | Extend Phase 7 |
| 360+ automated tests | None | Add in Phase 12 |
| Intelligent plan detection (surfaces multiple impl approaches for selection) | Partial (pipeline /build) | Extend Phase 12c |

### Where ZedPlus wins
| ZedPlus capability | Mysti status |
|---|---|
| IDE-agnostic (terminal binary, no VS Code dependency) | VS Code only |
| Codebase indexing + semantic search (tree-sitter + nomic-embed-text) | Not built |
| LoRA fine-tuning pipeline from usage data | Not built |
| Distillation → training → benchmarking loop | Not built |
| Smart automatic task classifier (no @-mention required) | @-mention only |
| Adaptive routing (learns from override patterns) | Not built |
| Session persistence + resume by project+branch | Not built |
| Per-phase pipeline (Reasoning/Planning/Execution with different models) | Not built |
| Lite-task routing (local model first for docs/quick queries) | Not built |
| CLI subscription priority (Claude/Gemini CLI before API) | Not built |

### Key insight
Mysti optimises the **answer quality** for a single query by using multiple models to debate it. ZedPlus optimises the **cost and continuity** of an entire project session by smart routing, local-first execution, and training a private model from accumulated history. They're complementary, not competing — Mysti is a debate club, ZedPlus is an adaptive team that gets cheaper over time.

---

## Phase 1 — Foundation ✅ COMPLETE

**Goal:** Runnable binary, config system, platform plumbing.

- [x] `cargo new zedplus --bin`, set up workspace with `Cargo.toml`
- [x] Add core dependencies: `clap`, `tokio`, `serde`/`serde_json`/`toml`, `dirs`, `rusqlite`, `anyhow`, `tracing`
- [x] `platform/dirs.rs` — resolve config and data dirs cross-platform using `dirs` crate
- [x] `platform/secrets.rs` — `keyring` wrapper (store/retrieve/delete API keys from OS keychain)
- [x] `config/schema.rs` — serde structs for `config.toml` and `.zedplus.toml`
- [x] `config/mod.rs` — load + merge global config with project config (project wins)
- [x] `config/costs.rs` — load `costs.toml` pricing table
- [x] `config/models.rs` — load `models.toml` capability registry
- [x] SQLite init: create all tables on first run (`files`, `chunks`, `usage`, `bench_results`, `model_registry`, `train_jobs`)
- [x] CLI skeleton: `clap` with all subcommand stubs (`init`, `index`, `ask`, `search`, `usage`, `distill`, `train`, `bench`, `model`, `profile`, `config`, `update`, `clear`)

**Milestone:** `zedplus --help` prints full command tree. Config loads from disk. Keychain read/write works on all three platforms.

---

## Phase 1b — Locale and Time Awareness ✅ COMPLETE

**Goal:** Current date/time and user locale injected into every system prompt from day one. Not an afterthought.

- [x] `context/locale.rs` — detect system locale (`std::env::var("LANG")`, system timezone via `chrono-tz`); store in config as `[locale]` block
- [x] System prompt builder: prepend `"Current date and time: {weekday} {date}, {time} {tz_abbr} (UTC{offset})\nUser location: {country}\nLanguage: {language}"` to every query
- [x] Gemini search backend: pass `gl={country_code}` in every search-grounded request for localised results
- [x] `config.toml` `[locale]` schema: `country`, `timezone`, `language`, `date_format`, `units`, `currency`
- [x] Init wizard Step 1: show auto-detected locale, confirm or override

**Milestone:** System prompt for every query includes the current date/time and country. Gemini search returns localised results.

---

## Phase 2 — Setup Wizard ✅ COMPLETE

**Goal:** `zedplus init` runs end-to-end and produces a valid routing config.

- [x] Add `inquire` dependency
- [x] `setup/detector.rs` — RAM/CPU via `sysinfo`; VRAM via `nvml-wrapper` (NVIDIA) and platform API (Apple Silicon); training feasibility verdict
- [x] `platform/auth.rs` — OAuth device flow for Gemini; browser-assist flow for Anthropic + OpenAI; manual entry fallback for all providers
- [x] `setup/services.rs` — checkbox prompt for AI services; per-provider auth flow; live validation; `[B]rowser / [M]anual` choice per provider
- [x] `zedplus auth [--provider X]` and `zedplus auth --revoke <provider>` subcommands
- [x] `setup/profile.rs` — use-case multi-select, routing priority select, auto-train opt-in question
- [x] `setup/mod.rs` — wizard orchestration: run all steps, write config + models.toml, store keys in keychain
- [x] Display routing plan summary with estimated monthly cost before saving
- [x] Show device verdict and local LLM recommendation

**Milestone:** `zedplus init` completes, writes `~/.config/zedplus/config.toml`, keys stored in OS keychain.

---

## Phase 3 — Code Indexer ✅ COMPLETE

**Goal:** `zedplus index` watches a codebase and builds the SQLite chunk index.

- [x] Add `notify`, `tree-sitter` + language grammars (Rust, JS/TS, Python, Go), `git2`
- [x] `indexer/parser.rs` — tree-sitter per-language: extract functions, classes, top-level symbols as chunks
- [x] `indexer/embedder.rs` — call Ollama `nomic-embed-text` HTTP API, return `Vec<f32>` embeddings
- [x] `indexer/store.rs` — SQLite upsert of chunks + embeddings; cosine similarity search (top-K); file hash change detection
- [x] `indexer/watcher.rs` — `notify` wrapper with 500ms debounce; re-index only changed files
- [x] `indexer/git.rs` — `git2` wrapper: read `git diff HEAD`, `git status`; expose as injectable context
- [x] `indexer/mod.rs` — orchestrate: watch → parse → embed → store; one-shot `index_snapshot()` for background startup indexing
- [x] `zedplus index` command: starts watcher, prints indexing progress
- [x] Background auto-indexing at REPL startup (content-hashed; only re-indexes changed files)

**Milestone:** `zedplus index` watches a Rust project, indexes it into SQLite, re-indexes on save, exposes similarity search.

---

## Phase 4 — AI Backends ✅ COMPLETE

**Goal:** All AI providers callable with streaming output.

- [x] Add `reqwest` (with `rustls`), `tokio-stream`, `futures`
- [x] `backends/mod.rs` — `Backend` async trait: `complete`, `complete_streaming`, `agent_step`; error variants including `RateLimit`, `Timeout`, `Auth`
- [x] `backends/claude.rs` — Anthropic Messages API + SSE streaming + prompt caching
- [x] `backends/gemini.rs` — Google AI API + chunked streaming + Search grounding flag
- [x] `backends/ollama.rs` — Ollama `/api/generate` streaming endpoint; health check
- [x] `backends/openai.rs` — OpenAI Chat Completions API + SSE streaming; LM Studio variant
- [x] `backends/claude_cli.rs` — Claude Code CLI via `claude --print`; `--yes` when auto-accept
- [x] `backends/gemini_cli.rs` — Gemini CLI via stdin pipe; `--yes` when auto-accept
- [x] Fallback chain: on `RateLimitError`, router tries CLI subscriptions then API fallback
- [x] Stream output to terminal via `crossterm`

**Milestone:** `zedplus ask "hello" --model claude-haiku` streams a response to the terminal.

---

## Phase 5 — Smart Router ✅ COMPLETE

**Goal:** Queries are automatically classified and routed to the right model.

- [x] `router/classifier.rs` — keyword heuristics: map query → `TaskType` enum (`WebSearch`, `CodeReview`, `ComplexReasoning`, `DataAnalysis`, `Documentation`, `QuickCompletion`, `Fallback`); "explain why" → ComplexReasoning, bare "explain" → Documentation
- [x] `router/cost.rs` — token count estimation; cost calculation from `costs.toml`
- [x] `router/rules.rs` — load routing rules from config; filter by strengths; select by priority mode; project overrides over global
- [x] `router/mod.rs` — routing pipeline: classify → filter → select → apply fallback chain
- [x] `resolve_model()` — prefix matching so "gemini-flash" resolves to "gemini-flash-2-5"
- [x] Per-phase model configurability: `[pipeline]` config block with `reasoning`, `planning`, `execution` model lists
- [x] Lite-task routing: Documentation/QuickCompletion → local model first → CLI subscription → API
- [x] Git context injection: CodeReview task type prepends git diff
- [x] `--explain` output: task type, selected model, reasoning, token/cost estimate, cheapest alternative

**Milestone:** `zedplus ask "review this function" --explain` routes correctly and prints the routing decision.

---

## Phase 6 — Distiller ✅ COMPLETE

**Goal:** Every AI call is captured to JSONL. Usage is tracked.

- [x] `distiller/mod.rs` — wraps every backend call; on complete, append Alpaca-format JSONL to `{data_dir}/distill/YYYY-MM.jsonl`
- [x] Monthly file rotation
- [x] Usage recording: write row to `usage` SQLite table on every call (model, task_type, tokens, cost, cache_hit)
- [x] Override signal: record `override_model` when user passed `--model`; negative signal on re-ask
- [x] `zedplus clear` — wipe in-memory session context only; distillation data preserved
- [x] `zedplus distill` — export JSONL with filters
- [x] `zedplus usage` — query usage table, render table by day/month/project with cost breakdown

**Milestone:** After 10 queries, `{data_dir}/distill/YYYY-MM.jsonl` has 10 valid Alpaca-format lines.

---

## Phase 7 — REPL + `zedplus ask` End-to-End ✅ COMPLETE

**Goal:** Full query pipeline: index lookup → route → stream → distill → session context.

- [x] `repl/mod.rs` — main REPL loop: read input, detect slash command vs query, dispatch, render response
- [x] `repl/readline.rs` — crossterm raw-mode editor with slash-command dropdown autocomplete; `KeyEventKind::Press` filter (Windows double-fire fix)
- [x] `repl/commands.rs` — slash command registry: `/help`, `/clear`, `/index`, `/usage`, `/history`, `/explain`, `/local`, `/cheap`, `/model`, `/scope`, `/agent`, `/accept`, `/apply`, `/build`, `/exit`
- [x] Context assembly: top-K chunks from similarity search + git diff (if code_review task)
- [x] Session context: in-memory `Vec<Message>` per session; prepend to each call
- [x] Session summarization: when context approaches token limit, summarize and replace history
- [x] Claude prompt caching: system prompt with cache headers on first call
- [x] Wire distiller around every backend call
- [x] Streaming terminal output with graceful Ctrl+C cancel
- [x] Exit summary: turns, cost, per-backend breakdown (local / subscription / API), resume hint
- [x] `/history` — last 20 turns with provider and answer preview
- [x] `/accept` — toggle auto-accept; propagates `--yes` to claude-cli and gemini-cli
- [x] `/agent` — toggle agentic mode (file tools, run_command, search, git tools, semantic search)
- [x] `/apply` — apply code blocks from last response to files

**Milestone:** `zedplus` opens REPL, multi-turn session works with context, streaming, and distillation.

---

## Phase 7b — Session Persistence & Resume ✅ COMPLETE

**Goal:** Sessions auto-save and resume by directory + branch with human-readable names.

- [x] `sessions` and `session_turns` SQLite tables
- [x] Auto-save: every `session_turns` row written atomically after each AI turn completes
- [x] Session naming: after first query, fire cheap model call to generate 3–5 word slug
- [x] Resume detection: on `zedplus ask`, query sessions for same `project_path` + `git_branch`; prompt if within threshold
- [x] Exit message: name, turn count, cost per backend; "resume: zedplus resume"
- [x] `zedplus resume` — load most recent session in current dir
- [x] `zedplus session list` — table of sessions for current project
- [x] `zedplus session list --all` — across all projects
- [x] `zedplus session resume <name>` — by name
- [x] `zedplus session rename / archive` subcommands
- [x] `[sessions]` config block: `auto_resume_threshold_hours`, `max_resume_candidates`

**Milestone:** Close terminal mid-session, reopen, run `zedplus resume` — conversation context fully restored.

---

## Phase 7c — Agentic Mode ✅ COMPLETE

**Goal:** Model reads files, runs tools, commits code autonomously.

- [x] `agent/mod.rs` — ReAct loop: agent_step → tool dispatch → append results → next step; max_iterations guard
- [x] `agent/tools.rs` — `read_file`, `write_file`, `list_dir`, `run_command`, `search_files`, `glob_files`, `search_semantic`, `git_status`, `git_commit`
- [x] Auto-fallback to ReAct text mode when backend doesn't support tool use (CLI backends)
- [x] `/build` pipeline — multi-phase: clarify → arch → plan → build → QC → test → devlog
- [x] `pipeline/selector.rs` — per-phase model cascade with user-configurable preferences
- [x] **Repomap Strategy:** Automated injection of high-level project structure into context for accurate "diff-only" editing.

---

## Phase 8 — Adaptive Routing ✅ COMPLETE

**Goal:** ZedPlus learns user preferences from usage patterns.

- [x] `router/adaptive.rs` — query `usage` table; count override patterns per task_type; count negative signals per model+task_type
- [x] Suggestion logic: if override_model X used ≥5 times for task_type Y, suggest adding routing rule
- [x] `zedplus profile --optimize` — run analysis, print diff of suggested routing changes
- [x] `zedplus profile --optimize --apply` — write suggestions to `.zedplus.toml`

**Milestone:** After manually overriding `data_analysis` to `gemini-pro` 5 times, `zedplus profile --optimize` suggests the rule change.

---

## Phase 8b — Developer Personas ✅ COMPLETE

**Goal:** Switch the AI's focus with a single slash command. Mysti ships 16 personas; ZedPlus builds 8 core ones.

- [x] `persona/mod.rs` — persona registry: `architect`, `debugger`, `security`, `performance`, `teacher`, `reviewer`, `tester`, `devops`
- [x] Each persona: supplemental system prompt block (priorities, heuristics, output format preferences)
- [x] `/persona <name>` — switch persona for session
- [x] `/persona` alone — list available personas with one-line description + active indicator
- [x] `/persona off` — clear active persona
- [x] `[persona]` config block: `default_persona`, `show_in_prompt`
- [x] Persona logged in `session_turns.persona` DB column (via additive migration)
- [x] Persona injected into system prompt for both agent and streaming modes

**Why:** A single model responds very differently when told "you are a performance engineer who cares only about hot paths" vs default. This is nearly free to implement (system prompt injection) and dramatically improves response relevance.

---

## Phase 8c — Multi-agent Brainstorm ✅ COMPLETE

**Goal:** For hard problems, get two models to answer independently, then synthesize. Convergence detection exits early when they agree.

- [x] `/debate <query>` slash command — route same query to two backends (claude → gemini)
- [x] 4 collaboration strategies: `debate` (A proposes, B critiques), `red-team` (A proposes, B stress-tests), `perspectives` (both answer independently), `delphi` (iterative refinement up to 3 rounds)
- [x] Convergence detection: word Jaccard similarity ≥ 0.62 → converged; Delphi exits early when met
- [x] `[brainstorm]` config: `default_strategy`, `convergence_threshold`, `max_delphi_rounds`
- [x] `/debate <strategy> <query>` — named strategy prefix (debate/red-team/perspectives/delphi)
- [x] Combined response stored as `last_response` so `/apply` can use it

**Pending / future:**
- [x] Token accounting across all brainstorm calls
- [x] Auto-trigger for `ComplexReasoning` tasks above a token threshold

---

## Phase 8d — @-mention Inline Routing ✅ COMPLETE

**Goal:** Route a query to a specific backend inline, without switching commands.

- [x] Query parser: detect `@claude`, `@gemini`, `@local`, `@cheap`, `@fast` anywhere in input
- [x] Single `@provider` mention: routes whole query to that backend, stripped before sending
- [x] `@cheap` / `@fast` virtual mentions: resolve to `flags.cheap = true`
- [x] `@local` virtual mention: resolve to `flags.local = true`
- [x] `@claude` / `@gemini` prefer CLI subscriptions when available, fall back to API
- [x] `/help` updated with @-mention documentation
- [x] Multiple `@provider` mentions: split query at mention boundaries, route segments independently
- [x] `@openai` mention support
- [x] `@codex` mention: routes to OpenAI with `codex-mini-latest`; auto-completes in REPL dropdown
- [x] `@groq`, `@qwen` mention support

---

## Phase 9 — Local Model Training ✅ COMPLETE

**Goal:** ZedPlus can trigger and monitor LoRA fine-tuning on the user's local model.

- [x] `distiller/trainer.rs` — recency-weighted JSONL export logic (1.0× last 30d, 0.5× 30–90d, 0.25× older)
- [x] Training orchestration: shell out to Unsloth or Axolotl with constructed args; write `train_jobs` row; tail output
- [x] `zedplus train [--base model] [--data file] [--lora | --full]` — manual training trigger
- [x] `zedplus train --status` — poll `train_jobs` table, display progress/ETA
- [x] **Significance Heuristics:** Automated training suggestions based on session value (cost, turns, files written).
- [x] **Environment Orchestration:** Selection between Docker and Venv based on system capabilities.
- [x] **Specialized Modes:** Coding vs Writing dataset filtering and base model recommendations.
- [x] `zedplus model import <path|ollama-id> --name X` — register in `model_registry` + `models.toml`

**Milestone:** `zedplus train --base llama3.2:8b --lora --bench` starts a training job and runs a benchmark on completion.

---

## Phase 10 — Benchmarking ✅ COMPLETE

**Goal:** ZedPlus can measure whether a fine-tuned model is better than the baseline.

- [x] `distiller/bench.rs` — sample distillation JSONL as frozen benchmark set
- [x] **Scoring Engine:** cosine similarity of output embedding (Semantic), Token F1 (Lexical), and Format Correctness (<tool_call> tags).
- [x] Write score rows to `bench_results` table
- [x] `zedplus bench [--model X] [--baseline Y]` — run scoring, print table by task type with delta
- [x] Routing recommendation output: suggest updating routing rules where new model wins

**Milestone:** `zedplus bench --model my-lora --baseline llama3.2:8b` prints a scored comparison table.

---

## Phase 11 — Update System ✅ COMPLETE

**Goal:** Users can check for and install new ZedPlus versions.

- [x] `platform/update.rs` — HTTP call to GitHub Releases API; compare semver; download signed binary; replace self
- [x] `zedplus update --check` — print available version if newer
- [x] `zedplus update` — download + install, confirm before replacing binary
- [x] Startup version check (opt-in, non-blocking)
- [x] GitHub Actions release workflow — tag-triggered builds for Windows (MSVC), macOS arm64/x86_64, Linux; auto-attaches artifacts to GitHub release

**Milestone:** `zedplus update --check` correctly detects whether a newer release exists on GitHub.

---

## Phase 12 — Polish & v1 Release ✅ COMPLETE (v0.8.0)

**Goal:** Stable, documented, cross-platform v1.0.

- [x] Error messages: Ollama 404 now includes actual model name + "did you mean" suggestions; `@local/<name>` shows discovered model list on mismatch
- [x] Rate limit graceful degradation: `RateLimitError` triggers fallback chain with user notification; Codex added to failover chain
- [x] `/ui [native|claude|gemini]` — show/change UI style, persisted to config immediately
- [x] First-run UI preference prompt — detects installed CLIs, asks preference before first REPL loop
- [x] Adaptive routing fix — `select_alias()` no longer defaults to Claude when the configured alias is absent from registry; picks best available model by task strengths
- [x] Bundled `costs.toml` and `models.toml` with current model set (Codex, GPT-4.1, Gemini 2.5, Claude Opus 4.7)
- [x] GitHub Actions matrix (ubuntu-latest, macos-latest, windows-latest)
- [x] Release pipeline: cross-compile for all platforms; `.zip` (Windows), `.dmg` (macOS), `.tar.gz` (Linux)
- [x] Install scripts: `curl | sh` for Mac/Linux (`install.sh`), PowerShell one-liner for Windows (`install.ps1`)
- [x] `zedplus config --show` / `--edit` / `--reset` / `--set KEY=VALUE` — full config inspection and live editing (15+ settable keys)
- [x] README with quickstart, install, command reference, and routing table
- [x] **Test suite**: 22 unit tests for `router/classifier.rs` (7 task types), `router/cost.rs` (token/cost), `router/rules.rs` (alias selection + fallback), `local_models.rs` (param extraction + scoring); all 25 tests pass

---

## Phase 12b — ZEDPLUS.md, Hooks, Shell Mode, Headless ✅ COMPLETE (v0.10.0)

**Goal:** Project context file, automation hooks, shell command generation, CI mode.

- [x] `context/zedplusmd.rs` — walk directories upward from cwd to find `ZEDPLUS.md`; inject into every system prompt under `## Project Context`
- [x] `zedplus init --context` — scaffold a `ZEDPLUS.md` from the codebase index (language breakdown, key files, README/CHANGELOG excerpts, directory structure)
- [x] `hooks/mod.rs` — full `HookRunner` with `run()` and `run_warn()` methods; `[hooks]` config schema with all 8 points
- [x] Eight hook points: `before_apply_change`, `after_apply_change`, `before_commit`, `after_commit`, `before_session`, `after_session`, `before_search`, `before_cloud_send`; wired into REPL session start/end and apply_response
- [x] `shell/mod.rs` — `zedplus shell "<description>"`: AI generates OS-aware command, displays it, `[Y/n/e(dit)]` confirm, executes
- [x] `shell/integration.rs` — bash/zsh/fish hotkey snippets (Ctrl+Z); `zedplus shell --install-hotkey` appends to RC file idempotently
- [x] Headless mode: non-TTY detection; `run_pipe_loop()` suppresses prompts
- [x] `--output json` and `--output plain` flags for `zedplus ask` (scripted use)
- [x] `--exit-code` flag: exits 1 when AI response contains error/warning/failed (CI gate)

**Milestone:** `ZEDPLUS.md` injected at session start. Pre-commit hook runs on apply. `zedplus shell "..."` generates and runs a shell command. `zedplus ask "..." --output json --exit-code` works as a CI gate.

---

## Phase 12c — Architect/Editor Mode ✅ COMPLETE (v0.8.1)

**Goal:** Smart model plans, cheap model applies. 30–50% cost reduction on large code tasks.

- [x] `router/architect.rs` — `check_eligibility()` detects eligible tasks (`CodeReview`, `ComplexReasoning`; keyword match or query length ≥ `threshold_lines`)
- [x] Architect phase: high-quality model receives query + code context + "produce a plan only" instruction; streams plan inline with `── Architect phase ──` header
- [x] Editor phase: fast model receives original query + plan + "implement this" instruction; streams implementation inline with `── Editor phase ──` header
- [x] Architect/editor mode bypassed when `@mention`, `/model`, `/local`, `/cheap` override is active; also bypassed in agent mode
- [x] `--explain` updated to show both model IDs and per-phase cost estimate (`Arch/Edit: … → …`, `Split cost: $x.xx (arch) + $x.xx (edit)`)
- [x] `[routing.architect_editor]` config: `enabled`, `architect_model`, `editor_model`, `threshold_lines`
- [x] `is_architect_split: true` in distiller entries; combined plan+implementation stored in session context

---

## Phase 13 — Background Test Runner ✅ COMPLETE (v0.9.0)

**Goal:** Run the project's real tests after every AI-made change and surface failures inline.

- [x] `tester/mod.rs` — detect test runner from project files (`Cargo.toml` → cargo test, `pytest.ini`/`conftest.py` → pytest, `package.json` → npm test, `go.mod` → go test)
- [x] `tester/runner.rs` — background test job; capture stdout/stderr; parse pass/fail; write to `test_runs` SQLite table; print ✅/❌ inline
- [x] Fire test runner after every `write_file` tool call in agent mode; retries agent turn with failure context if tests fail
- [x] `tester/coverage.rs` — heuristic coverage scanner: walks project tree, counts test annotations (`#[test]`, `def test_`, `describe(`, `func Test`, `#[cfg(test)]`) without running tests
- [x] `/fix` REPL command: routes `session.last_test_failure` to AI with a "fix this minimal" prompt; clears state after fix attempt
- [x] `check_last_test_failure()` queries `test_runs` table after each agent turn; surfaces `[tests still failing] Type /fix` hint when the last run failed
- [x] `session.last_test_failure` field persists stderr across turns within a session

**Deferred to later phase:**
- [ ] Benchmark runner: detect `cargo bench` / `pytest-benchmark`; surface regressions
- [ ] `[testing]` config knobs: `auto_run`, `suggest_tests`, `run_benchmarks`

**Milestone:** After `zedplus ask` modifies a Rust source file, `cargo test` runs in the background and failed tests appear inline; `/fix` sends the failure to AI for resolution.

---

## Phase 15 — Multimodal, Goal Anchoring & Skill Packs (Week 16–18)

**Goal:** Image/file inputs, scope enforcement, and domain skill packs.

### 13a. Multimodal inputs
- [ ] Add base64 image encoding for `--image` flag; pass to vision-capable backends
- [ ] Add `supports_vision` and `supports_pdf` to `models.toml`; router enforces vision-capable model
- [ ] PDF handling: native pass-through for Gemini; base64 for Claude; text extraction for Ollama
- [ ] CSV / plain text: read file, inject as fenced context block
- [ ] `platform/clipboard.rs` — detect image in clipboard on macOS/Windows

### 13b. Goal anchoring and minimal footprint
- [ ] System prompt: add "minimal footprint" instruction
- [ ] Session context: prepend original first user message to every subsequent turn
- [ ] `--scope narrow|broad` flag; `narrow` as default
- [ ] Change confirmation: show diff prompt before applying any file modification
- [ ] `zedplus task "<multi-step request>"` — decomposed plan with approval gate per step
- [ ] Negative signal on scope creep: flag prior response with `scope_violation = 1`

### 13c. Skill packs
- [ ] `skills/mod.rs` — load `.toml` skill packs from `~/.config/zedplus/skills/`
- [ ] `skills/library.rs` — bundled skill pack registry; ship 6 built-in packs
- [ ] `skills/suggest.rs` — analyze usage table → suggest matching skill packs
- [ ] `zedplus skills list / install / suggest / create` subcommands

**Milestone:** `zedplus ask "explain this" --image ./error.png` routes to a vision model. `zedplus skills install react-developer` tunes routing for `.tsx` files.

---

## Phase 16 — Community Ecosystem (Week 18–20)

**Goal:** Allow users to share and discover custom skill packs, LoRA adapters, and high-quality distillation datasets via a decentralized, GitHub-backed registry.

- [ ] **GitHub-Backed Registry:** Instead of a complex CDN, ZedPlus pulls a `registry.json` from a central public GitHub repository containing metadata and URLs for community assets.
- [ ] `zedplus skills search <query>` — Search the public registry for skill packs (e.g., `zedplus skills search rust`).
- [ ] `zedplus model adapters list --remote` — Browse community-trained LoRA models.
- [ ] **Opt-in Distillation Contribution:** `zedplus distill --export-community` to scrub, anonymize, and upload high-quality, verified ReAct tool-use examples to a public HuggingFace dataset to improve open-source agentic models.
- [ ] **Skill Mentions:** Support for inline skill injection (e.g., `zedplus ask "how do I center this div? @css-expert"`).

**Milestone:** A user can run `zedplus skills search react` and install a community-maintained React skill pack directly from the CLI.

---

## What's deferred to v2

| Feature | Why deferred |
|---|---|
| BM25 + reranker for context retrieval | Works without it; adds complexity |
| `zedplus undo` | Requires tracking applied diffs in SQLite |
| LM Studio backend | Ollama covers the local use case for v1 |
| Editor plugin (VS Code / JetBrains) | CLI must be solid first |
| `--stream` interrupt + course-correct | Park-and-complete is simpler for v1 |
| Clipboard auto-detection for images | `--image` flag covers the use case |
| Mysti-style web dashboard | CLI-first philosophy |

---

## Dependency map

```
Phase 1  (Foundation) ✅
  └─→ Phase 2  (Setup Wizard) ✅
        └─→ Phase 3  (Indexer) ✅
              └─→ Phase 4  (Backends) ✅
                    ├─→ Phase 5  (Router) ✅
                    │     ├─→ Phase 6  (Distiller) ✅
                    │     │     └─→ Phase 7  (REPL end-to-end) ✅  ← first usable product
                    │     │           ├─→ Phase 7b (Session Persistence) ✅
                    │     │           ├─→ Phase 7c (Agentic Mode) ✅
                    │     │           ├─→ Phase 8  (Adaptive Routing) ✅
                    │     │           ├─→ Phase 8b (Developer Personas) ✅
                    │     │           ├─→ Phase 8c (Multi-agent Brainstorm) ✅
                    │     │           ├─→ Phase 8d (@-mention Routing) ✅
                    │     │           ├─→ Phase 9  (Training)
                    │     │           │     └─→ Phase 10 (Benchmarking)
                    │     │           ├─→ Phase 15 (Multimodal + Skills)
                    │     │           │     └─→ Phase 16 (Community Ecosystem)
                    │     └─→ Phase 11 (Update System) ✅
                    └─→ Phase 12 (Polish + Release) ✅ v0.8.0
                          ├─→ Phase 12b (ZEDPLUS.md, Hooks, Shell) ✅ v0.10.0
                          ├─→ Phase 12c (Architect/Editor Mode) ✅ v0.8.1
                          └─→ Phase 13 (Background Test Runner) ✅ v0.9.0
```

**First usable product:** ✅ Complete as of v0.6.x — `zedplus ask` works end-to-end with routing, streaming, distillation, sessions, and agentic mode.

**Polish complete:** ✅ v0.10.0 — config management, unit tests, README, architect/editor routing, test runner, ZEDPLUS.md context, hooks, shell command generation, CI output flags.

**MVP for public launch:** Phase 15 — multimodal inputs, goal anchoring, skill packs.

**Full v1 feature complete:** End of Phase 15 (~week 18).

---

## Crates reference

```toml
[dependencies]
# CLI
clap = { version = "4", features = ["derive"] }

# Async runtime
tokio = { version = "1", features = ["full"] }

# Serialization
serde = { version = "1", features = ["derive"] }
serde_json = "1"
toml = "0.8"

# HTTP + streaming
reqwest = { version = "0.12", features = ["stream", "rustls-tls"], default-features = false }
tokio-stream = "0.1"
futures = "0.3"

# Database
rusqlite = { version = "0.31", features = ["bundled"] }

# File watching
notify = "6"

# Tree-sitter + grammars
tree-sitter = "*"
tree-sitter-rust = "*"
tree-sitter-javascript = "*"
tree-sitter-python = "*"
tree-sitter-go = "*"
tree-sitter-typescript = "*"

# Git
git2 = { version = "0.18", default-features = false }

# TUI / terminal
crossterm = "0.27"
inquire = "0.7"

# Platform
dirs = "5"
keyring = "2"
sysinfo = "0.30"
open = "5"

# Time and locale
chrono = { version = "0.4", features = ["serde"] }
chrono-tz = "0.9"

# Error handling + logging
anyhow = "1"
thiserror = "1"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }

# Async trait support
async-trait = "0.1"
```
