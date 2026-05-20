# ZedPlus Architecture

## What it is

A single Rust CLI that combines four capabilities no existing tool ships together:

1. **Realtime incremental code indexing** — tree-sitter parsing + embeddings, only reindexes changed files
2. **Smart multi-AI routing** — Claude, Gemini, OpenAI, local LLM — selected by task type, cost tier, and user profile
3. **Web search** — via Gemini's built-in Google Search grounding (no extra API key needed)
4. **Response distillation** — JSONL capture of all AI responses for local LLM fine-tuning

The distillation loop (AI responses → JSONL → LoRA fine-tune → better local model → benchmark → routing update) is the most novel gap in the market. Cursor covers 3/4 but skips this entirely. Over time, the local model learns the user's codebase, style, and patterns — compounding value with every session.

---

## Cross-platform support

**Short answer: yes — all three platforms are first-class.**

Rust's stdlib and the chosen crates handle the platform differences:

| Concern | How it's handled |
|---|---|
| Paths | `std::path::PathBuf` everywhere — no hardcoded `/` or `\` |
| Config dirs | `dirs` crate: `%APPDATA%` (Win) / `~/Library/...` (Mac) / `~/.config` (Linux) |
| Data dirs | `dirs` crate: `%LOCALAPPDATA%` / `~/Library/...` / `~/.local/share` |
| File watching | `notify` crate: ReadDirectoryChangesW / FSEvents / inotify — same API |
| TLS | `rustls` (no OpenSSL dependency — critical for clean Windows builds) |
| Terminal UI | `crossterm` + `inquire` — no ANSI escape codes written directly |
| Secret storage | `keyring` crate: Credential Manager (Win) / Keychain (Mac) / libsecret (Linux) |
| System info | `sysinfo` crate — RAM, CPU, processes — cross-platform |
| GPU info | `nvml-wrapper` (NVIDIA), platform-native for Apple Silicon |

The only platform-specific branch is GPU detection (see Device Detection below).

---

## Temporal and locale awareness

Every system prompt includes the current date, time, and user locale. This is injected automatically — the model always knows when "now" is and where the user is. No tool in the current landscape does this explicitly; they rely on training cutoff knowledge, which goes stale.

```toml
# set at init, stored in config.toml
[locale]
country = "AU"           # ISO 3166-1 alpha-2
timezone = "Australia/Sydney"
language = "en-AU"       # BCP 47
date_format = "DD/MM/YYYY"
units = "metric"
currency = "AUD"
```

### What locale affects

| Setting | Where it's used |
|---|---|
| `country` | Gemini search `gl` parameter — localised results; legal/compliance context in responses |
| `timezone` + current time | Injected into every system prompt; model knows "now" without guessing |
| `language` | Response spelling, idiom, and phrasing (en-AU ≠ en-US) |
| `date_format` | All dates in CLI output; code snippets use local format by default |
| `units` | Responses use metric or imperial appropriately |
| `currency` | Cost displays (`$4.20 AUD`) and code examples |

### System prompt injection (automatic, every query)

```
Current date and time: Saturday 17 May 2026, 14:32 AEST (UTC+10)
User location: Australia (AU)
Language: Australian English
```

This means:
- "What's the latest version of X?" gets a temporally grounded search, not a stale training answer
- "How do I comply with privacy law?" gets Australian Privacy Act context, not GDPR by default
- "Review this date parsing code" knows DD/MM/YYYY is expected, not MM/DD/YYYY

### Init question

```
Step 1/6  ─  Where are you based?
  Country: Australia [AU]  (detected from system locale — change?)
  Timezone: Australia/Sydney  (detected — change?)
  Language: English (AU)  (change?)
```

Auto-detected from `std::env` locale and system timezone on first run. User confirms or overrides. Stored in config, not in keychain — not sensitive.

### Re-configuration

```
zedplus config --set locale.country=GB
zedplus config --set locale.timezone=Europe/London
```

---

## Setup wizard (`zedplus init`)

Run once on first use (or re-run to reconfigure). Uses `inquire` for interactive prompts — no raw terminal codes, works in all terminals including Windows Terminal and PowerShell.

### Flow

```
┌──────────────────────────────────────────────────────────┐
│  Welcome to ZedPlus                                      │
│  Let's set up your AI routing in ~2 minutes.            │
└──────────────────────────────────────────────────────────┘

Step 1/6  ─  Which AI services do you have access to?
  [x] Anthropic (Claude)   — best for complex code & reasoning
  [x] Google AI (Gemini)   — best for web search + data analysis
  [ ] OpenAI (GPT-4o)      — broad ecosystem compatibility
  [ ] Ollama (local/free)  — requires capable hardware (checking...)
  [ ] LM Studio (local)    — requires capable hardware (checking...)

Step 2/6  ─  What do you primarily use AI for?  (pick all that apply)
  [x] Web development (React, Vue, Node, APIs)
  [ ] Mobile development (iOS / Android / Flutter)
  [ ] Backend / systems / low-level
  [x] Data analysis / ML / notebooks
  [ ] DevOps / infra / scripts
  [ ] Writing / documentation

Step 3/6  ─  What's your routing priority?
  > Balanced  (quality + cost — recommended for most users)
    Highest quality  (cost is secondary)
    Lowest cost  (limited credits)
    Local first  (privacy / offline)

Step 4/6  ─  API keys
  Anthropic API key: ****************************  [validated ✓]
  Google AI API key: ****************************  [validated ✓]

Step 5/6  ─  Your device
  RAM: 32 GB  |  GPU: NVIDIA RTX 4070 (12 GB VRAM)
  → Local LLMs: up to 13B models (fast), 30B (slower)
  → Suggested: pull llama3.2:8b via Ollama for quick completions

  Routing plan (editable):
    web search        → gemini-flash (Search grounding)
    quick completion  → local llama3.2:8b   ← free
    code review       → claude-sonnet-4-6
    data analysis     → gemini-pro-2.5
    documentation     → claude-haiku-4-5
    fallback          → claude-haiku-4-5

  Estimated cost at ~200 queries/day: ~$4–8/month

Step 6/6  ─  Local model auto-training
  ZedPlus can automatically improve your local model over time by
  fine-tuning on your conversations during idle periods.

  > Yes — auto-train when I accumulate 200+ new conversations (recommended)
    Yes — auto-train weekly regardless of volume
    No  — I'll trigger training manually with `zedplus train`

  [Save and continue] [Edit routing rules] [Skip for now]
```

Keys are stored in the OS keychain (never in plaintext config files). Authentication uses the best available flow per provider — OAuth device flow where supported, browser-assisted API key generation otherwise. Manual paste is always available as a fallback.

### Authentication flows per provider

**Google AI (Gemini) — OAuth device flow**
```
  Authenticating with Google AI...
  Opening browser → https://accounts.google.com/device
  Code: ZEDP-7842  (expires in 10 minutes)

  Waiting... ⠋
  ✓ Authenticated as gautam.bass@gmail.com
```
CLI requests a device code, opens the browser automatically (`open`/`xdg-open`/`start`), then polls the OAuth token endpoint every 5 seconds. On success, the access + refresh tokens are stored in the OS keychain. Refresh is handled silently on expiry — the user never re-authenticates unless they explicitly revoke.

**Anthropic (Claude) — browser-assisted API key**
Anthropic has no public OAuth device flow. ZedPlus opens the API keys page directly and waits for the user to paste the generated key:
```
  Opening Anthropic Console → API Keys page...

  Generate a new key there, then paste it here:
  API key: sk-ant-****  [validated ✓]
```

**OpenAI — browser-assisted API key**
Same pattern as Anthropic:
```
  Opening OpenAI Platform → API Keys page...

  API key: sk-****  [validated ✓]
```

**Manual entry (always available)**
Any provider can be configured by pasting a key directly — no browser required. Used for headless environments, CI, or users who prefer it:
```
  Google AI API key (or press B to open browser): ****  [validated ✓]
