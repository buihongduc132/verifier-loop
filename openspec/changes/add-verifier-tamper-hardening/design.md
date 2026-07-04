## Context

`jewilo`/`jewije` today produce a tamper-evident completion hash (`mmddyy-XXXXXXXX` + `fullDigest`) computed over `salt + goalId + goalSignature + round + canonicalJSON(matchingVerdicts) + matchedAt`. The hash is a **post-hoc audit trail**: an auditor who re-reads the goal dir later will detect edits to `goalText`, a verdict's `status`/`notes`/`registeredAt`, or the matched set.

But the hash does not **prevent** tampering in the window before consensus is evaluated. Forensics on the same machine as the defendant:

- `jewije` resolves the verifier identity from `VERIFIER_LOOP_VERIFIER_ID` env with **no authentication** — any process that can set env vars and reach the filesystem can write an APPROVE into any V*'s slot.
- `first-write-wins` only blocks **overwriting** a non-null verdict. A `null` placeholder slot can be filled by **anyone** before the legitimate V* verdicts — pre-emptive consensus.
- The completion hash is computed **from** the verdicts. If a verdict is edited in-flight (before eval), the recomputed hash is "valid" over the lie; detection only works for *post*-completion edits.

The result: a same-box sub-agent (or A itself) is simultaneously defendant, judge, and forensics lab. The current scheme coordinates honest agents and audits after the fact; it cannot resist a malicious same-box process.

Stakeholders: operator (A, the arbiter process that drives jewilo), verifier sub-agents (V*, spawned as real ACP-JSON CLI processes), and the auditor (any process re-reading the goal dir + salt to recompute the hash).

Constraints inherited from `add-verifier-loop-cli`:
- D2 — identity comes from `VERIFIER_LOOP_*` env (env wins over args; no arg override).
- D9 — fail-closed; a NULL verdict never becomes APPROVE.
- The store root is a directory tree under `VERIFIER_LOOP_HOME` (default `~/.verifier-loop/`).
- `jewilo` is out-of-process relative to A's reasoning loop but runs on the same host.

## Goals / Non-Goals

**Goals:**
- Raise the bar for same-box forgery from "set an env var" to "must possess the per-verifier secret pinned at spawn."
- Make in-flight verdict edits (between registration and consensus eval) fail signature verification, not merely post-hoc-auditable.
- Make the receipt log tamper-evident by hash-chaining it and folding its head into the completion hash inputs.
- Document, honestly and prominently, what same-box jewilo can and cannot guarantee — so operators do not mistake the hash for a prevention mechanism.

**Non-Goals (explicit):**
- Out-of-process V* on a separate host (infra/deploy change, not code). This is the only true fix for same-box forgery; we make the code-side bar higher but do not claim it is unbreakable.
- Hardware-backed keys (TPM, YubiKey, HSM).
- Protecting against a fully-compromised A process that can read its own child process environment (the spawned secret lives in V*'s env; A's compromised code can read it). Documented as the fundamental same-box limit, not solved.
- Per-verifier `maxTurn` refresh (OT2), audit subcommand (OT1), `chattr +a` (OT3) — unrelated orthogonal tracks.
- Migrating historical goal dirs (pre-change). Existing `verdict.json` files without signatures are rejected by the new code; old goals remain readable for audit but cannot be re-evaluated. A `--legacy-audit-mode` flag may be added if needed.

## Decisions

### D0 — Cryptographic primitive: Ed25519
**Choice:** Ed25519 via `ed25519-dalek` + `rand`/`getrandom` for keypair generation.
**Why over alternatives:**
- EdDSA vs ECDSA: Ed25519 has deterministic signatures (no nonce-reuse footgun), small fixed-size 64-byte sigs, and a single well-vetted Rust impl (`ed25519-dalek`, audited).
- Asymmetric vs HMAC: an HMAC with a shared jewilo/jewije key would be simpler but would not separate V* identity — anyone with the shared key could forge any V*. We need per-verifier keys, which means asymmetric.
- Ed25519 vs RSA-PSS: RSA keys + sigs are 10-20× larger; no benefit at this scale.
**Alternative considered:** Noise-style static-static handshake. Rejected — overkill; we don't need a channel, just signed records.

### D1 — Per-verifier keypair minted at spawn, pubkey pinned to slot
**Choice:** The spawn layer (`src/spawn/`) mints a fresh `SigningKey` per V* immediately before launching the V* process. It writes `verifier-pubkey.json` (`{pubkey: <hex>, mintedAt: <iso>}`) into the slot dir and injects the `SigningKey` hex into V*'s env as `VERIFIER_LOOP_VERIFIER_SECRET`.
**Why pin the pubkey to the slot:** consensus evaluation + verdict verification compare the verdict signature against **this slot's pinned pubkey**, not any "current" pubkey. This means a V* cannot silently rotate its key mid-run, and a forger cannot mint a fresh keypair and overwrite the pinned pubkey file without that overwrite being itself detectable (the pubkey file's `mintedAt` + the slot dir's immutability semantics).
**Alternative considered:** jewilo holds a key registry, V* sends pubkey at first verdict. Rejected — jewilo then becomes a trusted third party on the same box; the forger just calls jewilo's registry API.

### D2 — Verdict signature scope: canonical record bytes
**Choice:** The signature covers `status || notes || registeredAt || goalId || verifierId || round` (length-prefixed canonical bytes — `serde_json` with `BTreeMap` keys is acceptable since the field set is closed and small).
**Why include identity in the signature:** prevents moving a verdict from one slot to another (cut-and-paste across `verifierId`/`round`/`goalId`).
**Alternative considered:** sign only `{status, notes, registeredAt}`. Rejected — a verdict signed by legitimate V*1 could be copied into V*2's slot by anyone with fs access.

