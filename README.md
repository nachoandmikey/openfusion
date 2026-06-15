# OpenFusion

**Don't pick a model. Pick a panel.**

OpenFusion is a local-first CLI for running the same prompt through multiple LLM backends and getting one synthesized answer. Use it for model consensus, best-of-N answers, code review, CI gates, and local OpenAI-compatible routing.

OpenFusion works with API providers, local models, and installed agent CLIs:

- OpenRouter
- OpenAI-compatible APIs: OpenAI, LiteLLM, vLLM, LM Studio, LocalAI, etc.
- Anthropic / Claude API
- Ollama
- Codex CLI
- Claude Code CLI
- Gemini CLI
- Any custom non-interactive command

## Install

Requires Rust/Cargo. If you do not have Cargo yet, install it with [rustup](https://rustup.rs/).

```bash
cargo install --git https://github.com/nachoiacovino/openfusion
openfusion --help
```

If your shell cannot find `openfusion` after install, add Cargo's bin directory to PATH:

```bash
export PATH="$HOME/.cargo/bin:$PATH"
```

## Update

After a new commit lands on GitHub, reinstall from the same Git URL with `--force`:

```bash
cargo install --git https://github.com/nachoiacovino/openfusion --force
```

If you installed from a local checkout instead, pull and reinstall:

```bash
git pull
cargo install --path . --force
```

From a local checkout:

```bash
git clone https://github.com/nachoiacovino/openfusion.git
cd openfusion
cargo install --path .
```

For coding agents that use [skills.sh](https://skills.sh/), preview the companion skill install command, then opt in explicitly:

```bash
openfusion skills install          # dry-run; prints the npx command only
openfusion skills install --yes    # actually writes agent skill files
```

## Quick start

Create a starter config and verify the setup:

```bash
openfusion init
openfusion list
openfusion doctor
```

`openfusion init` prefers logged-in local CLIs: Codex, Claude Code, and Gemini. It makes a tiny live probe before enabling each one, so a new user with Codex or Claude already installed and logged in does not need an API key to start. To generate config without live model calls, use `openfusion init --no-probe`.

Then run a prompt:

```bash
openfusion ask "What is the biggest risk in this launch plan?"
```

Long-running calls print queue/progress events to stderr before the final stdout answer, for example:

```txt
openfusion: strategy consensus: queued 3 candidate call(s)
openfusion: calling claude (opus-4.8)...
openfusion: calling codex (gpt-5.5)...
openfusion: candidate claude finished ok in 42137ms
openfusion: judging with claude (opus-4.8)...
```

Progress is intentionally on stderr so stdout remains parseable when JSON is selected automatically or explicitly. Use `--quiet` to suppress it.

Show each model's answer before the final synthesis:

```bash
openfusion ask --show-candidates "Should this PR merge?"
```

Run the fastest successful backend:

```bash
openfusion ask --strategy race "Give me a one-sentence answer."
```

## Common commands

```bash
openfusion init                         # create global config
openfusion init --local                 # create ./openfusion.toml for this project
openfusion list                         # show runners and strategies
openfusion doctor                       # live-check keys, local servers, and CLI model calls
openfusion keys set openrouter          # store key in OS keychain
openfusion keys list                    # list stored/discovered key refs
openfusion keys list -c ./other.toml     # include refs from an explicit config
openfusion keys check openrouter        # verify key exists without printing it
openfusion ask "Question..."            # markdown on a terminal; JSON when piped
openfusion ask --format json "Q"        # force machine-readable trace
openfusion ask --format markdown "Q"    # force human-readable answer
openfusion ask --quiet "Question"       # suppress progress/status lines
openfusion review src/main.rs           # code review panel
openfusion ci-gate src/main.rs          # JSON verdict + non-zero exit on fail
openfusion serve --port 8787            # local OpenAI-compatible API
```

## Configuration

`openfusion init` writes a global user config. On macOS this is usually:

```txt
~/Library/Application Support/openfusion/config.toml
```

On Linux it is usually:

```txt
~/.config/openfusion/config.toml
```

For a project-specific override, run:

```bash
openfusion init --local
```

Config resolution order:

1. `--config path/to/config.toml`
2. `./openfusion.toml`, if present
3. global user config

Secrets are not stored in TOML. Config stores key references like `api_key_ref = "openrouter"`; the key itself lives in the OS keychain.

## Add API keys

```bash
openfusion keys set openrouter      # paste key securely
openfusion keys set openai
openfusion keys set anthropic
openfusion keys list -c ./openfusion.toml
openfusion keys check openrouter
openfusion keys remove openrouter
```

Import from an existing environment variable:

```bash
openfusion keys set openrouter --value-env OPENROUTER_API_KEY
```

Example OpenRouter runner:

```toml
[[runners]]
name = "qwen-cheap"
kind = "openrouter"
model = "qwen/qwen3.5-flash-02-23"
api_key_ref = "openrouter"
api_key_env = "OPENROUTER_API_KEY" # fallback if keychain is missing
weight = 0.8
max_tokens = 900
```

## Backends

### OpenAI-compatible APIs

Use this for OpenAI, LiteLLM, vLLM, LM Studio, LocalAI, and compatible proxies.

```toml
[[runners]]
name = "openai-small"
kind = "openai_compat"
base_url = "https://api.openai.com/v1"
model = "gpt-4o-mini"
api_key_ref = "openai"
api_key_env = "OPENAI_API_KEY"
weight = 0.8
```

### Anthropic / Claude API

```toml
[[runners]]
name = "claude-api"
kind = "anthropic"
model = "claude-sonnet-4-5"
api_key_ref = "anthropic"
api_key_env = "ANTHROPIC_API_KEY"
max_tokens = 1200
weight = 1.0
```

### Ollama

```bash
ollama serve
ollama pull llama3.2
```

```toml
[[runners]]
name = "ollama"
kind = "ollama"
base_url = "http://localhost:11434"
model = "llama3.2"
weight = 0.3
```

### LM Studio

Start the LM Studio local server, then use an OpenAI-compatible runner:

```toml
[[runners]]
name = "lmstudio"
kind = "openai_compat"
base_url = "http://127.0.0.1:1234/v1"
api_key_env = "LOCAL_LLM_API_KEY"
model = "your-loaded-model"
weight = 0.4
```

For local servers that do not require a real key:

```bash
export LOCAL_LLM_API_KEY=local
```

### Codex CLI

Requires `codex` installed and logged in.

```toml
[[runners]]
name = "codex"
kind = "codex"
model = "gpt-5.5"
weight = 1.2
```

### Claude Code CLI

Requires `claude` installed and logged in.

```toml
[[runners]]
name = "claude"
kind = "claude"
model = "opus-4.8"
weight = 1.1
```

### Gemini CLI

Requires `gemini` installed and logged in or configured.

```toml
[[runners]]
name = "gemini"
kind = "gemini"
model = "gemini-3-pro"
weight = 1.0
```

### Any CLI

Wrap any command that can take a prompt non-interactively:

```toml
[[runners]]
name = "custom-cli"
kind = "process"
command = "your-ai-cli"
args = ["--non-interactive", "{{prompt}}"]
extract = "text"
weight = 0.7
```

## Strategies

Strategies decide which runners are called and how the final answer is produced.

```toml
[[strategies]]
name = "consensus"
type = "consensus"
runners = ["qwen-cheap", "claude", "codex"]
judge = "qwen-cheap"
```

Available strategy types:

- `consensus` — ask multiple runners, then ask a judge to synthesize.
- `best-of-n` — run multiple attempts, then choose/synthesize the best answer.
- `review` — code-review prompt for bugs, false positives, and recommendations.
- `fallback` — try runners in order until one succeeds.
- `race` — run candidates in parallel and return the first successful answer.

Run the same runner multiple times with `runner*N`:

```toml
[[strategies]]
name = "best-of-qwen"
type = "best_of_n"
runners = ["qwen-cheap*5"]
judge = "qwen-cheap"
```

## Reasoning controls

Each runner can have its own reasoning profile.

```toml
[[runners]]
name = "fast-scout"
kind = "openrouter"
model = "qwen/qwen3.5-flash-02-23"
reasoning_effort = "low"        # minimal | low | medium | high
reasoning_budget_tokens = 512
reasoning_include = false
max_tokens = 700

[[runners]]
name = "deep-judge"
kind = "openai_compat"
base_url = "https://api.openai.com/v1"
api_key_ref = "openai"
api_key_env = "OPENAI_API_KEY"
model = "gpt-5.1"
reasoning_effort = "high"
reasoning_budget_tokens = 4096
reasoning_include = true
max_tokens = 2000
```

Provider-specific request fields can be passed through with TOML inline tables:

```toml
reasoning_extra = { summary = "auto" }
extra_body = { top_p = 0.9, seed = 42 }
```

## Code review and CI gates

Review one or more files:

```bash
openfusion review src/main.rs templates/openfusion.toml
```

Use `ci-gate` when you want a strict pass/fail result for automation:

```bash
openfusion ci-gate src/main.rs
```

`ci-gate` prints JSON and exits non-zero on failure.

OpenFusion is agent-friendly without making direct human runs ugly: `ask` and `review` default to Markdown when stdout is a terminal, and JSON when stdout is piped/non-TTY. Progress messages always go to stderr. Use `--format json` or `--format markdown` to override detection, and `--quiet` to silence stderr for automation.

## OpenAI-compatible API mode

Start the local server:

```bash
openfusion serve --port 8787
```

Point any OpenAI-compatible client at it:

```bash
export OPENAI_BASE_URL=http://127.0.0.1:8787/v1
export OPENAI_API_KEY=local
```

Strategies appear as models:

```txt
openfusion/consensus
openfusion/best-of-n
openfusion/review
openfusion/fallback
```

Example request:

```bash
curl http://127.0.0.1:8787/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{
    "model": "openfusion/consensus",
    "messages": [{"role":"user","content":"Give one launch risk."}]
  }'
```

Streaming/SSE mode is supported:

```bash
curl http://127.0.0.1:8787/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{"model":"openfusion/consensus","stream":true,"messages":[{"role":"user","content":"Give one launch risk."}]}'
```

Note: for CLI-backed runners such as Codex, Claude Code, Gemini, or `process`, OpenFusion receives the complete runner response and then splits it into small SSE deltas. This is OpenAI-client-compatible incremental framing, not true upstream token-by-token latency.

## Agent-friendly discovery

Commands agents should know:

```bash
openfusion init                  # write starter config
openfusion list                  # show runners/strategies
openfusion doctor                # check installed CLIs/env keys/local servers
openfusion ask                   # markdown on TTY; JSON when piped/non-TTY
openfusion ask --format json      # force machine-readable trace
openfusion ask --format markdown  # force human-readable answer
openfusion review FILES...        # same TTY-aware output behavior
openfusion ci-gate FILES...      # strict JSON + non-zero exit on fail
openfusion serve                 # local OpenAI-compatible API
```

## Agent skill

OpenFusion includes a companion skill for agents that use [skills.sh](https://skills.sh/):

```bash
openfusion skills install                                # preview only
openfusion skills install --global --yes                 # install globally/user-level
openfusion skills install -g -y --agent claude-code      # limit to one agent
openfusion skills install -g -y --agent claude-code --skill openfusion-consensus
```

Equivalent commands:

```bash
npx skills add nachoiacovino/openfusion --global --yes
npx skills add nachoiacovino/openfusion --global --yes --agent claude-code --skill openfusion-consensus
```

## Safety

OpenFusion does not write API keys into config files. API keys set with `openfusion keys set` are stored in the OS keychain, with environment variables as an explicit fallback.

Agent CLIs can run tools or commands depending on their own configuration. OpenFusion's built-in Codex adapter uses read-only sandboxing and ephemeral sessions. For arbitrary `process` runners, choose safe flags and commands for your environment.
