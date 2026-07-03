## Context

The `verifier-loop` skill in pi-plugins guides an AI agent (A) through spawning independent verifiers (V*) to review completed work before sign-off. Today this is enforced only by prompt text: A reads the skill, decides whether to follow it, and controls how verifiers are prompted. That trust model is broken — A is incentivised to skip verification, narrow the verifier's scope, or simply claim "approved" without proof.

A parallel explore session (see `pi-plugins/flow/findings/verifier-loop-cli/`, locked decisions LD1–LD27) established the full design. This change implements the agreed architecture: two out-of-process CLIs that A cannot subvert, producing a cryptographic proof hash on consensus.

Key constraints established by the user during explore:
- A is adversarial; V* is honest but fallible (≈40% per-verifier error rate). n/m consensus compensates (3/3 → wrong prob 0.4³ = 6.4%).
- Goal text is immutable after creation (prevents goalpost-moving).
- Verifiers are blind — they never see round number, other verdicts, n/m config, or the hash.
- Verdict registration MUST be an explicit deterministic CLI call. NO pattern/regex matching on V* output.
- All verifiers run in parallel, non-blocking; the loop gathers all results at the end.
- Everything fails closed: a NULL verdict (crash/timeout/forgot) never becomes an APPROVE.
- Backend is ACP JSON stream (`pi --mode json` etc.), so the SID is parseable for resume.

The skill stays as-is for now; later it may become a thin wrapper around the CLI (deferred — OT4).

## Goals / Non-Goals

**Goals:**
- A cannot bypass, bias, or forge verification. The only way to obtain a hash is genuine n/m consensus.
- A's interface is minimal: `NEW` / `RESUME` / read rejection output. A can append fix notes but never edit the goal or write verdicts.
- Verifiers are real CLI-agent sessions (ACP JSON), spawned in parallel, naturally exiting after registering a verdict.
- A tamper-evident completion hash links back to the goal directory for audit.
- Pluggable backends (pi/hermes/acpx built-in, custom via config) sharing one ACP stream parser.
- Configurable n/m, maxTurn, timeout, diff size, and prompt/resume templates.

**Non-Goals:**
- Protecting against V* itself being biased by prompt framing (only A-side cheating is in scope; V* is trusted-as-fallible).
- Cost/round caps — A may retry indefinitely (OT7, closed).
- Per-verifier maxTurn refresh — v1 refreshes ALL verifiers after maxTurn rounds (OT2, deferred).
- `chattr +a` / separate-UID hardening of the salt (OT3, deferred). Same-UID A can technically read `.salt`; hash tamper-detection is the v1 guard.
- `audit` subcommand (OT1, deferred). The hash is generated but hash→goal reverse-lookup tooling comes later.
- Fan-out scouts (find similar issues across the codebase) — that is A's job, not the CLI's (OT6, deferred).
- Migrating the skill into a wrapper (OT4, deferred).

## Decisions

### D0 — Implementation language: Rust
The CLI is implemented in Rust. Rationale (full comparison in `flow/findings/2026-07-03-language-choice.md`):
- **Exhaustive `enum` + `match`** on ACP events and `VerdictStatus` — the compiler catches unhandled event types and invalid state transitions. A missed `agent_end`/`session` event is exactly the silent bug that breaks fail-closed (no SID → no resume → null verdict).
- **`Result<T,E>` enforced at compile time** — fail-closed is the central safety property of this design; Rust makes ignoring an error a compile error, Go's `error` is ignorable via `_ =`.
- **`Option<T>` instead of `nil`** — no nil-deref panics on the goal store / verdict slots.
- **Memory profile** — short-lived orchestrator (spawn → wait → hash → exit), baseline ~3MB per process vs Go ~10MB vs Node ~50MB. At "many loops running concurrently" this matters.
- **Single static binary** — no runtime dependency on the host; `jewilo`/`jewije` are drop-in.

Alternatives considered:
- **Go**: simpler project setup (~1 day saved), native `exec.CommandContext` kill-on-timeout, but string-typed enums (silent typos), default-case `switch` (silent missing-event), and ignorable `error` work against the fail-closed guarantee.
- **TypeScript (Node/Bun)**: rejected — runtime dependency, ~50MB baseline, no compiler-enforced error handling. Only wins if shared library code with pi were needed, but the contract is shelling out to `pi -p --mode json`.

The fail-closed design depends on every error path being explicit. Rust enforces this; the others do not. The ~1 day toolchain-setup overhead is accepted as the cost of compiler-enforced safety.

### D1 — Two CLIs, strict capability separation
`verifier-loop` (jewilo) is A's interface (NEW/RESUME, spawn, gather, consensus, hash). `verifier-verdict` (jewije) is V*'s interface (approve/reject, deterministic file write). Separation means A's process has no code path that writes verdicts.
**Alternative considered:** single CLI with role flag. Rejected — A could invoke the verdict writer directly.

### D2 — Identity via env var AND prompt `{{}}`
Spawn injects `VERIFIER_LOOP_GOAL_ID`, `VERIFIER_LOOP_VERIFIER_ID`, `VERIFIER_LOOP_ROUND` as env vars (machine-readable, read by `verifier-verdict` to locate the verdict path) AND renders the same values into the prompt (human-readable, so V* knows its role). Both are needed because the IDs differ each spawn and V* is an LLM that reads the prompt while `verifier-verdict` is a CLI that reads env.
**Alternative considered:** env-only. Rejected — V* wouldn't know it is a verifier. Prompt-only. Rejected — `verifier-verdict` couldn't find its path deterministically.

