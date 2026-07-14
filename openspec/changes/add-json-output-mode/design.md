## Context

The `verifier-loop` crate ships two binaries — `jewilo` (`verifier-loop`, subcommands `NEW` / `RESUME` / `RECOVER` / `STATUS`) and `jewije` (`verifier-verdict`, subcommands `approve` / `reject`). Their entire stdout surface today is free text:

- `jewilo NEW` / `RESUME` → `goalId: <id>` then a bare `mmddyy-XXXXXXXX` short hash on success, or a multi-line stderr rejection summary on failure.
- `jewilo RECOVER` → bare short hash, or stderr rejection.
- `jewilo STATUS` → already pretty-prints a JSON object (the only structured output today, but it is a bare body, not a uniform envelope).
- `jewije approve` / `reject` → `Verdict registered` on success, free-text error on stderr.

An outer driving agent, a CI consumer, or any wrapper script must today scrape and regex these lines. The success/failure signal is conflated with exit code + prose, and rich fields (round, hash, needs, rejection breakdown) live on separate stdout/stderr lines with no single parse point. This change adds a stable `--json` machine-readable mode to every command.

Constraints that any design MUST preserve (project fail-closed invariants, see `AGENTS.md`):

- A NULL verdict never becomes APPROVE.
- A missing store yields no hash.
- `goalText` edit → signature mismatch → hash mismatch.
- Verdict edit → hash mismatch.
- The completion-hash **inputs** are exactly `(salt, goalId, goalSignature, roundNumber, canonicalJSON(matchingVerdicts), matchedAtISO, receiptLogHead)`. The JSON envelope MUST NOT extend this set.
- Exit codes are part of the public contract. `--json` MUST NOT alter them.

Stakeholders: the outer driving agent (the source of the `.jewilo-*` CWD bloat per `AGENTS.md`), CI consumers, and any future programmatic wrapper. Humans reading the terminal still rely on the default free-text path.

## Goals / Non-Goals

**Goals:**
- Every `jewilo` and `jewije` command supports a top-level `--json` flag.
- Exactly one JSON object is emitted on stdout per invocation, conforming to a single stable envelope schema (camelCase, matching the on-disk artifact convention).
- Default (no `--json`) behavior is byte-identical to today — zero breaking change for humans or existing scripts.
- On-disk artifacts (`completion.json`, `receipt-log.jsonl`, `trace.jsonl`, goal/round files) are byte-identical with and without `--json`.
- Exit codes, hash inputs, verdict semantics, and signature verification are unchanged by `--json`.
- Human diagnostics remain on stderr; stdout under `--json` is the single structured parse point.

**Non-Goals:**
- A streaming / NDJSON mode (one object per phase). Out of scope: one object per invocation only.
- Changing the on-disk JSON formats. `completion.json` / `receipt-log.jsonl` / `trace.jsonl` are untouched.
- Replacing the human-readable default. `--json` is opt-in.
- Machine-readable tracing export beyond what the existing `add-otel-observability` layer already provides.
- Versioning the envelope via a top-level `schema` field. (A consumer-tolerant camelCase schema is enough for v1; a `schema` version can be added later as an additive field.)

## Decisions

### D0 — Single root JSON object, not NDJSON, not one-object-per-phase
**Choice:** One JSON object total on stdout per invocation.
**Rationale:** Consumers want one parse. Multi-phase streaming would force consumers to assemble state and would couple them to internal phase names. One object keeps the contract minimal and stable.
**Alternative considered:** NDJSON with one event per phase (round.start, spawn, consensus). Rejected: it leaks internal phase taxonomy into the public contract and makes "did it pass?" require scanning all lines. Internal phases are already in `trace.jsonl`.

### D1 — One envelope type for both binaries; per-command fields are optional
**Choice:** A single `JsonEnvelope` struct (serde) shared by `jewilo` and `jewije`. Fields like `hash`, `fullDigest`, `needs`, `rejection`, `verifierId`, `error` are `Option<…>` and emitted only when relevant (serde `skip_serializing_if = "Option::is_none"`).
**Rationale:** Consumers parse one schema regardless of which binary they invoked. Per-command richness is additive, not a different type.
**Alternative considered:** One struct per command (`NewResult`, `RecoverResult`, …). Rejected: doubles the consumer surface and makes a generic wrapper script impossible.

### D2 — Top-level global flag, not per-subcommand
**Choice:** `--json` / `-j` lives on the top-level `Cli` of each binary (before the subcommand), so `jewilo --json NEW "<goal>"` and `jewilo NEW "<goal>" --json` both parse. clap supports both placements when the flag is on the top-level struct and the subcommands are flattened.
**Rationale:** Matches how users expect global flags; survives subcommand reshuffles.
**Alternative considered:** Per-subcommand flag. Rejected: must be re-added on every future subcommand; easy to forget; inconsistent UX.
**Risk:** clap arg placement semantics — verify with a RED test that both placements work; if clap forces one placement, document the chosen one explicitly.

### D3 — STATUS is wrapped in the envelope only under `--json`
**Choice:** Without `--json`, `STATUS` prints its current bare JSON body (byte-identical). With `--json`, the same fields are lifted into the standard envelope (`ok:true`, `command:"status"`, plus `round`/`state`/`needs`/`verdicts`).
**Rationale:** STATUS is the one command that already prints JSON; wrapping it conditionally keeps default behavior identical while unifying the machine-readable schema. A consumer that wants one schema always passes `--json`.
**Alternative considered:** Always wrap STATUS. Rejected: breaks byte-identity for any existing STATUS consumer.

