# OpenFusion

**Don't pick a model. Pick a panel.**

OpenFusion is a small Rust CLI for **best-of-N, consensus, and code-review panels** across multiple LLM backends. It calls independent runners, captures their candidate answers, then asks a judge runner to select/synthesize a final verdict with disagreements preserved.

It is useful today as a CLI and can also run as a local OpenAI-compatible API for agent tools.

## What you can do

```bash
# Ask a consensus panel
openfusion ask --strategy consensus "What is the biggest risk in this launch plan?"

# Run best-of-N: same model multiple times + other candidates + judge
openfusion ask --strategy best-of-n "Find the simplest design for this CLI."

# Review files with a panel
openfusion review src/auth.rs src/session.rs

# CI/release gate: JSON verdict and exit code
openfusion ci-gate src/auth.rs

# Show every candidate answer and latency
openfusion ask --show-candidates --strategy consensus "Should this PR merge?"

# JSON trace for scripts/CI/agents
openfusion ask --format json --strategy consensus "Return a release risk assessment."

# Serve as an OpenAI-compatible local endpoint
openfusion serve --port 8787
```

## Requirements

- Rust/Cargo for the current install path. Install with [rustup](https://rustup.rs/) if needed.
- At least one backend:
  - OpenRouter API key, or
  - OpenAI/Anthropic API key, or
  - logged-in Codex/Claude/Gemini CLI, or
  - local Ollama/LM Studio server.

## Install

Current recommended install is Cargo from GitHub:

```bash
cargo install --git https://github.com/nachoiacovino/openfusion
openfusion --help
```

Prebuilt GitHub Release binaries and Homebrew are planned next; see [`docs/DISTRIBUTION.md`](docs/DISTRIBUTION.md).

From a local checkout:

```bash
git clone https://github.com/nachoiacovino/openfusion.git
cd openfusion
cargo install --path .
```

Install the companion agent skill through skills.sh:

```bash
openfusion skills install
# equivalent to: npx skills add nachoiacovino/openfusion
```

## 60-second setup

```bash
openfusion init                 # writes the global user config
openfusion keys set openrouter  # paste key; stored in OS keychain
openfusion list
openfusion doctor
```

After `doctor` passes, try:

```bash
openfusion ask --strategy race "Reply with one sentence about why consensus helps."
openfusion ask --show-candidates --strategy consensus "Review this launch idea."
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

Resolution order is:

1. `--config path/to/config.toml`
2. `./openfusion.toml` local project override, when present
3. global user config

## Add API keys

OpenFusion stores API keys in the OS keychain by default. Secrets are not written into TOML.

```bash
openfusion keys set openrouter      # paste key securely
openfusion keys set openai
openfusion keys check openrouter    # verifies without printing the key
openfusion keys remove openrouter
```

Non-interactive import from an existing env var:

```bash
openfusion keys set openrouter --value-env OPENROUTER_API_KEY
```

Config stores a key reference, not the secret:

```toml
[[runners]]
name = "qwen-cheap"
kind = "openrouter"
model = "qwen/qwen3.5-flash-02-23"
api_key_ref = "openrouter"          # OS keychain service=openfusion, account=openrouter
api_key_env = "OPENROUTER_API_KEY"  # fallback if keychain is missing
weight = 0.8
max_tokens = 900
```

### OpenAI-compatible APIs

Works with OpenAI, LiteLLM, vLLM, LM Studio, LocalAI, OpenRouter-compatible proxies, etc.

```toml
[[runners]]
name = "openai-small"
kind = "openai_compat"
base_url = "https://api.openai.com/v1"
api_key_ref = "openai"
api_key_env = "OPENAI_API_KEY"
model = "gpt-4o-mini"
weight = 0.8
```

For a local server with no real key, set a dummy env var or key ref:

```bash
export LOCAL_LLM_API_KEY=local
```

```toml
[[runners]]
name = "lmstudio"
kind = "openai_compat"
base_url = "http://127.0.0.1:1234/v1"
api_key_env = "LOCAL_LLM_API_KEY"
model = "local-model"
weight = 0.4
```

## Add local models

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

Start the LM Studio local server, then use the OpenAI-compatible runner:

```toml
[[runners]]
name = "lmstudio"
kind = "openai_compat"
base_url = "http://127.0.0.1:1234/v1"
api_key_env = "LOCAL_LLM_API_KEY"
model = "your-loaded-model"
weight = 0.4
```

## Add installed agent CLIs

### Codex

Requires `codex` installed and logged in. This is the default/preferred path for Codex: use the official CLI OAuth/session, not a copied token. OpenFusion uses `codex exec` with read-only sandboxing and ephemeral sessions.

```toml
[[runners]]
name = "codex"
kind = "codex"
# omit model to use the default supported by your logged-in account
weight = 1.2
```

### Claude Code

Requires `claude` installed and logged in. This is the default/preferred path: use the official Claude Code OAuth/subscription session, not a copied token.

```toml
[[runners]]
name = "claude"
kind = "claude"
model = "sonnet"
weight = 1.1
```

### API-key override for Codex/Claude

`kind = "codex"` and `kind = "claude"` use the official CLI sessions by default.
To force API keys instead, define API runners and point your strategy at them:

```toml
[[runners]]
name = "codex-api"
kind = "openai_compat"
base_url = "https://api.openai.com/v1"
model = "gpt-5.1-codex"
api_key_ref = "openai"
api_key_env = "OPENAI_API_KEY"

[[runners]]
name = "claude-api"
kind = "anthropic"
model = "claude-sonnet-4-5"
api_key_ref = "anthropic"
api_key_env = "ANTHROPIC_API_KEY"

[[strategies]]
name = "api-only"
type = "race"
runners = ["codex-api", "claude-api"]
```

Then store keys in the OS keychain, not TOML:

```bash
openfusion keys set openai --value-env OPENAI_API_KEY
openfusion keys set anthropic --value-env ANTHROPIC_API_KEY
```

### Gemini CLI

Requires `gemini` installed and logged in or configured. Prefer the official Gemini CLI OAuth/session where available.

```toml
[[runners]]
name = "gemini"
kind = "gemini"
model = "gemini-2.5-pro"
weight = 1.0
```

### Any CLI

Wrap any non-interactive command:

```toml
[[runners]]
name = "custom-cli"
kind = "process"
command = "your-ai-cli"
args = ["--non-interactive", "{{prompt}}"]
extract = "text"
weight = 0.7
```


## Reasoning controls

OpenFusion lets each runner carry its own reasoning profile, so cheap scouts can run light while judges run deep.

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
api_key_env = "OPENAI_API_KEY"
model = "gpt-5.1"
reasoning_effort = "high"
reasoning_budget_tokens = 4096
reasoning_include = true
max_tokens = 2000
```

Portable fields:

- `reasoning_effort`: `minimal`, `low`, `medium`, or `high`
- `reasoning_budget_tokens`: thinking/reasoning token budget hint
- `reasoning_include`: ask providers to include reasoning traces when supported
- `reasoning_extra`: provider-specific JSON merged into the reasoning object last
- `extra_body`: provider-specific JSON merged into the request body last

Example power-user overrides:

```toml
reasoning_extra = { summary = "auto" }
extra_body = { top_p = 0.9, seed = 42, parallel_tool_calls = false }
```

Mapping:

- `openai_compat` and `openrouter` send `reasoning` plus a `thinking` alias for providers that use that name.
- `ollama` maps temperature/token settings into `options` and still includes generic `reasoning`/`thinking` for compatible local servers/proxies.
- `process`, `codex`, `claude`, and `gemini` receive environment hints such as `OPENFUSION_REASONING_EFFORT` and `OPENFUSION_REASONING_BUDGET_TOKENS`; support depends on the wrapped CLI.

## Daily use cheat sheet

```bash
openfusion list                              # see configured runners/strategies
openfusion doctor                            # verify keys, CLIs, and local servers
openfusion ask "Question..."                 # run default consensus strategy
openfusion ask --strategy race "Question..." # fastest successful candidate wins
openfusion ask --show-candidates "Question"  # show raw panel answers
openfusion review src/main.rs                # panel code review
openfusion ci-gate src/main.rs               # JSON verdict + non-zero exit on fail
openfusion serve --port 8787                 # local OpenAI-compatible API
```

## Customize panels

Strategies decide which runners are called and who judges.

```toml
[[strategies]]
name = "consensus"
type = "consensus"
runners = ["qwen-cheap", "llama-cheap", "codex"]
judge = "qwen-cheap"
```

### Run N instances of the same model

Use `runner*N` in strategy runner lists:

```toml
[[strategies]]
name = "best-of-qwen"
type = "best_of_n"
runners = ["qwen-cheap*5"]
judge = "qwen-cheap"
```

This asks the same runner five independent times, then judges the candidates.

### Control concurrency and rate spacing

Global config:

```toml
max_jobs = 4

[[runners]]
name = "slow-or-rate-limited"
min_interval_ms = 500
```

Per run:

```bash
openfusion ask --jobs 2 --strategy consensus "Review this migration"
```

### Weight models

Weights are soft priors shown to the judge:

```toml
weight = 1.2  # stronger/more trusted
weight = 0.3  # cheap scout/local model
```

The judge can still select a lower-weight answer if it finds a real issue.

## Strategies

- `consensus` — ask multiple runners, judge agreements/disagreements, synthesize.
- `best-of-n` — multiple attempts, judge picks/synthesizes best.
- `review` — code-review-chair prompt for bugs, false positives, final recommendations.
- `fallback` — try runners in order until one succeeds.
- `race` — launches candidates in parallel and returns the first successful arrival; remaining in-process futures are dropped immediately.

OpenFusion is agent-friendly without making direct human runs ugly: `ask` and `review` default to Markdown when stdout is a terminal, and JSON when stdout is piped/non-TTY. Progress messages always go to stderr. Use `--format json` or `--format markdown` to override detection, and `--quiet` to silence stderr for automation.

## OpenAI-compatible API mode

Start server:

```bash
openfusion serve --port 8787
```

Use from any OpenAI-compatible client:

```bash
export OPENAI_BASE_URL=http://127.0.0.1:8787/v1
export OPENAI_API_KEY=local
```

Models are strategies:

```txt
openfusion/consensus
openfusion/best-of-n
openfusion/review
openfusion/fallback
```

Example:

```bash
curl http://127.0.0.1:8787/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{
    "model": "openfusion/consensus",
    "messages": [{"role":"user","content":"Give one launch risk."}]
  }'

# Streaming/SSE mode also works:
curl http://127.0.0.1:8787/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{"model":"openfusion/consensus","stream":true,"messages":[{"role":"user","content":"Give one launch risk."}]}'
```

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

## More agent use cases

OpenFusion is designed for agents calling a CLI. Useful tools/commands to add next:

- `openfusion plan` — ask a model panel for implementation plans, then return the lowest-risk plan.
- `openfusion test-plan` — generate edge-case test matrices before coding.
- `openfusion debug` — collect independent root-cause hypotheses, then rank by reproducibility.
- `openfusion security` — specialized review prompt for auth, injection, secrets, file-system writes, and supply-chain risk.
- `openfusion release-check` — consensus launch checklist for docs, tests, CI, install, and migration risk.
- `openfusion prompt-review` — have multiple models critique an agent prompt or skill for ambiguity and injection risk.
- `openfusion ci-gate` — implemented: machine-readable verdict + exit code for PR automation.

These all reuse the same runner → strategy → judge engine.

## Safety notes

Agent-native CLIs can edit files or run commands. OpenFusion's built-in Codex adapter uses read-only sandboxing and ephemeral sessions. Claude/Gemini adapters use plan/no-session style flags where available. For arbitrary process runners, you are responsible for safe flags and isolated workdirs.

OpenFusion does not write API keys into config files. API-provider secrets live in the OS keychain when set with `openfusion keys set`, with environment variables as an explicit fallback. OAuth-capable CLIs such as Codex, Claude, and Gemini should use their own official logged-in sessions by default.

## Roadmap

Done since v0.1: SSE-compatible `serve` streaming, first-success `race`, `--jobs`, per-runner `min_interval_ms`, `ci-gate` JSON schema/exit codes, and tag-driven GitHub Release binary workflow.

Still next:

- Cost estimates and budget-aware early stopping.
- Native OpenCode/Cline adapters.
- Homebrew/npm/cargo-binstall distribution. See [`docs/DISTRIBUTION.md`](docs/DISTRIBUTION.md).

## Positioning

There are existing consensus/debate tools. OpenFusion's intended niche is **small Rust single-binary, CLI-first, backend-agnostic, transparent traces, easy use in shell/CI, and local OpenAI-compatible gateway mode.**
