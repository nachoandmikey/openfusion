# Clawpatch notes for OpenFusion

Inspected `openclaw/clawpatch` for lessons because it also shells out to agent-native CLIs underneath.

## What Clawpatch does well

- **Provider abstraction:** providers normalize local CLIs and HTTP APIs behind one review/fix interface.
- **Strict output contracts:** Codex calls use JSON schemas and output validation; malformed output is treated as a provider error, not silently trusted.
- **Read-only vs write modes:** review/revalidate run read-only; fix uses explicit write permissions. This is the right safety split for agent CLIs.
- **Doctor command:** checks provider availability before real work.
- **Parallel jobs and rate limits:** `--jobs` and rate limiting are first-class because many provider calls run under the hood.
- **Trusted config boundary:** dangerous provider passthrough config is allowed only from explicitly trusted config, not repo-discovered config.
- **Install story:** npm/pnpm global package for JS users, plus source install. For OpenFusion, native Cargo install is cleaner now; npm can be a later binary wrapper.

## Lessons applied / to apply

Already applied:

- `doctor` command.
- explicit runner/provider abstraction.
- read-only Codex default flags.
- local skill install command.
- JSON output mode for agents.

Next to apply:

- strict structured schemas for `review` / `release-check` / `ci-gate`.
- per-strategy `jobs` / concurrency controls.
- rate limits per runner.
- trusted-config boundary for dangerous process runners.
- validation partitioning: keep valid findings even if one candidate emits malformed sections.
- exit codes for CI gates.

## Install comparison

Clawpatch is TypeScript and ships via npm/pnpm:

```bash
pnpm add -g clawpatch
npm install -g clawpatch
```

OpenFusion is Rust and should launch as native install first:

```bash
cargo install --git https://github.com/nachoiacovino/openfusion
```

Later, an npm package can download the native binary for agent ecosystems that expect `npm i -g`.