```

**Re-authentication**
```
zedplus auth                     # re-run auth for all configured providers
zedplus auth --provider gemini   # re-auth a specific provider
zedplus auth --revoke gemini     # remove stored credentials
```

---

## Device detection & local LLM feasibility

Runs silently during `zedplus init` and on first `zedplus ask` if config is missing.

### Detection logic

```
sysinfo → total_memory, cpu_count
nvml-wrapper (Windows/Linux NVIDIA) → vram
system-configuration (Mac) → is_apple_silicon, unified_memory
```

### Thresholds

| RAM | GPU VRAM | Apple Silicon | Local LLM verdict |
|---|---|---|---|
| < 8 GB | any | no | **Disabled** — inform user, hide local options |
| 8–15 GB | < 4 GB | no | CPU-only: 3B–7B Q4 (slow, functional) |
| 8–15 GB | 4–8 GB | no | GPU: 7B Q4 fast |
| 16–31 GB | 8–15 GB | no | GPU: 13B comfortable |
| 32 GB+ | 16 GB+ | no | GPU: 30B+, fast 13B |
| any | — | M1/M2 8 GB unified | 7B Q4 (Metal acceleration) |
| any | — | M1/M2 16 GB unified | 13B, fast 7B |
| any | — | M2/M3 Pro 32 GB+ | 30B+, fast 13B |

**Training feasibility** is stricter than inference — auto-training requires a GPU with ≥ 6 GB VRAM for LoRA runs on 7B models. On CPU-only devices, auto-training is silently disabled; the distillation JSONL still accumulates so the user can train externally.

When local LLM is disabled, the local module is omitted from the routing table and all references to it are hidden from CLI output. User is shown a one-time message:

```
⚠  Local LLM disabled: your device has 6 GB RAM (minimum: 8 GB).
   All queries will route to cloud providers.
   Run `zedplus init` after upgrading hardware to re-enable.
```

---

## AI cost model

Pricing stored in `costs.toml` (bundled, user-updatable). Model capabilities stored separately in `models.toml` (see Model Registry below). Used for cost-impact display and routing decisions.

### Approximate pricing (mid-2026, per million tokens)

| Model | Input $/MTok | Output $/MTok | Best for |
|---|---|---|---|
| claude-haiku-4-5 | $0.80 | $4.00 | Docs, summaries, cheap fallback |
| claude-sonnet-4-6 | $3.00 | $15.00 | Complex code, reviews — best quality/cost |
| claude-opus-4-7 | $15.00 | $75.00 | Hardest reasoning tasks only |
| gemini-flash-2-5 | $0.15 | $0.60 | Search grounding, cheap queries |
| gemini-pro-2-5 | $1.25 | $5.00 | Data analysis, long context |
| gpt-4o-mini | $0.15 | $0.60 | OpenAI cheap tier |
| gpt-4o | $2.50 | $10.00 | OpenAI quality tier |
| local (ollama) | $0.00 | $0.00 | Quick completions, private data |

ZedPlus tracks tokens per session and per project in SQLite and surfaces a `zedplus usage` report.

**Adding new models:** Update `costs.toml` (pricing) and `models.toml` (capabilities). No code changes required for new models within an existing provider. New providers require a new backend `.rs` file implementing the `Backend` trait.

---

## Model registry (`models.toml`)

Every model's capabilities are declared in `models.toml` — separate from pricing. This drives routing decisions without hardcoding logic in Rust.

```toml
[models.gemini-flash-2-5]
provider = "gemini"
id = "gemini-2.5-flash"
context_window = 1_000_000
supports_search_grounding = true
supports_vision = true
quality_tier = 2           # 1=cheapest/weakest … 5=strongest
speed_tier = 5             # 1=slowest … 5=fastest
strengths = ["web_search", "data_analysis", "quick_completion"]
weaknesses = ["complex_reasoning"]

[models.claude-sonnet-4-6]
provider = "claude"
id = "claude-sonnet-4-6"
context_window = 200_000
supports_cache = true
quality_tier = 4
speed_tier = 3
strengths = ["code_review", "complex_reasoning", "documentation"]
weaknesses = ["web_search"]

[models.local-llama]
provider = "ollama"
id = "llama3.2:8b"
context_window = 128_000
quality_tier = 2
speed_tier = 4
strengths = ["quick_completion", "private_data"]
weaknesses = ["complex_reasoning", "web_search"]
is_local = true
```

The task classifier emits a `TaskType`. The router filters models whose `strengths` list contains that type, then picks based on routing priority (quality / balanced / cost / local-first). `--explain` output shows: *"Selected gemini-flash: supports web_search, speed_tier=5, cost=$0.0003"*.

Adding a new Gemini release = one new `[models.gemini-X]` block in `models.toml` + one new entry in `costs.toml`. Zero recompile.

---

## Smart routing

### Routing inputs

```
query text  →  task classifier (regex + heuristics, v1)
user profile (what they use AI for, set in init)
routing priority (quality / balanced / cost / local-first)
available backends (detected at init)
model registry (strengths/weaknesses per model)
token budget remaining (optional per-project cap)
```

### Default routing rules (all overridable)

```toml
# ~/.config/zedplus/config.toml  or  .zedplus.toml (project wins)

[routing.priority]
mode = "balanced"   # quality | balanced | cost | local-first

[routing.rules]
web_search        = "gemini-flash"    # keyword: search/latest/news
quick_completion  = "local"           # short, fast, zero cost
code_review       = "claude-sonnet"   # keyword: review/audit/refactor
complex_reasoning = "claude-sonnet"   # keyword: design/architect/explain
data_analysis     = "gemini-pro"      # keyword: analyze/csv/dataframe/plot
documentation     = "claude-haiku"    # keyword: docs/readme/comment
fallback          = "claude-haiku"    # everything else

[routing.fallback_chain]
local_failure    = "claude-haiku"    # if local errors/times out, retry with haiku
timeout_secs     = 30               # give up on local after 30s

[routing.overrides]
# Pin patterns to specific models (glob on file path or query text)
# "review **/migrations/*.sql" = "claude-opus"
# "ask *secret*"               = "local"         # never send to cloud
```

### Adaptive routing (learns from usage)

Every manual model override (`--model X`) is recorded in the usage table. After 5+ overrides of the same task type to the same model, ZedPlus surfaces a suggestion:

```
$ zedplus profile --optimize

Analyzing last 90 days (1,240 queries)...

