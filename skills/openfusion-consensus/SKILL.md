---
name: openfusion-consensus
description: Use OpenFusion CLI to ask multiple LLM runners, preserve dissent, and return a consensus/best-of-N verdict for coding-agent tasks.
version: 0.1.0
author: OpenFusion contributors
license: MIT
metadata:
  tags: [llm, consensus, best-of-n, code-review, agents, cli]
---

# OpenFusion Consensus

Use this skill when a task would benefit from multiple independent model opinions: architecture choices, code review, release risk, debugging hypotheses, security review, test planning, migration plans, or PR readiness.

OpenFusion is intended primarily for **agents calling a CLI**, not humans chatting directly with it. Treat the CLI output as structured evidence from a model panel.

## Prerequisites

From a repo with `openfusion.toml`:

```bash
openfusion doctor
openfusion list
```

If missing config:

```bash
openfusion init
```

API keys live in environment variables, not config files:

```bash
export OPENROUTER_API_KEY=...
```

## Default workflows

### Consensus decision

Use for design choices, launch risk, debugging hypotheses, and ambiguous tradeoffs.

```bash
openfusion ask --strategy consensus --format json "QUESTION"
```

Read:

- `final` for the synthesized answer.
- `candidates[]` for each runner's raw opinion, status, latency, and weight.

### Best-of-N

Use when you want multiple independent attempts, including repeated samples from one model.

```bash
openfusion ask --strategy best-of-n --format json "TASK"
```

Configure repetitions in `openfusion.toml`:

```toml
runners = ["or-flash*3", "codex", "ollama"]
```

### Code review panel

Use before shipping code, especially when false positives and dissent matter.

```bash
openfusion review --format json src/foo.rs src/bar.rs
```

### OpenAI-compatible local model

If an agent harness needs a model endpoint, start:

```bash
openfusion serve --port 8787
```

Then configure the harness:

```bash
OPENAI_BASE_URL=http://127.0.0.1:8787/v1
OPENAI_API_KEY=local
```

Use model ids like `openfusion/consensus` or `openfusion/review`.

## Agent output pattern

When summarizing OpenFusion output to a user, include:

1. Consensus verdict.
2. Material disagreements.
3. The recommended next action.
4. Any failed runners or timeouts.

Do not hide dissent. The point of OpenFusion is not to average answers into bland agreement; it is to make tradeoffs visible.

## Safety

- For arbitrary CLI runners, verify they are configured read-only unless the user explicitly wants edits.
- Do not put API keys in `openfusion.toml`; use env vars.
- Treat local CLI adapters as potentially stateful. Prefer ephemeral/read-only modes when available.
- If all candidates fail, surface the failure instead of inventing a verdict.
