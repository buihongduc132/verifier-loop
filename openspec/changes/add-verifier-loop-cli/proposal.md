## Why

The current `verifier-loop` skill is enforced only via prompt instructions. An AI agent (A) being verified can trivially bypass it: skip the loop entirely, craft biased prompts to verifiers, narrow the scope to dodge wrong parts, or fabricate "approved" claims without proof. A is both defendant and judge. We need an out-of-process CLI that A cannot subvert, providing cryptographic proof (a hash) that independent verifiers actually reached consensus against an immutable goal.

## What Changes

- **New CLI: `verifier-loop`** (aliased `jewilo`) — A's only interface. Two subcommands:
  - `NEW "<goal>" [--context "..."]` — creates an immutable, signed goal; spawns N verifier sessions in parallel (non-blocking, ACP JSON stream); gathers all results; checks n/m consensus; outputs a completion hash or a rejection with notes.
  - `RESUME <goalId> [--fix "..."]` — loads prior state, increments round, spawns verifiers (reusing sessions up to `maxTurn`, then fresh), repeats until n/m or A gives up.
- **New CLI: `verifier-verdict`** (aliased `jewije`) — the verifier's only interface to register a verdict. `approve` or `reject --notes "..."`. Deterministic file writes via the JSON store. First verdict is FINAL; cannot be changed.
- **Immutable goal store** at `~/.verifier-loop/goals/<goalId>/` — goal.json (north star, signed once), rounds/<round>/<verifierId>/ verdicts with captured initial-prompt + final-output, completion.json (only on n/m match).
- **ACP adapter layer** — built-in adapters for `pi`, `hermes`, `acpx` (all emit the same ACP JSON event stream); custom adapters configurable via JSON. Resume via `--session <sid>` reuses the same SID.
- **Completion hash** — `vl:` + 40 hex chars, derived from salt + goalId + goalSignature + roundNumber + matchingVerdicts + matchedAtISO. Fool-proof: A cannot forge it without the secret salt; tampering any input invalidates the hash.
- **Fail-closed everywhere** — NULL verdict (verifier forgot/crashed/timed out) = no-pass, never silent-approve. Deleting the store = no proof = no-pass.
- **Configuration** at `~/.verifier-loop/config.json` — n, m, maxTurn, backend, model, gitDiffMaxChars, cwd, verifierTimeoutSec, and optional prompt/resume-prompt templates.

## Capabilities

### New Capabilities
- `goal-lifecycle`: Immutable goal creation (NEW), signed with salt+timestamp; RESUME with append-only fix notes; goal text can never change after creation.
- `verifier-spawn`: Parallel non-blocking spawn of N verifier sessions via ACP JSON stream adapters (pi/hermes/acpx/custom); session reuse up to maxTurn rounds then fresh spawn; gather all agent_end events.
- `verdict-registration`: Deterministic verdict registration via separate CLI (`verifier-verdict`); first verdict final; no pattern/regex matching on output; env-var identity injection.
- `consensus-check`: n/m consensus evaluation after each round; on match output completion hash, else output rejection + notes to A.
- `completion-proof`: Tamper-evident completion hash generation from salt + goal signature + matching verdicts; audit-traceable back to the goal directory.
- `verifier-prompt`: Blind prompt rendering with frozen artifacts (cwd, git status, file edit times, git diff top N chars); V* never sees round number, other verdicts, n/m config, or the hash.

### Modified Capabilities
<!-- None — this is a brand-new repo with no existing specs. -->

## Impact

- **New repo**: `../verifier-loop/` (sibling to pi-plugins). No code lives in pi-plugins; the existing skill may later become a thin wrapper (deferred).
- **New runtime dependency**: ACP-compatible CLI agents (`pi` with `--mode json` is the primary backend). Each verifier is a real CLI-agent session, not an in-process function.
- **Filesystem**: writes to `~/.verifier-loop/` (goal store, salt at mode 600, config). A runs as the same UID, so the salt gap (chattr +a hardening) is a known deferred limitation.
- **No existing code affected**: greenfield. The verifier-loop skill in pi-plugins stays untouched for now.
- **Binaries**: `verifier-loop` (jewilo) and `verifier-verdict` (jewije) installed on PATH.
