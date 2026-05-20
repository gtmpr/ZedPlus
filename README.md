# ZedPlus

Smart AI routing CLI with realtime code indexing, multi-provider backends, and local model distillation.

## What it does

ZedPlus routes each query to the best AI model for the task — Claude for complex reasoning and code review, Gemini for web-grounded answers, fast local models (Ollama / LM Studio) for quick completions — and learns from your overrides over time.

- **Smart router** — classifies queries into 7 task types and picks the right model
- **Multi-backend** — Claude, Gemini, OpenAI, Codex, Ollama, LM Studio in one CLI
- **Code indexer** — tree-sitter parsing with semantic search and git diff context
- **Adaptive routing** — detects your override patterns and suggests routing changes
- **Distillation** — logs every Q&A to JSONL for fine-tuning local models
- **Session persistence** — auto-resumes previous sessions with named history
- **Multi-agent brainstorm** — debate, red-team, perspectives, and Delphi strategies
- **Architect/Editor mode** — high-quality model plans, fast model implements

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

## One-shot commands

```sh
zedplus "explain how async Rust works"   # non-interactive query
zedplus --explain "refactor this code"   # show routing decision
zedplus --local "quick fix"              # force local model
zedplus --cheap "summarize this"         # force cheapest model
```

## Config

```sh
zedplus config --show              # pretty-print active config
zedplus config --edit              # open in $EDITOR
zedplus config --set routing.priority=cost
zedplus config --set routing.rules.code_review=gemini-pro
zedplus config --reset             # restore defaults
```

Config is stored at `~/.config/zedplus/config.toml`. Project-level overrides go in `.zedplus.toml` in your working directory.

## Adaptive routing

```sh
zedplus profile --optimize         # suggest routing changes based on your usage
zedplus profile --optimize --apply # write suggestions to .zedplus.toml
```

Triggers when you override the same task type 5+ times consistently.

## Distillation and training

```sh
zedplus distill                    # export JSONL for fine-tuning
zedplus distill --task code_review --since 2025-01-01
zedplus train --base mistral:7b --lora   # generate training script
zedplus train --status             # show job history
zedplus model import <path|id> --name my-model   # register custom model
```

## Session management

```sh
zedplus session list               # list saved sessions
zedplus session rename <id> "new name"
zedplus session archive <id>
zedplus resume                     # resume most recent session
```

## Usage tracking

```sh
zedplus usage --today
zedplus usage --month
```

## Models

ZedPlus ships a model registry at `assets/models.toml` with quality/speed tiers and task strengths. See `zedplus config --show` for the active routing rules, or `/model` in the REPL for available aliases.

Default routing:

| Task | Default model |
|---|---|
| Web search | Gemini Flash |
| Code review | Claude Sonnet |
| Complex reasoning | Claude Sonnet |
| Data analysis | Gemini Pro |
| Documentation | Claude Haiku |
| Quick completion | Local (Ollama) |

## License

MIT
