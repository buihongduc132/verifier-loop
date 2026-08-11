# jewilo — verifier-loop

**Out-of-process verification gate for AI agents.**

Subagents claim "done". jewilo proves it.

## The Problem

When an AI agent (A) self-reports work as complete, it is both **defendant and judge**. It can:

- Skip verification entirely
- Bias prompts to friendly verifiers
- Fabricate "approved" claims without evidence
- Gaslight via omission — hide rejection feedback

There is no cryptographic proof that independent verifiers actually reached consensus.

## The Solution

jewilo is an out-of-process CLI that spawns independent verifier sessions, gathers their verdicts, and produces a **tamper-evident completion hash** only on genuine consensus.

```
Agent A does work
    ↓
jewilo spawns m independent verifiers (V*)
    ↓
n/m consensus? NO → REJECT + notes back to A → A fixes
    ↓
RESUME (round 2) → repeat until n/m APPROVE
    ↓
Consensus reached → tamper-evident hash: mmddyy-XXXXXXXX
```

The hash is derived from salt + goal + verdicts + timestamp. Any edit to the goal or verdicts invalidates it. A cannot forge it without the secret salt.

## Two Binaries

| Binary | Alias | Role | Interface |
|--------|-------|------|-----------|
| `verifier-loop` | `jewilo` | Agent A | `NEW`, `RESUME`, spawn, gather, consensus, hash |
| `verifier-verdict` | `jewije` | Verifier V* | `approve`, `reject --notes "…"` |

Strict capability separation: A cannot call `jewije` directly. V* identity comes from environment variables injected at spawn, not arguments.

## Quick Start

```bash
# Install
cargo install --path .
# or
./scripts/install.sh  # installs to ~/.local/bin with jewilo/jewije aliases

# Verify install
command -v jewilo jewije

# Agent A: start a fresh goal
jewilo NEW "implement the foo-bar endpoint with tests"
# → prints goalId, then on consensus: mmddyy-XXXXXXXX

# Agent A: next round with fix notes
jewilo RESUME <goalId> --fix "addressed the missing error path"

# Verifier V*: register verdict (identity from env, not args)
jewije approve
jewije reject --notes "issue 1: missing test for the error path"
```

## Configuration

`~/.verifier-loop/config.json` — all fields optional, sensible defaults:

```json
{
  "n": 2,
  "m": 3,
  "maxTurn": 200,
  "backend": "hermes-verifier",
  "gitDiffMaxChars": 8000,
  "verifierTimeoutSec": 1800,
  "minGoalChars": 500
}
```

| Key | Default | Meaning |
|-----|---------|---------|
| `n` | 2 | Consensus threshold (n of m must APPROVE) |
| `m` | 2 | Number of verifiers per round |
| `maxTurn` | 3 | Per-verifier turn budget before fresh spawn |
| `backend` | `"pi"` | ACP backend: `pi` \| `hermes` \| `acpx` \| custom |
| `gitDiffMaxChars` | 10000 | Cap on frozen git diff snapshot |
| `verifierTimeoutSec` | 1800 | Per-verifier wall-clock timeout |
| `minGoalChars` | 0 | Minimum goal text length (0 disables) |

**Dynamic pipeline (2+1/3):** set `dumpAdapter` + `smartAdapter` instead of `backend`. Gate = m dump slots, Confirm = `confirmCount` smart slots. Output format shows which phases passed: `2+1/3` = Gate(2 dump) + Confirm(1 smart) / m=3.

## Fail-Closed Guarantees

- **NULL verdict** (crash/timeout/forgot) → never becomes APPROVE
- **Missing store** → no hash
- **Goal text edit** → signature mismatch → hash mismatch
- **Verdict edit** → hash mismatch on recompute

## JSON Output Mode

Both binaries accept `--json` for machine-readable output:

```bash
jewilo --json NEW "implement the foo-bar endpoint"
# stdout: {"ok":true,"command":"new","goalId":"...","round":1,"status":"consensus-passed","hash":"071526-00a50e40","fullDigest":"..."}
```

Single JSON object on stdout. Human-readable diagnostics on stderr. Exit codes unchanged.

## Observability

- **Per-goal traceId**: minted at NEW, propagated to every V* child
- **trace.jsonl**: newline-delimited lifecycle events per goal
- **Opt-in OTLP/gRPC**: build with `--features otel` + set `VERIFIER_LOOP_OTEL_EXPORTER_OTLP_ENDPOINT`

Tracing is fail-open: errors never block verdicts or hashes. `traceId` is metadata, NOT a hash input.

## Documentation

- **[USAGE.md](USAGE.md)** — full invocation reference
- **[AGENTS.md](AGENTS.md)** — agent-facing source of truth
- **[THREAT-MODEL.md](THREAT-MODEL.md)** — security model, attacker matrix, out-of-process V* requirement
- **Design source**: `openspec/changes/`, `flow/explore/`, `flow/design/`

## Build

```bash
cargo build --release
# binaries land in target/release/{verifier-loop,verifier-verdict}
```

## License

MIT OR Apache-2.0
