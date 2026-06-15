# OpenFusion architecture

OpenFusion has three layers:

1. **Runners** call one backend and normalize output to `{runner, model, weight, text, error, latency}`.
2. **Strategies** decide which runners to call and how to combine them.
3. **Judges** are just runners receiving a structured judge prompt.

## Runner classes

- `openrouter`: convenience wrapper for OpenRouter chat completions.
- `openai_compat`: any `/v1/chat/completions` endpoint.
- `ollama`: native `/api/chat` endpoint.
- `codex`: installed Codex CLI with read-only sandbox and ephemeral session.
- `process`: arbitrary installed CLI with `{{prompt}}` substitution.

## Strategies

- `best_of_n`: candidates answer independently; judge chooses/synthesizes the strongest.
- `consensus`: judge identifies agreement, disagreement, and final answer.
- `review`: judge acts as a senior code-review chair.
- `fallback`: runners are tried in order until one succeeds.
- `race`: parallel call path; current v0 returns the first successful candidate in completion collection order. True cancellation/first-arrival semantics are planned.

## Weights

Weights are included in the judge prompt as a prior. They are intentionally not a hard vote because a weaker/cheaper model can still catch a real issue.

## Why CLI-first

The core value is the consensus primitive, not HTTP plumbing. The CLI proves:

- adapter normalization,
- useful judge prompts,
- transparent traces,
- shell/CI ergonomics.

The planned `serve` mode will wrap the same engine as an OpenAI-compatible model for opencode/aider/Cline/Continue.