Suggested routing changes:
  data_analysis  →  gemini-pro-2-5   (you've overridden to this 23/40 times)
  documentation  →  local            (no negative signals in 67 queries, saves ~$0.40/mo)

Apply? [Y/n/edit]
```

Implicit negative signal: if a user re-asks within 30s of a response, the prior response is flagged as likely unsatisfactory and down-weights that model for that task type.

### Override at call time

```
zedplus ask "..." --model claude-opus    # explicit model
zedplus ask "..." --local               # force local
zedplus ask "..." --cheap              # force cheapest available
```

### Cost-impact display (optional, `--explain`)

```
$ zedplus ask "review this auth module" --explain

  Routing: claude-sonnet-4-6
  Reason:  task=code_review, priority=balanced, strengths match
  Context: 1,840 input tokens (index pruned from 12 files → 3 chunks)
           + git diff HEAD (42 lines of staged changes)
  Est. cost: $0.006   [cheapest alt: claude-haiku $0.002, quality diff: ~30%]

  Continue? [Y/n/use-haiku]
```

---

## Token efficiency

High quality + low tokens is the core efficiency constraint. Five strategies:

**1. Context pruning via index**
Never send whole files. The SQLite index finds the top-K most similar chunks to the query using cosine similarity on embeddings. Only those chunks go into context.

*v1 note:* Cosine similarity on code embeddings is imperfect — it finds lexically similar code, not always semantically relevant code. v2 will add a reranker pass (BM25 hybrid or lightweight cross-encoder) before final context assembly.

**2. Prompt caching**
Claude supports cached system prompts (90% discount on cache hits). The system prompt (user profile + routing rules + project context) is sent once and cached. Subsequent turns hit the cache.

**3. Model tiering**
Default to the cheapest model that can handle the task. Escalate only when the task classifier signals high complexity. A "write a docstring" query never touches Opus.

**4. Per-task token budgets**
Optional cap per task type in config. When a query would exceed budget, ZedPlus truncates context or downgrades model tier before sending.

**5. Session summarization**
Long multi-turn sessions are summarized (locally, using a small local model or Haiku) before the context window fills. The summary replaces raw history.

---

## Streaming responses

All `zedplus ask` output streams by default. Every backend (Claude SSE, Gemini chunked, Ollama streaming) delivers tokens as they arrive — the terminal renders progressively. For slow local models (7B on CPU) this is critical for perceived responsiveness; without streaming the user stares at a blank prompt for 30–90 seconds.

```
zedplus ask "..." --no-stream    # collect full response before printing (for piping)
```

---

## Git awareness

The indexer reads git state alongside code content. When a query is detected as review/diff/change related, git context is injected automatically:

```
zedplus ask "review my changes"
# automatically includes: git diff HEAD (staged + unstaged)
# plus: relevant indexed chunks near changed lines
```

`src/indexer/git.rs` wraps `git2` crate for cross-platform git operations. Git context is always local — it is never sent to cloud models if `privacy.cloud_allowed = false`.

---

## Privacy boundaries

### Per-query

```
zedplus ask "..." --local    # force local for this query
```

### Per-project (persistent)

Add to `.zedplus.toml` at the project root:

```toml
[privacy]
cloud_allowed = false   # all cloud backends disabled for this project
```

When `cloud_allowed = false`, all cloud routing is blocked, local unavailability shows an error, and `--explain` labels the constraint. Useful for medical data, financial code, proprietary IP.

---

## Distillation loop and local model training

### Two distinct data stores (never conflated)

| Store | Location | What it holds | Cleared by `zedplus clear`? |
|---|---|---|---|
| Session context | in-memory | multi-turn conversation history, in-flight chunks | **Yes** |
| Distillation JSONL | `{data_dir}/distill/YYYY-MM.jsonl` | every AI response ever, Alpaca format | **Never** |
| SQLite index | `{data_dir}/index.db` | code chunks + embeddings | Only with `--reset` |
| SQLite usage | `{data_dir}/usage.db` | cost/token history | Only with explicit purge |

`zedplus clear` wipes only the current session's conversation context. All distillation data is append-only and permanent.

### Full training flow

```
Every AI call
  └─→ Distiller appends to {data_dir}/distill/YYYY-MM.jsonl  (immediate, atomic)

Export (recency-weighted by default):
  zedplus distill --weighted --out training.jsonl
  # last 30d = 1.0×, 30–90d = 0.5×, 90d+ = 0.25×

  Other filters:
  zedplus distill --task code_review       # specialist model
  zedplus distill --model claude-sonnet    # only top-quality responses
  zedplus distill --since 2026-01-01       # date range

Fine-tune (outside ZedPlus, Unsloth/Axolotl):
  unsloth --data training.jsonl --base llama3.3:8b --lora --output ./my-lora

Import result:
  zedplus model import ./my-lora --name my-llama --as local
  └─→ registers in model_registry table + models.toml

Benchmark:
  zedplus bench --model my-llama --baseline llama3.2:8b

Update routing if benchmark shows improvement:
  zedplus config --set routing.rules.quick_completion=my-llama
```

### Transferring learning across base model upgrades

The JSONL is the portable knowledge artifact — not the model weights. When a new base model is released (e.g., Llama 4 8B), re-fine-tune on the same accumulated JSONL. The full history of the user's AI interactions carries forward; each generation starts from the compounded dataset, not from scratch.

---

## Auto-training

When the user opts in during `zedplus init`, training runs automatically in the background when:
1. New conversations since last training ≥ `auto_train_min_new` (default: 200)
2. System is idle: CPU < 20% for 5 consecutive minutes, no active `zedplus` processes

**Method:** LoRA-only by default (not full fine-tune). LoRA on a 7B model takes 20–45 minutes vs 4–12 hours for full fine-tune, uses ~6 GB VRAM, and produces an adapter that merges at inference time. Full fine-tune is available via `zedplus train --full` for users who want it explicitly.

If the device lacks a training-capable GPU (< 6 GB VRAM), auto-training is skipped silently; distillation JSONL still accumulates for external use.

```
[ZedPlus] Auto-training triggered: 247 new conversations since last run
  Base model:  llama3.2:8b
  Dataset:     1,840 examples (recency-weighted from 3,200 total)
  Method:      LoRA (r=16, alpha=32)
  ETA:         ~22 minutes
  GPU usage:   ~8 GB VRAM

  Training in background — active queries route to base model until done.
  Run `zedplus train --status` to monitor.
```

Auto-training config:
```toml
[training]
auto_train = true
auto_train_min_new = 200      # conversations since last run
auto_train_schedule = "volume" # volume | weekly | manual
lora_rank = 16
lora_alpha = 32
```

---

## Local model evaluation

### Benchmark set

ZedPlus holds out a random 10% of the distillation JSONL as a frozen benchmark set (`{data_dir}/bench/benchmark.jsonl`). Frozen so comparisons across training runs are stable.

### Scoring

No external eval API. Two lightweight heuristics reusing existing infrastructure:

- **Similarity score:** cosine similarity between the new output's embedding and the gold output's embedding, using the same `nomic-embed-text` Ollama model already running for code indexing
- **Length ratio:** output length / gold length — flags truncated or bloated outputs

```
$ zedplus bench --model my-llama --baseline llama3.2:8b

Benchmarking my-llama vs llama3.2:8b (120 held-out examples)...

Task type        | my-llama | baseline | delta
code_review      |   0.82   |   0.71   | +15.5% ✓
documentation    |   0.78   |   0.76   | +2.6%
quick_completion |   0.91   |   0.89   | +2.2%
data_analysis    |   0.63   |   0.68   | -7.4% ✗  (regression — keep baseline for this task)

Overall: +3.2% improvement
Recommendation: route code_review → my-llama, keep data_analysis → llama3.2:8b
```

Benchmark results are stored in `bench_results` SQLite table with model name + timestamp — improvement history is preserved across all training runs.

---

## Update system

### ZedPlus binary updates

The CLI binary is versioned and auto-updates are opt-in (default: notify only):

```
zedplus update           # download + install latest from GitHub Releases
zedplus update --check   # check version only, no install
```

Notification on startup (if enabled):
```
ZedPlus 0.4.1 available — run `zedplus update` to install.
```

`costs.toml` and `models.toml` ship with the binary release, so new model pricing and capabilities arrive with normal updates.

### Community LoRA adapters (v2)

The ZedPlus team will publish general-purpose LoRA adapters trained on public coding data (GitHub, Stack Overflow). These improve the base local model for common coding tasks without touching user data.

```
zedplus model adapters list                       # show available adapters
zedplus model adapters install code-review-v2    # download + activate
```

Adapters are versioned, signed, and reviewable — analogous to VS Code extensions. Deferred to v2 (requires CDN + signing infrastructure).

---

## Privacy and scale

### Opt-in community contribution (v2)

Users who want to improve the community model can export an anonymized subset of their JSONL:

```
zedplus distill --export-community --review
# shows exactly what would be shared before confirming
```

Auto-filters applied before export:
- Strips file paths, project names, identifiers (regex scrub)
- Removes queries containing secrets, tokens, passwords (keyword detection)
- User reviews the filtered output before upload

The ZedPlus team trains community LoRA adapters on the aggregated dataset and publishes them back to all users.

### Why not federated learning

Federated learning (local training, gradient aggregation) is privacy-preserving in theory but requires: a coordination server, gradient compression, differential privacy noise, secure aggregation, and all users running the same base model + training config simultaneously. This is research-grade infrastructure inappropriate for a v1 CLI. Deferred indefinitely.

### v1 approach

ZedPlus generates training data synthetically using Claude/Gemini on public coding tasks and publishes community LoRAs trained on that. Zero user data involved. Binary updates ship new model pricing and capability definitions. Users improve their personal local model entirely locally.

---

## Known v1 gaps (roadmap)

These are design decisions deferred to future versions — noted here so they aren't forgotten.

| Gap | Risk | v2 fix |
|---|---|---|
| Context retrieval is cosine-similarity only | Retrieves lexically similar chunks, not always semantically relevant | BM25 hybrid + lightweight reranker pass |
| No `zedplus undo` | User applies a bad suggestion, no recovery path | Always suggest `git add` first; `zedplus undo` reverses last applied change |
| No explicit `RateLimitError` in Backend trait | Rate limit hit → crash, not graceful degradation | Add `RateLimitError` variant, router downtiers automatically |
| Community LoRA adapter publishing | No central improvement distribution for local models | CDN + signing infrastructure in v2 |
| Opt-in JSONL contribution | No community dataset | Scrub + review pipeline in v2 |

---

## Component diagram

```
┌─────────────────────────────────────────────────────────────────┐
│                        CLI (clap)                               │
│  init | index | ask | search | usage | distill | train | bench  │
│  model | config | profile | update                              │
└──────────┬──────────────────────────────────────────────────────┘
           │
           ▼
┌──────────────────┐     ┌──────────────────────────────────────┐
│  Setup Wizard    │     │  Smart Router                        │
│  (inquire TUI)   │     │  task classifier                     │
│  service detect  │     │  model registry (models.toml)        │
│  device scan     │     │  cost model (costs.toml)             │
│  keychain store  │     │  adaptive rules (usage signals)      │
│  auto-train opt  │     │  fallback chain                      │
└──────────────────┘     └──────────────┬─────────────────────┘
                                        │
           ┌──────────────────┐         │
           │  Indexer         │         ▼
           │  notify watcher  │  ┌─────────────────────────────┐
           │  tree-sitter     │  │  AI Backends (streaming)    │
           │  git.rs (diff)   │  │  claude.rs   (+ caching)    │
           │  Ollama embed    │  │  gemini.rs   (+ Search)     │
           │  SQLite store    │  │  openai.rs                  │
           │  chunk retrieval │→→│  ollama.rs   (local)        │
           └──────────────────┘  └──────────────┬──────────────┘
                                                │
                                                ▼
                                 ┌──────────────────────────────┐
                                 │  Distiller                   │
                                 │  JSONL writer (Alpaca fmt)   │
                                 │  usage tracker (SQLite)      │
                                 │  override signal recorder    │
                                 └──────────────┬───────────────┘
                                                │
                                                ▼
                                 ┌──────────────────────────────┐
                                 │  Trainer / Bench             │
                                 │  recency-weighted export     │
                                 │  LoRA fine-tune orchestrate  │
                                 │  benchmark scoring           │
                                 │  bench_results (SQLite)      │
                                 └──────────────────────────────┘
```

---

## Module layout

```
src/
  main.rs
  setup/
    mod.rs          — init wizard orchestration
    detector.rs     — device scan (RAM, GPU, Apple Silicon, training feasibility)
    services.rs     — API key prompting + validation
    profile.rs      — user use-case + priority + auto-train questions
  indexer/
    mod.rs
    watcher.rs      — notify wrapper, debounce
    parser.rs       — tree-sitter, per-language
    embedder.rs     — Ollama nomic-embed-text
    store.rs        — SQLite schema, upsert, similarity search
    git.rs          — NEW: git2 wrapper, diff/staged context injection
  router/
    mod.rs          — routing decision pipeline
    classifier.rs   — task type from query text
    cost.rs         — cost model, token estimation
    rules.rs        — rule loading + override resolution
    adaptive.rs     — NEW: usage-pattern analysis, routing suggestions
  backends/
    mod.rs          — Backend trait (includes RateLimitError, streaming)
    claude.rs       — Anthropic API + prompt caching + SSE streaming
    gemini.rs       — Gemini API + Search grounding + chunked streaming
    openai.rs       — OpenAI API + streaming
    ollama.rs       — Ollama local API + streaming
  distiller/
    mod.rs          — wraps every AI call, appends JSONL, records usage + overrides
    bench.rs        — NEW: benchmark set mgmt, scoring, bench_results table
    trainer.rs      — NEW: recency-weighted export, LoRA orchestration, auto-train scheduler
  config/
    mod.rs          — merge global + project config
    schema.rs       — serde structs for .zedplus.toml / config.toml
    costs.rs        — bundled pricing table (costs.toml), user-overridable
    models.rs       — NEW: model capability registry (models.toml)
  platform/
    mod.rs          — platform detection helpers
    secrets.rs      — keyring wrapper (OS keychain)
    dirs.rs         — XDG / AppData / Library path resolution
    update.rs       — NEW: binary version check + self-update
```

---

## Data formats

### SQLite schema

```sql
CREATE TABLE files (
  path TEXT PRIMARY KEY,
  hash TEXT NOT NULL,
  indexed_at INTEGER NOT NULL
);

CREATE TABLE chunks (
  id INTEGER PRIMARY KEY,
  file_path TEXT NOT NULL,
  symbol TEXT,
  content TEXT NOT NULL,
  embedding BLOB NOT NULL        -- f32 array, raw bytes
);

CREATE TABLE usage (
  id INTEGER PRIMARY KEY,
  ts INTEGER NOT NULL,
  model TEXT NOT NULL,
  task_type TEXT,
  input_tokens INTEGER,
  output_tokens INTEGER,
  cost_usd REAL,
  cache_hit INTEGER DEFAULT 0,   -- 1 if Claude prompt cache hit
  override_model TEXT,           -- set if user passed --model explicitly
  negative_signal INTEGER DEFAULT 0  -- 1 if re-asked within 30s
);

CREATE TABLE bench_results (
  id INTEGER PRIMARY KEY,
  ts INTEGER NOT NULL,
  model TEXT NOT NULL,
  task_type TEXT,
  example_id TEXT NOT NULL,      -- hash of the benchmark query
  similarity_score REAL,
  length_ratio REAL,
  baseline_model TEXT
);

CREATE TABLE model_registry (
  name TEXT PRIMARY KEY,         -- user-facing alias
  provider TEXT NOT NULL,
  model_id TEXT NOT NULL,
  path TEXT,                     -- local path for .gguf or LoRA adapter
  imported_at INTEGER,
  last_trained_at INTEGER,       -- last time a training run used this as base
  is_active INTEGER DEFAULT 1
);

CREATE TABLE test_runs (
  id INTEGER PRIMARY KEY,
  ts INTEGER NOT NULL,
  runner TEXT NOT NULL,              -- "cargo-test" | "pytest" | "npm-test" | etc.
  triggered_by TEXT,                 -- path of file that changed
  passed INTEGER NOT NULL,           -- count of passing tests
  failed INTEGER NOT NULL,
  duration_ms INTEGER,
  output TEXT                        -- truncated stdout/stderr
);

CREATE TABLE bench_perf (
  id INTEGER PRIMARY KEY,
  ts INTEGER NOT NULL,
  benchmark_name TEXT NOT NULL,
  duration_ns INTEGER,
  triggered_by TEXT,                 -- path of file that changed
  delta_pct REAL                     -- % change vs prior run (NULL on first run)
);

CREATE TABLE train_jobs (
  id INTEGER PRIMARY KEY,
  started_at INTEGER NOT NULL,
  finished_at INTEGER,
  base_model TEXT NOT NULL,
  output_model TEXT,
  dataset_size INTEGER,
  method TEXT,                   -- "lora" | "full"
  status TEXT,                   -- "running" | "done" | "failed"
  benchmark_delta REAL           -- overall score delta vs baseline
);
```

### Distillation JSONL (Alpaca format, append-only)

```json
{"instruction": "...", "input": "...", "output": "...", "meta": {"model": "claude-sonnet-4-6", "task": "code_review", "ts": 1747267200, "tokens_in": 820, "tokens_out": 340}}
```

---

## Interactive REPL (primary mode)

Running `zedplus` with no arguments opens an interactive conversation window — the same pattern as Claude Code and Gemini CLI. This is the primary human interface. Subcommands (`zedplus ask`, `zedplus index`, etc.) are the programmatic/scripting interface.

### Entry modes

```
zedplus                         # interactive REPL — primary mode
zedplus "what does X do?"       # REPL with first query pre-loaded, then continues
zedplus ask "..."               # non-interactive: print response and exit (for pipes/scripts)
```

### REPL startup

On open, session resume logic runs (see Session persistence section). The header shows the current routing context:

```
╭─ ZedPlus ────────────────────────────────────────────────────────╮
│  Resumed "auth-options-debug"  ·  2h ago  ·  4 turns  ·  $0.03  │
│  balanced  ·  local → llama3.2:8b  ·  review → claude-sonnet    │
╰──────────────────────────────────────────────────────────────────╯

> _
```

For a new session (no resume):
```
╭─ ZedPlus ────────────────────────────────────────────────────────╮
│  New session  ·  my-project  ·  branch: feature/auth             │
│  balanced  ·  local → llama3.2:8b  ·  review → claude-sonnet    │
╰──────────────────────────────────────────────────────────────────╯

> _
```

### Autosuggest and real-time routing preview

**Slash command completion**
Type `/` → filtered dropdown of available commands with descriptions, updated as you type. Standard pattern from Claude Code — copy it directly.

**File path tab completion**
After `--image`, `--file`, or `@` prefix: filesystem tab completion using the standard readline/crossterm input handler.

**Query history**
Up/down arrow scrolls previous queries within the current session. Stored in `session_turns` table — history persists across REPL restarts.

**Real-time routing preview** *(unique to ZedPlus)*
The task classifier runs on the partial query as the user types (debounced 300ms). A status line below the input updates live:

```
> review the auth midd|

  [route: claude-sonnet · code_review · est. $0.006]
```

As the user adds or changes words, the predicted route updates. Before hitting Enter, the user already knows which model will handle the query and the estimated cost. They can type `/cheap` or `/local` and watch the indicator change immediately — no surprises after sending.

This is only possible because ZedPlus has a local task classifier. Claude and Gemini have nothing to predict — they have one model.

**Post-response contextual suggestions**
After each response, ZedPlus shows 2–3 relevant follow-up slash commands based on what just happened:

```
  ↳  /test  — run tests against the change above
     /explain  — show why claude-sonnet was chosen
     /distill  — save this as a training example
```

The classifier drives these — a code_review response suggests `/test`; a web_search response suggests `/search` for a follow-up; an expensive response suggests `/cheap` for the next query.

**Cost nudges**
When cumulative session cost crosses a configurable threshold (default: $0.50):
```
  $0.52 this session — switch to local for routine queries?
  [/local next]  [/local always]  [dismiss]
```

Shown once per threshold crossing, not repeated until the next threshold.

### Slash commands (REPL only)

Slash commands expose all subcommand functionality without leaving the conversation. Per-query flags (`/local`, `/explain`, `/model X`) apply to the next query only, then reset.

| Command | Effect |
|---|---|
| `/help` | list slash commands |
| `/index` | trigger background indexing |
| `/usage` | show cost + token report |
| `/distill` | export JSONL |
| `/train` | trigger local model training |
| `/bench` | benchmark local model |
| `/models` | show model list with capabilities |
| `/sessions` | session browser |
| `/skills` | skill packs list |
| `/config` | show current config |
| `/clear` | reset context (session + JSONL preserved) |
| `/explain` | show routing decision for next query |
| `/local` | force local model for next query |
| `/cheap` | force cheapest model for next query |
| `/model <name>` | override model for next query |
| `/scope broad` | relax minimal-footprint for next query |
| `/exit` | exit with one-line summary |

### Exit

Ctrl+C or `/exit`:
```
  Session "auth-options-debug" saved  ·  14 turns  ·  $0.04 total
  resume: zedplus resume
```

---

## CLI surface

```
# ── Primary entry point ────────────────────────────────────────────
zedplus                                   # interactive REPL (primary human interface)
zedplus "what does X do?"                 # REPL with first query pre-loaded
zedplus init                              # setup wizard (first-time or re-configure)
zedplus auth [--provider X]               # re-authenticate a provider
zedplus auth --revoke <provider>          # remove stored credentials

# ── Programmatic / scripting interface ────────────────────────────
zedplus ask   "<query>" [--model X]       # non-interactive: print response and exit
zedplus ask   "<query>" --explain         # show routing decision + cost estimate
zedplus ask   "<query>" --local           # force local model
zedplus ask   "<query>" --cheap           # force cheapest available
zedplus ask   "<query>" --no-stream       # collect full response before printing
zedplus ask   "<query>" --image ./img.png # attach image (vision models)
zedplus ask   "<query>" --file ./doc.pdf  # attach file
zedplus ask   "<query>" --scope narrow    # strict: answer only what was asked
zedplus search "<query>"                  # force Gemini + web grounding
zedplus resume                            # resume most recent session in current dir
zedplus clear                             # clear session context (distillation data preserved)
zedplus usage  [--today | --month | --project]   # cost + token report

zedplus distill [--out file] [--format alpaca|sharegpt]
zedplus distill --weighted --out file     # recency-weighted export for training
zedplus distill --task X [--model X] [--since DATE]  # filtered export
zedplus distill --export-community --review  # opt-in anonymized community contribution

zedplus train [--base model] [--data file] [--lora | --full]  # manual training run
zedplus train --status                    # monitor background training job

zedplus bench [--model X] [--baseline Y] # evaluate local model vs baseline

zedplus model list                        # all known models + capability summary
zedplus model add <provider> <id>         # scaffold models.toml entry for new model
zedplus model import <path|ollama-id> --name X  # register local/LoRA model
zedplus model adapters list               # community LoRA adapters (v2)
zedplus model adapters install <name>     # download + activate adapter (v2)

zedplus profile --optimize [--apply]      # suggest routing changes from usage patterns

zedplus config  [--show | --edit | --reset]
zedplus config  --set routing.rules.code_review=claude-opus

zedplus update [--check]                  # check or install binary updates
zedplus shell  "<description>"            # generate + confirm + run a shell command
zedplus init --context                    # generate starter ZEDPLUS.md from codebase

zedplus ask   "<query>" --architect       # force architect/editor split
zedplus ask   "<query>" --no-interactive  # headless mode for CI
zedplus ask   "<query>" --output json     # structured output for scripts
zedplus ask   "<query>" --image ./screenshot.png   # multimodal: attach image
zedplus ask   "<query>" --file ./diagram.pdf        # multimodal: attach file
zedplus ask   "<query>" --scope narrow              # strict: answer only what was asked

zedplus skills list                       # all installed + available skill packs
zedplus skills install react-developer    # install a skill pack
zedplus skills suggest                    # AI-driven skill suggestions from usage
zedplus skills create --name my-skill     # scaffold a custom skill
```

---

## Multimodal inputs

ZedPlus supports image and file attachments at the CLI level. The model registry's `supports_vision = true` field drives routing — when an attachment is present, the router enforces a vision-capable model.

### Image inputs

```
zedplus ask "what's wrong in this screenshot?" --image ./error.png
zedplus ask "implement this wireframe" --image ./mockup.png
```

Accepted formats: PNG, JPEG, GIF, WebP — passed as base64 to API backends that support it.

If the currently routed model doesn't support vision (e.g., `local` was selected), the router falls back to the cheapest vision-capable cloud model automatically and notes this in the output.

### File inputs

```
zedplus ask "summarize this spec" --file ./requirements.pdf
zedplus ask "review this data" --file ./export.csv
```

- **PDF:** Gemini backends support native PDF; Claude accepts base64. For Ollama, text is extracted and injected as context.
- **CSV / plain text:** Read and injected as context directly.
- **Other binary:** Error with clear message — only PDF, images, and text files are supported.

### Permission model for included files

ZedPlus follows the Claude/Gemini pattern: only ask about files the user didn't explicitly provide.

- **Explicitly attached** (`--image ./file.png`, `--file ./data.csv`): no prompt. The flag is the consent.
- **Auto-detected** (clipboard image, skill pack `always_include`, indexer context injection): show a confirmation before reading.

```
  Including in context:
    [file] package.json      (react-developer skill pack)
    [file] tsconfig.json     (react-developer skill pack)
    [img]  clipboard image   (1024×768, detected)

  Continue? [Y/n/skip-clipboard]
```

This avoids surprise data exposure without being annoying for explicitly-provided attachments.

### Clipboard detection (opt-in)

If `clipboard_detection = true` in config, `zedplus ask` checks the clipboard on invocation. If it contains an image (screenshot), it's automatically included as an attachment — no `--image` flag needed. This matches the workflow of screenshotting an error and immediately asking about it.

### Model routing for multimodal

```toml
# in models.toml
[models.gemini-flash-2-5]
supports_vision = true
supports_pdf = true

[models.claude-sonnet-4-6]
supports_vision = true
supports_pdf = false   # Claude accepts images but not native PDF

[models.local-llama]
supports_vision = false
```

Router logic: if `--image` or `--file .pdf` is present, filter to `supports_vision = true` models before applying the normal routing logic.

---

## Session persistence and resume

Sessions are continuously auto-saved. There is no "save before exit" step and no session IDs visible to the user.

### Session identity

Sessions are scoped by **project directory + git branch**. This means:
- Switching to a different branch automatically offers the session from that branch last time
- No ID to remember — the directory and branch *are* the lookup key
- `zedplus resume` with no args always means "the most recent session here"

Sessions are named automatically from the first user query — a cheap local/haiku call generates a 3–5 word human-readable slug:
- "why is auth middleware blocking OPTIONS requests?" → `auth-options-debug`
- "implement pagination for the user list API" → `pagination-user-list`

The UUID is internal only, stored in SQLite, never shown.

### Resume prompt (at `zedplus ask` start, not at init)

The resume question appears when you're actually about to work, not during setup days earlier.

**Single obvious candidate** (same dir, same branch, within 24h):
```
  Resume "auth-options-debug" from 2h ago? [Y/n]
```

**Multiple candidates or stale session:**
```
┌─ Resume ──────────────────────────────────────────────────────────┐
│  auth-options-debug    2h ago  · 4 turns · $0.03  (this branch)  │
│  pagination-impl       3d ago  · 12 turns · $0.18                │
│  > Start new session                                              │
└────────────────────────────────────────────────────────────────────┘
```

Sessions older than 7 days are not offered for resume by default (configurable). They remain accessible via `zedplus session list`.

### Exit message — one line

```
  Session "auth-options-debug" saved  ·  14 turns  ·  $0.04 total
  resume: zedplus resume
```

No UUID. No per-model token breakdown (that lives in `zedplus usage`). One line is enough — the user already knows what they were doing.

### `clear` vs session archive

```
zedplus clear          # clears in-memory context for the current turn only
                       # session history in SQLite is preserved
                       # distillation JSONL is never touched

zedplus session archive <name>   # soft-deletes the session (hidden from resume list)
                                 # distillation JSONL still preserved for training
```

`clear` is a narrow operation — it resets the live context window so the AI starts fresh on the next query, without losing the record of what happened.

### Session data model

```sql
CREATE TABLE sessions (
  id TEXT PRIMARY KEY,             -- internal UUID, never shown to user
  name TEXT,                       -- auto-derived slug, or user-renamed
  project_path TEXT NOT NULL,
  git_branch TEXT,
  started_at INTEGER NOT NULL,
  last_active INTEGER NOT NULL,
  turn_count INTEGER DEFAULT 0,
  total_cost_usd REAL DEFAULT 0,
  status TEXT DEFAULT 'active'     -- active | archived
);

CREATE TABLE session_turns (
  id INTEGER PRIMARY KEY,
  session_id TEXT NOT NULL,
  ts INTEGER NOT NULL,
  role TEXT NOT NULL,              -- user | assistant
  content TEXT NOT NULL,
  model TEXT,
  tokens_in INTEGER,
  tokens_out INTEGER
);
```

### Session CLI surface

```
zedplus resume                        # resume most recent session in current dir
zedplus session list                  # sessions for this project
zedplus session list --all            # across all projects
zedplus session resume <name>         # resume by name
zedplus session rename <old> <new>    # override auto-generated name
zedplus session archive <name>        # hide from resume list (data preserved)
```

### Config

```toml
[sessions]
auto_resume_threshold_hours = 24   # offer auto-resume if last active within N hours
max_resume_candidates = 3          # max sessions shown in resume picker
```

---

## Staying on task: goal anchoring and minimal footprint

**The problem:** AI models optimizing beyond the scope of a request is a real failure mode — fixing what was asked, then also refactoring nearby code, removing "redundant" features, renaming things, introducing new abstractions. The original ask gets buried. ZedPlus is designed to prevent this.

### Design principles (encoded in system prompt and CLI behavior)

**1. Original ask is always in context**
In multi-turn sessions, the very first user message of the current task is prepended verbatim to every subsequent turn. The model can't forget the original goal because it's always at the top of the prompt, not summarized away.

**2. Minimal footprint by default**
The system prompt explicitly instructs models:
> "Answer only the specific question asked. Do not refactor, rename, reorganize, or modify code beyond what was explicitly requested. If you notice adjacent issues, mention them briefly at the end — do not fix them."

This is the default. The user can relax it:
```
zedplus ask "..." --scope broad    # allow AI to suggest adjacent improvements
```

**3. Scope narrow by default**
```toml
[behavior]
default_scope = "narrow"   # narrow | broad
```

`narrow` = answer only what was asked, note but don't fix anything else.
`broad` = AI may suggest and implement adjacent improvements (with user confirmation per change).

**4. Change confirmation for code modifications**
When the AI proposes code changes, ZedPlus always shows a diff before applying:
```
  Proposed change to src/auth.rs (+12 / -3 lines):
  [diff output]

  Apply? [Y/n/edit/explain]
```
The user sees exactly what will change. Scope creep is visible before it happens.

**5. Task decomposition with explicit approval**
For multi-step requests (`zedplus task`), ZedPlus shows the planned steps before executing any of them:
```
$ zedplus task "migrate the auth module from JWTs to sessions"

Planned steps:
  1. Remove JWT dependencies from Cargo.toml
  2. Replace JWT generation in src/auth/token.rs
  3. Replace JWT validation in src/middleware/auth.rs
  4. Update session store in src/auth/session.rs
  5. Update tests in tests/auth_test.rs

Proceed? [Y/n/edit steps]
```
No step runs until the user approves the plan. Each step shows a diff before applying.

**6. Distillation captures scope violations**
If a user applies a change and immediately asks "wait, why did you also change X?", that sequence is a negative signal (logged in usage with `negative_signal = 1`). Over time, scope-violating responses are down-weighted in routing and training.

### What ZedPlus learned from watching Claude Code

- Verbatim anchoring of the original instruction prevents drift in long sessions
- "Note but don't fix" is the right default — users want to know about adjacent issues, not have them silently changed
- Showing diffs before applying changes is non-negotiable — it's the only way a user can verify scope wasn't exceeded
- Multi-step tasks need a plan approval gate before execution
- The AI should never delete or rename code that wasn't mentioned in the request

---

## Skill packs

A skill pack is a named configuration bundle that tunes ZedPlus for a specific domain or workflow. Think VS Code extensions but for AI routing and context injection.

### What a skill pack contains

A TOML file in `~/.config/zedplus/skills/<name>.toml`:

```toml
[skill]
name = "react-developer"
version = "1.2.0"
description = "Optimized for React/Next.js frontend development"

[routing_overrides]
code_review       = "claude-sonnet"
quick_completion  = "local"
documentation     = "claude-haiku"

[context_injection]
# Always include these files in context when present in the project
always_include = ["package.json", "tsconfig.json", "next.config.*"]

[task_types]
# Custom task types added by this skill
[task_types.component_generation]
keywords = ["create component", "new component", "generate component"]
route_to = "claude-sonnet"
system_prompt_append = "Follow React best practices. Use TypeScript. Use functional components with hooks."

[system_prompt_append]
global = "This project uses React 19, Next.js 15, and TypeScript. Prefer server components where possible."
```

### Built-in skill packs (bundled)

| Pack | Best for |
|---|---|
| `react-developer` | React/Next.js frontend |
| `python-data` | pandas, numpy, Jupyter notebooks |
| `devops` | Docker, Kubernetes, CI/CD scripts |
| `rust-systems` | Low-level Rust, unsafe, performance |
| `mobile-flutter` | Flutter/Dart cross-platform |
| `security-review` | Focused on OWASP, auth, secrets |

### Usage-based skill suggestions

ZedPlus watches task patterns and file types in the project index. After enough data:

```
$ zedplus skills suggest

Based on your usage (last 30 days):

  You've asked 47 questions about React components and 12 about Next.js routing.
  Suggestion: install react-developer skill pack
    → adds component_generation task type, tunes routing for JSX/TSX files
    → estimated to improve response quality ~15% for your common task types

Install? [Y/n]
```

The suggestion engine looks at:
- Most common `task_type` values in usage table
- File extensions most frequently appearing in indexed context (`.tsx`, `.py`, `.go`, etc.)
- Most frequent query keywords

### Skill library updates

Skill packs ship with the ZedPlus binary (bundled) and update via `zedplus update`. Community-contributed packs will be available via `zedplus skills install <name>` in v2 (requires the same CDN infrastructure as community LoRA adapters).

### Creating custom skills

```
zedplus skills create --name my-company
# scaffolds ~/.config/zedplus/skills/my-company.toml
# opens in $EDITOR
```

Custom skills can define proprietary task types, inject company-specific system prompt context (e.g., "This codebase follows our internal API design guide"), and override routing for specific file patterns.

---

## Architect/editor mode

Borrowed and extended from Aider — the most cost-effective pattern for code tasks. A high-quality model plans the change; a cheap/fast model applies it as diffs. Result: 30–50% cost reduction with equivalent output quality for most code tasks.

### How it works

```
User: "refactor the auth module to use sessions instead of JWTs"

Phase 1 — Architect (claude-sonnet or gemini-pro)
  → Reads relevant code from the index
  → Produces a structured change plan: which files, what changes, in what order
  → Returns a plan, not code

Phase 2 — Editor (claude-haiku or local model)
  → Receives the plan + relevant file contents
  → Produces precise diffs, file by file
  → Applies each diff with user confirmation
```

The architect never writes code. The editor never reasons — it only applies. This separation is the insight: frontier models are expensive because of reasoning; cheap models are fast at mechanical translation.

### Config

```toml
[routing.architect_editor]
enabled = true
architect_model = "claude-sonnet"    # does the reasoning
editor_model    = "claude-haiku"     # applies the diffs (or "local")
threshold_lines = 50                 # only split if change spans > N lines
```

### CLI

```
zedplus ask "..." --architect       # force architect/editor mode
zedplus ask "..." --no-architect    # single-model pass (for simple queries)
```

Auto-enabled for `code_review`, `complex_reasoning`, and `refactor` task types when the change is large enough. Single-model pass stays for `quick_completion`, `documentation`, `web_search`.

### Cost display with --explain

```
  Routing: architect/editor
  Architect: claude-sonnet  →  plan (est. 1,200 tokens, $0.018)
  Editor:    claude-haiku   →  diffs (est. 3,400 tokens, $0.014)
  Total est: $0.032   [vs single-model claude-sonnet: $0.071]
```

---

## ZEDPLUS.md — project context file

A file committed to the repository root that ZedPlus reads at every session start. Captures project-specific knowledge that should always be in context: architecture decisions, conventions, what to avoid, local setup instructions.

Distinct from skill packs (which are domain templates) — ZEDPLUS.md is the project's own truth, version-controlled alongside the code.

```markdown
# ZEDPLUS.md

## Architecture
- Auth is session-based (not JWT) — do not suggest JWT solutions
- All database access goes through the repository pattern in src/db/
- No ORM — raw SQL only, see src/db/queries/

## Conventions
- Error handling: use anyhow for application errors, thiserror for library errors
- No unwrap() outside of tests
- All public APIs must have doc comments

## Local setup
- Requires PostgreSQL 16 running on port 5433 (not default)
- Run `make seed` before first test run

## Do not touch
- src/legacy/ — maintained separately, changes break prod
```

### Loading behaviour

ZedPlus searches for `ZEDPLUS.md` from the current directory upward (same traversal as `.git`). If found, its contents are prepended to the system prompt on every session start.

Multiple files are merged if nested projects exist (parent + child both have `ZEDPLUS.md`).

```
zedplus init --context     # generate a starter ZEDPLUS.md from the codebase index
```

---

## Hooks

User-defined shell commands that run before or after specific ZedPlus actions. Inspired by Claude Code's hooks system — enables automated guardrails without writing Rust.

```toml
# .zedplus.toml or config.toml

[hooks]
before_apply_change = "cargo fmt --check"      # block if formatter would change files
after_apply_change  = "cargo clippy -- -D warnings"  # lint after every AI edit
before_commit       = "cargo test --quiet"     # block commit if tests fail
after_session       = "echo 'Session ended' | notify-send ZedPlus"
```

Hooks run as shell commands in the project directory. A non-zero exit code from a `before_*` hook blocks the action and surfaces the output to the user.

```
  Hook: before_apply_change
  $ cargo fmt --check
  Diff in src/auth/session.rs — run `cargo fmt` first

  ✗ Change blocked by hook. Fix formatting and retry.
```

The user sees what the hook said and why the action was blocked — no silent failures.

### Available hook points

| Hook | Fires when |
|---|---|
| `before_apply_change` | AI proposes a file modification |
| `after_apply_change` | A file modification is confirmed and applied |
| `before_commit` | `zedplus` is about to auto-commit |
| `after_commit` | An auto-commit completes |
| `before_session` | REPL starts |
| `after_session` | REPL exits |
| `before_search` | A web search query is about to be sent |
| `before_cloud_send` | Any data is about to leave the device (privacy hook) |

---

## Shell command mode

`zedplus shell` generates, previews, and optionally executes shell commands from natural language. Separate from `ask` — this is action-oriented, not conversational.

```
$ zedplus shell "archive log files older than 7 days to ./backup"

  Generated:
  find ./logs -mtime +7 -exec mv {} ./backup/ \;

  Run? [Y/n/edit]
```

The generated command is shown before any execution. The user can edit it in-place before confirming. Execution requires explicit confirmation — never auto-runs.

### Shell integration (opt-in)

Add to `.bashrc` / `.zshrc`:
```bash
# ZedPlus shell hotkey — Ctrl+Z in terminal buffer
zedplus_hotkey() { READLINE_LINE=$(zedplus shell "$READLINE_LINE" --inline); }
bind -x '"\C-z": zedplus_hotkey'
```

When activated in the shell buffer: the current typed command (or blank buffer) is sent to ZedPlus shell mode. The result replaces the buffer text. The user sees the suggested command in the prompt and can edit/run it normally. No context switch needed.

ZSH equivalent ships in the install script. Fish shell supported via `bind`.

---

## Headless / CI mode

`zedplus ask` is fully non-interactive when stdout is not a TTY, or when `--no-interactive` is passed. Suitable for CI/CD pipelines, scripts, and automation.

```bash
# GitHub Actions
- name: Review PR diff
  run: |
    git diff origin/main...HEAD | zedplus ask "review these changes for security issues" \
      --no-interactive \
      --model claude-sonnet \
      --output json \
      > review.json
```

Flags available in headless mode:
```
--no-interactive     disable all prompts, use defaults or fail
--output json        structured JSON output instead of terminal rendering
--output plain       plain text, no ANSI codes
--exit-code          exit 1 if AI response contains warnings/errors (for CI gates)
```

`ZEDPLUS_API_KEY_ANTHROPIC`, `ZEDPLUS_API_KEY_GOOGLE` environment variables are read in headless mode — no keychain required for CI.

---

## Background test runner

After every AI-made code change, ZedPlus runs the project's existing test suite in the background and surfaces failures immediately. This is the mechanism — not AI-generated tests.

### Why not auto-generate tests

Auto-generating tests from the same AI pass that wrote the code produces tests that confirm the implementation, not verify it. If the code has a bug, the AI's test will encode the same misunderstanding and pass. If the AI also auto-corrects failing tests, it will weaken assertions to make them pass rather than fix the underlying code. The test suite grows, everything passes, and the bugs ship. This is the oracle problem.

**The separation that makes testing meaningful:** the AI writes code; the human authors (or approves) tests. ZedPlus assists the authorship step but doesn't bypass it.

### What ZedPlus does instead

**1. Run real tests after changes**

ZedPlus detects the project's test runner from the file tree:

| Detected file | Test runner |
|---|---|
| `Cargo.toml` | `cargo test` |
| `pytest.ini` / `pyproject.toml` | `pytest` |
| `package.json` (jest/vitest) | `npm test` / `npx vitest` |
| `go.mod` | `go test ./...` |
| `*.ipynb` | schema + snapshot validation (see below) |

After every AI-made file change, the test runner fires as a background job. If tests pass, a subtle indicator shows in the prompt (`✓ tests`). If tests fail, ZedPlus surfaces the failure inline with the changed files highlighted:

```
  ✗ 2 tests failed after last change (src/auth/session.rs)

    FAILED  tests::session_expiry_sets_correct_ttl
    FAILED  tests::session_token_is_unique

  Fix with AI? [Y/n]  |  Show diff? [d]  |  Skip [s]
```

The AI sees the test output + the change it just made and can diagnose whether the test or the code is wrong — presenting both options to the user, not auto-deciding.

**2. Suggest tests after modifying untested code**

After a change, ZedPlus checks which functions/methods were modified (via tree-sitter) and which have no corresponding test coverage (heuristic: function name not found in test files). It then suggests — not writes — test cases:

```
  src/auth/session.rs modified. No tests found for:
    - Session::new()
    - Session::is_expired()

  Suggested test cases (review before writing):
    - Session::new() creates token of expected length
    - Session::new() sets TTL from config
    - Session::is_expired() returns true after TTL elapses
    - Session::is_expired() returns false before TTL elapses

  Write these tests? [Y/n/edit]
```

User reviews the list, edits as needed, then ZedPlus writes them. The human is the oracle — ZedPlus proposes what to test, the user decides whether the behavior described is actually correct.

**3. Data analysis: snapshot and schema testing**

For notebook / pandas / data pipeline work, traditional unit tests are insufficient — correctness means "right output on real data." Two lightweight patterns:

**Schema assertions** — after any transformation, assert the output dataframe has expected columns and types:
```python
# auto-suggested after modifying a transform function
assert set(df.columns) == {"user_id", "event_ts", "value"}
assert df["value"].dtype == float
assert df["event_ts"].dt.tz is not None
```

**Snapshot tests** — run the pipeline on a small fixture dataset, save the output as a golden file. On future runs, assert output matches the golden file. When a change legitimately changes output, the user explicitly approves the new snapshot (`zedplus test --update-snapshots`).

ZedPlus detects `.ipynb` and `pandas`/`polars` usage and suggests these patterns after notebook edits.

**4. Performance benchmarks (opt-in)**

If the project has benchmarks (`cargo bench`, `pytest-benchmark`, `k6` scripts), ZedPlus runs them after changes to code flagged as performance-sensitive (hot paths, data transforms, indexing code). Results are stored in SQLite `bench_perf` table with timestamps — regressions are surfaced:

```
  ⚠ Performance regression detected:
    embedder::batch_embed  was 12ms, now 31ms  (+158%)
    Changed in: src/indexer/embedder.rs (last change)
```

### Configuration

```toml
[testing]
auto_run = true              # run tests after AI changes (default: true)
runner = "auto"              # auto | cargo-test | pytest | npm-test | none
suggest_tests = true         # suggest untested functions (default: true)
run_benchmarks = false       # run performance benchmarks (default: false, expensive)
snapshot_dir = ".zedplus/snapshots"
```

### What this is not

- Not a test framework — ZedPlus uses the project's own test runner
- Not a test generator that bypasses human review — tests are suggested and user-approved
- Not a correctness guarantee — "tests pass" means "the tested behaviors work"; it does not sign off on the full system

---

## Updated module layout (complete)

```
src/
  main.rs
  setup/
    mod.rs          — init wizard orchestration
    detector.rs     — device scan (RAM, GPU, Apple Silicon, training feasibility)
    services.rs     — API key prompting + validation
    profile.rs      — use-case + priority + auto-train + scope questions
  indexer/
    mod.rs
    watcher.rs      — notify wrapper, debounce
    parser.rs       — tree-sitter, per-language
    embedder.rs     — Ollama nomic-embed-text
    store.rs        — SQLite schema, upsert, similarity search
    git.rs          — git2 wrapper: diff, staged context injection
  router/
    mod.rs          — routing decision pipeline
    classifier.rs   — task type from query text + file context
    cost.rs         — cost model, token estimation
    rules.rs        — rule loading, override resolution, skill pack overrides
    adaptive.rs     — usage-pattern analysis, routing suggestions
  backends/
    mod.rs          — Backend trait (streaming, RateLimitError, vision support)
    claude.rs       — Anthropic API + prompt caching + SSE streaming + vision
    gemini.rs       — Gemini API + Search grounding + PDF + chunked streaming
    openai.rs       — OpenAI API + streaming + vision
    ollama.rs       — Ollama local API + streaming
  distiller/
    mod.rs          — JSONL writer, usage tracker, override + negative signals
    bench.rs        — benchmark set mgmt, scoring, bench_results table
    trainer.rs      — recency-weighted export, LoRA orchestration, auto-train
  repl/
    mod.rs          — NEW: interactive REPL loop, input handling, slash command dispatch
    prompt.rs       — NEW: header rendering, status line, crossterm input
    commands.rs     — NEW: slash command registry and execution
    suggest.rs      — NEW: real-time routing preview, post-response suggestions, cost nudges
  context/
    mod.rs          — NEW: ZEDPLUS.md loader, locale/time injection, system prompt assembly
    zedplusmd.rs    — NEW: find + parse ZEDPLUS.md from project root upward
    locale.rs       — NEW: country/timezone/language config, system prompt injection
  hooks/
    mod.rs          — NEW: hook registry, before/after execution, block on non-zero exit
  shell/
    mod.rs          — NEW: natural language → shell command, inline buffer editing
    integration.rs  — NEW: bash/zsh/fish hotkey installer
  tester/
    mod.rs          — NEW: test runner detection, background job dispatch
    runner.rs       — NEW: spawn test process, parse results, write test_runs table
    coverage.rs     — NEW: tree-sitter pass to find untested functions; suggest test cases
    snapshot.rs     — NEW: dataframe schema assertions + snapshot test scaffolding
  skills/
    mod.rs          — NEW: skill pack loading, merging into routing/context
    suggest.rs      — NEW: usage-pattern skill suggestions
    library.rs      — NEW: bundled + installed skill pack registry
  config/
    mod.rs          — merge global + project config
    schema.rs       — serde structs for .zedplus.toml / config.toml
    costs.rs        — bundled pricing table (costs.toml)
    models.rs       — model capability registry (models.toml)
  platform/
    mod.rs          — platform detection helpers
    secrets.rs      — keyring wrapper; stores OAuth tokens and API keys uniformly
    dirs.rs         — XDG / AppData / Library path resolution
    update.rs       — binary version check + self-update
    clipboard.rs    — NEW: cross-platform clipboard image detection
    auth.rs         — NEW: OAuth device flow (Gemini), browser-assist + manual paste (Claude/OpenAI)
```
