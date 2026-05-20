# ZedPlus

A terminal-first AI routing CLI for developers who use multiple AI models and want them unified, context-aware, and cost-controlled.

---

## What it does

ZedPlus sits between you and several AI providers and makes routing decisions so you don't have to. You type a query, it classifies the task, picks the right model, attaches relevant code context, and streams the response. As you override its choices, it notices the patterns and adjusts.

**Routing and providers**
- Classifies queries into task types (code review, web search, reasoning, data analysis, documentation, quick completion, brainstorm) and routes each to its best-fit model
- Supports Claude, Gemini, OpenAI, Ollama, and LM Studio — switch with `@claude`, `@gemini`, `@local`, or `--model`
- Adaptive routing: after 5+ consistent overrides on a task type it suggests a permanent rule change

**Code context**
- Indexes your working directory with tree-sitter — functions, classes, symbols, chunks
- Attaches semantically relevant code snippets to every query automatically
- Pulls live `git diff` context so answers reflect your current changes, not stale file state

**Multi-agent strategies**
- `/debate`, `/red-team`, `/perspectives`, `/delphi` — runs a query across multiple models and synthesizes the responses
- Architect/Editor mode: a high-quality model writes the plan, a fast model implements it

**Training data distillation**
- Every Q&A pair is logged to JSONL in Alpaca format
- Filter and export by task type, date range, or quality signal
- `zedplus train` generates a fine-tuning script for Unsloth or Axolotl against your local base model

**Ergonomics**
- Terminal REPL with session history, auto-resume, and named sessions
- One-shot mode: `zedplus "explain this"` — no REPL required
- `/usage` and `zedplus usage --month` show token and cost totals per provider

---

## What it doesn't do

- **No IDE integration.** ZedPlus is a CLI. There is no VS Code extension, no inline completion, no sidebar. If you want autocomplete in your editor, use Copilot or Supermaven.
- **Not an autonomous agent.** ZedPlus doesn't open a terminal, run tests, read error output, and loop until the code works. It answers queries and can apply code blocks to files, but it does not drive a full agentic loop the way Devin, SWE-agent, or Claude Code do, yet.
- **Doesn't train models.** It generates JSONL and a shell script. The actual GPU training runs in Unsloth or Axolotl on your hardware. ZedPlus doesn't manage that process, yet.
- **No shared workspaces.** Sessions and config are per-user, per-machine. There is no team dashboard, no shared prompt library, no org-level routing policy.
- **Doesn't manage your API accounts.** You bring your own keys. ZedPlus stores them in your system keychain and uses them — it doesn't proxy requests through its own service or track your usage centrally.
- **No GUI.** Everything is text, terminal, and config files.

---

## Who it's for

- **Developers who already use multiple AI tools** and are tired of context-switching between Claude, Gemini, and a local model depending on the task
- **Cost-conscious teams or solo devs** who want cloud AI quality for hard problems and local model speed for easy ones, without manually deciding every time
- **Anyone building fine-tuned local models** who wants organic, high-quality training data generated from their own real work rather than synthetic datasets
- **Terminal-first engineers** who find chat UIs slow and prefer composable tools they can script around
- **Developers who want code-aware answers** without manually pasting context every time — the indexer handles that automatically

---

## Who it's not for

- **Non-developers.** ZedPlus assumes you are writing code, running a terminal, and have at least one AI API key. There is no onboarding for general users, yet.
- **People who want a GUI.** If you want a chat interface, use claude.ai or Gemini. ZedPlus is deliberately terminal-only.
- **Teams wanting centralized AI governance.** There are no admin controls, audit logs, or org-level policy enforcement. It's a personal developer tool.
- **Users who want a fully autonomous coding agent.** If your goal is "fix this bug without me touching it," you want Claude Code, Cursor Agent, or a similar autonomous system. ZedPlus augments your decisions; it doesn't replace them, yet.
- **People happy with a single provider.** If you only use Claude and that covers everything you need, ZedPlus adds complexity without much benefit. It earns its place when you're regularly choosing between providers.