### D3 — First-fill of `null` slot now requires the pinned secret
**Choice:** `register_approve` / `register_reject` verify the caller's signature against the slot's pinned pubkey **before** flipping `null` → real verdict. A caller without the secret fails closed with `VerdictError::Unauthenticated`.
**Why:** closes the "pre-emptive consensus" hole where a `null` slot is filled before the legitimate V* verdicts.
**Backward impact:** any `jewije` invocation without `VERIFIER_LOOP_VERIFIER_SECRET` now fails. Direct manual invocation breaks; jewilo's spawn path injects it automatically.

### D4 — Receipt log: append-only JSONL, hash-chained, head folded into completion hash
**Choice:** Every `jewije` write appends one line to `~/.verifier-loop/goals/<goalId>/receipt-log.jsonl`: `{seq, kind, verdictId, status, prevHash, entryHash, signedBy: <pubkeyId>}`. `entryHash = SHA256(prevHash || canonicalEntryFields)`. The chain head (last `entryHash`) is appended to the completion-hash inputs alongside `matchingVerdicts`.
**Why hash-chain + fold into completion hash:** a retroactive edit to the log (delete/insert/modify a line) breaks the chain and/or the stored completion hash; both are detectable by an auditor.
**Alternative considered:** SQLite WAL. Rejected — adds a runtime dependency and a binary format that's harder to audit by eye. JSONL is append-only-friendly and `cat`-able.

### D5 — `signed-verdict-record` schema
**Choice:** `verdict.json` gains two fields:
```json
{
  "status": "APPROVE",
  "notes": null,
  "registeredAt": "2026-07-04T...",
  "signature": "<128-hex Ed25519 over canonical record bytes>",
  "pubkeyId": "<first16hex of pinned pubkey>"
}
```
The legacy null placeholder stays `{status: null}` (no signature — it's not a verdict).

### D6 — Honest limitation documented in THREAT-MODEL.md
**Choice:** A new top-level `THREAT-MODEL.md` plus a README section that states plainly:
- Same-box jewilo raises the bar to "must possess the per-verifier spawn-time secret" but cannot prevent a process with `~/.verifier-loop/` write access from forging if it can also read the spawned secret out of V*'s env.
- True prevention requires out-of-process V* on a separate host (a deploy change, not code).
- The new signing is a **deterrent + detection** layer, not a **prevention guarantee**.

### D7 — Canonical serialization for signatures
**Choice:** `serde_json::to_string` with `BTreeMap`-based serialization (sorted keys, no whitespace) over the `{status, notes, registeredAt, goalId, verifierId, round}` tuple. The exact byte string is reproducible by an auditor with the same field set.
**Why not a custom binary encoder:** simpler to audit, deterministic enough for a closed 6-field set, and matches the canonical-JSON approach already used in `consensus/mod.rs` for `matchingVerdicts`.

### D8 — Crates
**Choice:** `ed25519-dalek = "2"`, `rand = "0.8"` (or `getrandom` if we want zero deps for randomness). Both are well-audited and pure-Rust (no OpenSSL).

## Risks / Trade-offs

- **[R1] Spawned secret in V* env is readable by a compromised A** → D6 documents this as the fundamental same-box limit; the only true fix is out-of-process V* (non-goal). Mitigation: the secret is scoped to V*'s env only, not persisted to disk by jewilo; A's *honest* code never reads it.
- **[R2] BREAKING: manual `jewije` invocation now fails without the secret** → documented in proposal + README migration section. Stub-backend e2e tests get a trivial fixture update (the stub script receives the env from the spawn layer automatically). Operators who scripted `jewije` directly must migrate to driving it via `jewilo` or accept the secret-env requirement.
- **[R3] `ed25519-dalek` adds a transitive dependency surface** → pinned to v2 (audited); `cargo audit` gates releases.
- **[R4] Receipt log grows unboundedly** → capped per-goal by `maxTurn * m` entries (bounded by config); no GC needed within a goal. Cross-goal GC is an ops concern, out of scope.
- **[R5] Canonical-JSON signature byte-reproducibility across versions** → D7 fixes the field set; a future field addition changes the canonical bytes and breaks old sigs (intended — signatures are per-record-version). Auditor code must pin the field set per record version.
- **[R6] Existing pre-change goal dirs become unverifiable under new code** → documented; old goals are read-only audit artifacts. `--legacy-audit-mode` flag is a follow-up if operators need to re-evaluate old goals under the new code.

## Migration Plan

1. Land the change behind no feature flag — signing is the new contract.
2. Stub-backend e2e tests updated in the same PR (the stub receives the secret env from spawn automatically; no test-side secret management needed).
3. README + THREAT-MODEL.md ship with the change.
4. Rollback: revert the PR; old goals remain readable; new (signed) goals become unverifiable under old code (acceptable — new goals created post-rollback are unsigned and pass old code).
5. No data migration script — pre-change goals are read-only audit artifacts.

## Open Questions

- **OQ1:** Should the receipt log be per-goal (`goals/<id>/receipt-log.jsonl`) or global (`~/.verifier-loop/receipt-log.jsonl`)? Proposal: per-goal (bounded, simpler GC, matches the per-goal completion hash). Confirm in spec.
- **OQ2:** Should `pubkeyId` be `first16hex` of the pubkey, or a full hash? Proposal: `first16hex` is enough for human reading; the full pubkey is in `verifier-pubkey.json`. Confirm.
- **OQ3:** Should consensus evaluation treat a verdict with a *valid signature against a non-pinned pubkey* as fail-closed, or as a distinct error? Proposal: fail-closed (treat as untrusted), with a distinct error message in the rejection summary.