### D4 — Diagnostics stay on stderr; structured equivalents ride the envelope
**Choice:** All human-readable lines (cooldown notice, recoverable-round hint, prompt-budget warning, captured V* stderr preview, tracing init notes) stay on stderr under `--json`. The envelope carries the structured equivalent a consumer needs (`status:"cooldown-fallback"`, `status:"recover-null-after-timeout"`, null-verifiers list) so stdout is the single parse point.
**Rationale:** Stdout must be pure JSON. Anything that would corrupt the single-object contract goes to stderr.
**Alternative considered:** Emit diagnostics as an `warnings` array in the envelope. Rejected: doubles the surface and risks leaking free-text into stdout; stderr is the right channel for human chatter.

### D5 — Envelope field names camelCase; sorted arrays
**Choice:** Envelope uses camelCase (`goalId`, `fullDigest`, `nullVerifiers`, `rejectNotes`, `signatureFailures`) to match the existing on-disk artifact convention. Arrays in `rejection` are sorted by verifierId ascending for determinism.
**Rationale:** Consistency with `completion.json` / `receipt-log.jsonl` / `trace.jsonl`. Determinism makes golden-file testing and consumer equality checks trivial.
**Alternative considered:** snake_case to match Rust conventions. Rejected: the project's persisted JSON is camelCase; the envelope is a persisted-shape sibling.

### D6 — Output routed through a small formatter, not scattered `println!`
**Choice:** Introduce a thin output abstraction (a `Reporter` enum or two functions `print_human(...)` / `print_json(...)`) chosen once at startup from the parsed flag. Every existing `println!`/`eprintln!` success-site in both bins routes through it.
**Rationale:** Centralizes the "is this a JSON run?" decision so it is impossible to forget at one site. Keeps the diff mechanical.
**Alternative considered:** Inline `if json { … } else { … }` at each site. Rejected: error-prone, easy to leak a legacy line into JSON stdout.

### D7 — `--json` failures still print the envelope on stdout, not only stderr
**Choice:** On a fatal error under `--json` (missing store, unreadable config, validation failure), stdout gets exactly one envelope object `{ok:false, error:"…"}` and the process exits non-zero. The human-readable error stays on stderr as a debugging aid.
**Rationale:** A programmatic consumer reads stdout for the structured result even on failure; relying only on exit code + stderr forces consumers back to scraping.
**Alternative considered:** Only stderr + exit code on error. Rejected: defeats the purpose of `--json` for the failure paths that matter most (validation, config errors).

### D8 — New helper module, no new dependencies
**Choice:** Add the envelope + formatter in a new small module (e.g. `src/cli/json_output.rs`, or fold into `src/cli/mod.rs`). Use the already-present `serde` + `serde_json` deps. No new crate dependencies.
**Rationale:** Zero new supply-chain surface; minimal blast radius.
**Alternative considered:** A dedicated `reporting` crate. Rejected: premature.

## Risks / Trade-offs

- **Risk:** clap global-flag placement differs between `jewilo` (top-level struct with subcommand) and `jewije` (subcommand-only today). → **Mitigation:** Put `--json` on the top-level `Cli` of each; RED test both `jewilo --json NEW …` and `jewilo NEW … --json` placements; if clap rejects one, document the supported placement and update the spec scenario accordingly.
- **Risk:** A future contributor adds a new `println!` in a bin and forgets the JSON gate, leaking a legacy line into JSON stdout. → **Mitigation:** Route all success-path output through the formatter (D6); add a RED test that asserts stdout under `--json` is valid JSON for every command path; the test fails loudly on any leak.
- **Risk:** Envelope schema drift over time breaks consumers. → **Mitigation:** Keep fields additive (`Option` + skip-if-none); document that consumers MUST tolerate unknown fields. A future `schema` version field is additive.
- **Risk:** Lifting STATUS fields into the envelope under `--json` is confused with changing the bare STATUS body. → **Mitigation:** Two explicit RED tests — one asserting byte-identity without `--json`, one asserting envelope shape with `--json`.
- **Risk:** Error envelope on stdout could leak sensitive context (e.g. a path containing a username). → **Mitigation:** Error strings are the same human-facing messages already printed; no new secret surface. The salt is never printed (existing invariant preserved).
- **Trade-off:** Single root object (D0) means a long-running NEW round's progress is invisible on stdout until the end. Accepted: progress belongs to tracing/stderr, not the result channel.

## Migration Plan

1. Implement behind the opt-in `--json` flag (default off) — no migration required for existing users.
2. Document the envelope in `README.md` and add a one-line note to `AGENTS.md`.
3. The outer driving agent (per `AGENTS.md` the source of `.jewilo-*` bloat) may adopt `--json` to replace its scraping logic; adoption is optional and per-consumer.
4. Rollback: removing the flag reverts output to today. No on-disk artifact depends on it.

## Open Questions

- Should the envelope carry a `schema` version field from day one, or defer until a v2 is needed? (Current decision D5 leans defer; revisit if a consumer requests it.)
- Should `jewije` envelope include the registered verdict's `status` (APPROVE/REJECT) as a top-level field distinct from the lifecycle `status:"verdict-registered"`? (Current decision: lifecycle status only; the approve/reject distinction is already in `command`.)