---

## Install

### From source (requires Rust 1.78+)

```sh
git clone https://github.com/gautam-prakash/zedplus
cd zedplus
cargo install --path . --force
```

On Windows with MSYS2 / MinGW, ensure `C:\msys64\mingw64\bin` is on PATH before building.

### Pre-built binaries

Download from [Releases](https://github.com/gautam-prakash/zedplus/releases) and add to PATH.

Or use the self-updater once installed:

```sh
zedplus update --check   # see if a newer version is available
zedplus update           # download and install
```

---

## Quick start

```sh
# 1. Run the setup wizard (detects your CLIs, configures providers)
zedplus init

# 2. Authenticate with your API keys
zedplus auth

# 3. Start the REPL
zedplus
```

The wizard scans for installed CLIs (Claude, Gemini, Codex, Groq, Qwen, Aider), estimates routing costs, and writes `~/.config/zedplus/config.toml`.

---

## REPL commands

| Command | Description |
|---|---|
| `<query>` | Route and answer |
| `@claude`, `@gemini`, `@codex`, `@local`, `@cheap` | Force a provider for one query |
| `/explain <query>` | Show routing decision (model, reason, cost) |
| `/local <query>` | Force local model |
| `/cheap <query>` | Force cheapest model |
| `/model` | List model aliases |
| `/model <alias> <query>` | Override model for one query |
| `/build <description>` | Multi-phase build pipeline |
| `/debate [strategy] <query>` | Multi-agent brainstorm (debate/red-team/perspectives/delphi) |
| `/persona` | List developer personas |
| `/persona <name>` | Activate persona (architect/debugger/security/performance/teacher/reviewer/tester/devops) |
| `/agent` | Toggle agentic mode (file tools + command execution) |
| `/accept` | Toggle auto-accept for agent writes |
| `/apply` | Apply code blocks from last response to files |
| `/scope narrow\|broad` | Set context scope for next query |
| `/clear` | Reset session context |
| `/usage` | Show session token/cost totals |
| `/history` | Show last 20 turns |
| `/index` | Re-index current directory |
| `/ui [native\|claude\|gemini]` | Show or change UI style |
| `/exit` | End session with summary |

---

## One-shot commands

```sh
zedplus "explain how async Rust works"   # non-interactive query
zedplus --explain "refactor this code"   # show routing decision
zedplus --local "quick fix"              # force local model
zedplus --cheap "summarize this"         # force cheapest model
```

---

## Config

```sh
zedplus config --show              # pretty-print active config
zedplus config --edit              # open in $EDITOR
zedplus config --set routing.priority=cost
zedplus config --set routing.rules.code_review=gemini-pro
zedplus config --reset             # restore defaults
```

Config is stored at `~/.config/zedplus/config.toml`. Project-level overrides go in `.zedplus.toml` in your working directory.

---

## Adaptive routing

```sh
zedplus profile --optimize         # suggest routing changes based on your usage
zedplus profile --optimize --apply # write suggestions to .zedplus.toml
```

Triggers when you override the same task type 5+ times consistently.

---

## Distillation and training

```sh
zedplus distill                    # export JSONL for fine-tuning
zedplus distill --task code_review --since 2025-01-01
zedplus train --base mistral:7b --lora   # generate training script
zedplus train --status             # show job history
zedplus model import <path|id> --name my-model   # register custom model
```

---

## Session management

```sh
zedplus session list               # list saved sessions
zedplus session rename <id> "new name"
zedplus session archive <id>
zedplus resume                     # resume most recent session
```

---

## Usage tracking

```sh
zedplus usage --today
zedplus usage --month
```

---

## Default routing

| Task | Default model |
|---|---|
| Web search | Gemini Flash |
| Code review | Claude Sonnet |
| Complex reasoning | Claude Sonnet |
| Data analysis | Gemini Pro |
| Documentation | Claude Haiku |
| Quick completion | Local (Ollama) |

---

## License

MIT