### D3 — ACP JSON stream as the universal backend contract
All backends emit the same event stream (`session` → `agent_start` → `turn_*` → `message_*` → `agent_end`). The SID for resume comes from the first `{"type":"session","id":"..."}` line. One parser serves all adapters; only the spawn/resume command templates differ.
**Alternative considered:** plain `pi -p` text output. Rejected (LD18) — no SID, no structured events, no reliable agent_end detection.

### D4 — Verdict = explicit deterministic CLI call, never pattern-matched
V* must run `verifier-verdict approve` / `reject --notes "..."`. The CLI writes `verdict.json` atomically. First write wins; subsequent calls are rejected. There is NO regex/keyword scanning of V* output (LD10) — that would let A or V* drift the verdict semantics.
**Alternative considered:** parse "APPROVE"/"REJECT" from V* final message. Rejected — fragile and gameable.

### D5 — Goal immutability via salted signature
At `NEW`, the CLI writes `goal.json` (goalText, context, createdAt, config snapshot) and `signature.json` containing `SHA256(salt + goalText + createdAt)`. The salt lives at `~/.verifier-loop/.salt` (mode 600), generated once. The signature is an input to the completion hash, so editing goalText after creation breaks every downstream hash.
**Alternative considered:** OS-level immutability (chattr +i). Deferred (OT3) — not portable.

### D6 — Completion hash formula
`completionHash = "vl:" + first40hex(SHA256(salt + goalId + goalSignature + String(roundNumber) + JSON.stringify(matchingVerdicts) + matchedAtISO))` where `matchingVerdicts` is sorted by verifierId for determinism. Each input guards a distinct tamper vector (see turn4 table). `vl:` prefix aids audit grep.
**Alternative considered:** include all rounds' verdicts. Rejected — only the matching round matters for the proof.

### D7 — Parallel non-blocking spawn + gather
Round execution spawns all m verifiers concurrently, each as its own ACP process with injected env. The loop waits for all `agent_end` events (or timeout → NULL verdict), then reads verdict files and evaluates n/m. A is blocked only at the gather barrier.
**Alternative considered:** sequential spawn. Rejected (LD23) — slower, no benefit.

### D8 — Session reuse up to maxTurn, then fresh
`RESUME` reuses a verifier's SID via `pi --session <sid> -p "..." --mode json` if its `turnsUsed < maxTurn`; otherwise spawns fresh and archives the old SID. v1 refreshes all verifiers together (round-based). The round env var increments on resume; verifierId stays stable.
**Alternative considered:** per-verifier refresh counter. Deferred (OT2/OT3).

### D9 — Fail-closed verdict semantics
`verdict.json` is pre-created with `status: null` at spawn time. The only transitions are null→APPROVE or null→REJECT (first call wins). null after gather = no-pass. Timeout (default 1800s) kills the process and leaves null. Deleting the store yields no hash and no proof.
**Alternative considered:** default-to-approve on timeout. Rejected (LD16) — silent-pass is the exact failure we are preventing.

### D10 — Verifier prompt is blind + frozen-artifact
V* receives: the baked-in verifier policy, the immutable goalText, optional context, A's fix notes (resume only), V*'s own previous notes (resume only, template-gated), and a frozen snapshot (cwd, `git status --porcelain`, file edit times, `git diff` top `gitDiffMaxChars`). V* does NOT receive: round number (LD12), other verdicts, n/m config, the completion hash, or the spawn internals.

## Risks / Trade-offs

- **[Salt readable by same-UID A]** → v1 relies on hash tamper-detection (recompute ≠ stored). Mitigation deferred to OT3 (`chattr +a` or dedicated UID). Acceptable because forging still requires the full input set + formula.
- **[V* false-APPROVE at 40%]** → mitigated by n/m. Document the math for operators; default n=m=2 (unanimous) for high-stakes, allow 2/3 or 3/5 where recall matters.
- **[V* forgets to call verdict]** → pre-created null verdict + fail-closed. A sees a rejection with "verifier did not register a verdict" and must RESUME.
- **[ACP stream format drift across backends]** → one shared parser with per-adapter spawn/resume templates; add a parser conformance test per backend.
- **[ Hung verifier blocks gather]** → per-verifier timeout (default 1800s) kills the process, verdict stays null, round fails closed.
- **[Goal store grows unbounded]** → out of scope for v1; future `prune`/`audit` subcommand (OT1).
- **[Skill and CLI drift]** → skill stays the source of verifier *policy* text; CLI bakes that text into the default prompt template. Keep them in sync by reading the skill file at build time.

## Migration Plan

Greenfield repo — no migration of existing data. Rollout:
1. Implement `verifier-loop` + `verifier-verdict` in `../verifier-loop` (TypeScript/Node, matches the pi ecosystem).
2. Add built-in ACP adapters (pi primary, hermes + acpx stubs) + custom-adapter config path.
3. Install binaries `verifier-loop`→`jewilo`, `verifier-verdict`→`jewije` on PATH.
4. Smoke test: `verifier-loop NEW "say hi works"` with n=m=1 against `pi --mode json`; assert a hash is produced and the goal dir is populated.
5. Rollback: remove the binaries and `~/.verifier-loop/`. The skill in pi-plugins continues to work independently.

## Open Questions

- OQ1: Should `verifier-verdict` refuse to run outside a spawned env (no `VERIFIER_LOOP_GOAL_ID`)? (Lean yes — defence in depth; document the error.)
- OQ2: Exact JSON shape of `matchingVerdicts` in the hash input — keep minimal (verifierId, status, notes, registeredAt, sid) or full verdict? (Lean minimal; full verdict is already on disk for audit.)
- OQ3: When a verifier session is reused, does the *prior round's* initial-prompt get overwritten or versioned? (Lean versioned under `round-N/vK/` to preserve the trust trail.)
