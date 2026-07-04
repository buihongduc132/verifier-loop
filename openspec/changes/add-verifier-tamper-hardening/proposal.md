## Why

A verifier (V*) sub-agent on the same machine as the arbiter (A) can today forge any verdict it likes: `jewije` trusts `VERIFIER_LOOP_VERIFIER_ID` env with no authentication, the `first-write-wins` rule only blocks overwrites (not the initial fill of a `null` slot), and the completion hash is computed *from* the verdicts — so an in-flight edit produces a "valid" hash over a lie. The current hash scheme is a post-hoc audit trail, not a prevention mechanism; a same-box process with filesystem access is the defendant, judge, and forensics lab combined.

## What Changes

- **Per-verifier signing key.** jewilo mints a fresh random Ed25519 keypair per V* at spawn and writes the public half into the slot (`verifier-pubkey.json`); the secret is passed to V* via an env var the spawning process holds but A's code never reads.
  - **BREAKING**: `jewije approve` / `reject` now signs the verdict with the per-verifier secret; a verdict whose signature fails to verify against the slot's pinned pubkey is refused at registration time.
- **Null-slot first-fill hardening.** Only the V* that proves possession of the slot's pinned secret may flip a `null` placeholder to APPROVE/REJECT. A second `jewije` call from a different identity (even with env spoofed) fails closed.
- **Signed verdict record.** `verdict.json` gains `signature` (Ed25519 over the canonical record bytes) and `pubkeyId` fields; consensus evaluation rejects any verdict whose signature does not verify against the slot's pinned pubkey (in-flight edits to `status`/`notes`/`registeredAt` invalidate the signature).
- **Append-only receipt log.** Every `jewije` invocation appends a hash-chained entry to `~/.verifier-loop/goals/<id>/receipt-log.jsonl`; the chain head hash is folded into the completion hash inputs, so retroactive edits to the log are detectable by an auditor.
- **Honest same-box limitation docs.** README + a new `THREAT-MODEL.md` state plainly that same-box jewilo cannot prevent a process with `~/.verifier-loop/` write access from forging; the new signing only raises the bar to "must possess the per-verifier secret" (which the spawning process can leak if A's code is compromised). True prevention requires out-of-process V* on a separate host.

## Capabilities

### New Capabilities
- `verifier-identity`: per-verifier keypair minting at spawn, secret handoff to V*, pinned-public-key slots, and signature verification on verdict registration. Guards against identity spoofing (env-var forgery) and null-slot first-fill by an unauthenticated caller.
- `signed-verdict-record`: the on-disk verdict gains an Ed25519 signature over its canonical bytes; consensus evaluation and the audit recompute both verify the signature against the slot's pinned pubkey before trusting the verdict. Guards against in-flight verdict edits.
- `receipt-log`: append-only, hash-chained ledger of every `jewije` write, with its head hash folded into the completion-hash inputs so log tampering breaks the stored hash.

### Modified Capabilities
- `verdict-registration`: requirement changes — `register_approve` / `register_reject` now take a signing secret, produce a signed record, and refuse unsigned / bad-signature writes; first-fill of a `null` slot now requires proof of the pinned secret (not just any caller).
- `completion-proof`: requirement changes — the hash inputs now include the receipt-log head hash, and consensus evaluation verifies verdict signatures (not just parses the JSON) before treating a verdict as matching.
- `verifier-spawn`: requirement changes — the spawn layer now mints a per-verifier keypair, persists the public key to the slot, and injects the secret into V*'s environment instead of (only) the `VERIFIER_LOOP_VERIFIER_ID` env var.

## Impact

- **Code**: `src/verdict/` (signing + verification on the write/read paths), `src/spawn/` (keypair minting + secret injection), `src/consensus/` (signature verification before matching + receipt-log head in hash inputs), `src/store/` (new per-slot `verifier-pubkey.json` + `receipt-log.jsonl` files), `src/bin/verifier_verdict.rs` (CLI accepts the secret + produces signed writes), `src/bin/verifier_loop.rs` (spawns V* with the secret env).
- **Dependencies**: add `ed25519-dalek` + `rand` (or `getrandom`) crates; `serde_json` already present for canonical serialization.
- **APIs/CLI**: `jewije approve` / `jewije reject --notes "…"` are **BREAKING** — they now require `VERIFIER_LOOP_VERIFIER_SECRET` env (injected by jewilo at spawn). Direct manual invocation without the secret fails closed. jewilo's spawn path injects it automatically, so stub-backbone e2e tests need a trivial fixture update.
- **Specs**: `verdict-registration`, `completion-proof`, `verifier-spawn` delta specs; three new specs (`verifier-identity`, `signed-verdict-record`, `receipt-log`).
- **Docs**: README threat-model section + new `THREAT-MODEL.md`; honest statement of what same-box jewilo can and cannot guarantee.
- **Out of scope (explicit non-goals)**: out-of-process V* on a separate host (infra change, not code); hardware-backed keys (TPM/YubiKey); HSM integration; protecting against a fully-compromised A process that can read the spawned secret out of its own child env (same-box fundamental limit — documented, not solved).
