## 1. Dependencies + scaffolding

- [ ] 1.1 Add `ed25519-dalek = "2"` and `rand = "0.8"` to `Cargo.toml` `[dependencies]`; verify `cargo build` clean and `cargo audit` clean.
- [ ] 1.2 Create `src/crypto/mod.rs` exposing `Keypair`, `sign(canonical_bytes, &secret) -> Vec<u8>`, `verify(sig, canonical_bytes, &pubkey) -> bool`, `pubkey_id(pubkey) -> String` (first 16 hex), and `canonical_record_bytes(status, notes, registeredAt, goalId, verifierId, round) -> Vec<u8>` (BTreeMap-sorted JSON, no whitespace).
- [ ] 1.3 Add unit tests for `src/crypto/mod.rs`: keypair freshness, sign/verify round-trip, signature fails on byte-flip, `canonical_record_bytes` deterministic across key-orders.

## 2. Verifier identity — pinned pubkey (NEW spec: verifier-identity)

- [ ] 2.1 Add `VerifierPubkeyFile { pubkey: String (hex), mintedAt: String (iso) }` to `src/verdict/mod.rs` with serde + read/write helpers; `pubkey_path(root, goal_id, verifier_id, round)` mirrors `verdict_path`.
- [ ] 2.2 Add `mint_and_pin_pubkey(root, goal_id, verifier_id, round) -> Result<SigningKey, VerdictError>` that mints a fresh Ed25519 keypair, writes `verifier-pubkey.json` (fail-closed if it already exists — pinned is immutable), and returns the signing key to the caller (caller injects it into V* env; never persisted).
- [ ] 2.3 Add `read_pinned_pubkey(root, goal_id, verifier_id, round) -> Result<Option<VerifyingKey>, VerdictError>`: returns `None` if the file is absent (caller treats absence as Unauthenticated per spec), `Some(key)` if present. Earliest-mtime entry wins if duplicate writes are detected (immutability enforcement).
- [ ] 2.4 RED: `tests/verdict.rs` — `mint_and_pin_pubkey` writes `verifier-pubkey.json` before returning; second call on same slot fails; fresh keypairs across `v1`/`v2`; missing file on read returns `None`.
- [ ] 2.5 GREEN: implement until RED passes.

## 3. Signed verdict record (NEW spec: signed-verdict-record)

- [ ] 3.1 Extend `VerdictRecord` with `signature: Option<String>` (128 hex) and `pubkey_id: Option<String>` (16 hex); null placeholder keeps both as `None` (skip_serializing_if). Update `Default`/tests.
- [ ] 3.2 Add `verify_record(record, pinned_pubkey, goal_id, verifier_id, round) -> Result<(), VerdictError>`: recompute canonical bytes from the record's `{status, notes, registeredAt}` + the caller-supplied identity, verify the signature against the pinned pubkey, and reject on mismatch with distinct errors `BadSignature` vs `WrongPubkey`.
- [ ] 3.3 RED: `tests/verdict.rs` — APPROVE record carries `signature` + `pubkeyId`; signature binds identity (copy across `verifierId` fails verify); REJECT record signature covers notes; null placeholder has no signature field.
- [ ] 3.4 GREEN: implement until RED passes.

## 4. Verdict registration requires the pinned secret (MODIFIED spec: verdict-registration)

- [ ] 4.1 Add `ENV_VERIFIER_SECRET = "VERIFIER_LOOP_VERIFIER_SECRET"` to `src/bin/verifier_verdict.rs`; resolve it from env (D2 env-wins). Missing/empty → `VerdictError::Unauthenticated`.
- [ ] 4.2 Change `register_approve` / `register_reject` signatures to accept `secret: &SigningKey`; they compute the canonical record bytes, sign, write the signed `verdict.json`. First-fill of a `null` slot now requires the pubkey derived from `secret` to equal the slot's pinned pubkey (else `Unauthenticated`).
- [ ] 4.3 RED: `tests/verdict.rs` + `tests/cli_e2e.rs` — `jewije approve` without `VERIFIER_LOOP_VERIFIER_SECRET` exits non-zero, no verdict written; with a non-matching secret exits non-zero; with the correct secret writes a signed verdict that verifies.
- [ ] 4.4 GREEN: wire `bin/verifier_verdict.rs` to read the env, derive the pubkey, compare to pinned, sign, write. Until RED passes.

## 5. Receipt log (NEW spec: receipt-log)

- [ ] 5.1 Add `ReceiptEntry { seq, kind, verdictId, status, prevHash, entryHash, signedBy }` and `append_receipt(root, goal_id, kind, verdictId, status, pubkey_id) -> Result<String, VerdictError>` to a new `src/receipt/mod.rs`; `entryHash = SHA256(prevHash || canonicalFields)`; first entry `prevHash = ""`; returns the new chain head.
- [ ] 5.2 Wire `register_approve` / `register_reject` to call `append_receipt` AFTER the atomic verdict write succeeds; the receipt append is part of the same logical transaction (if it fails, the verdict write is logically rolled back by treating the slot as corrupt — documented fail-closed).
- [ ] 5.3 Add `read_receipt_head(root, goal_id) -> String` (empty string if log absent) for use by consensus.
- [ ] 5.4 RED: `tests/receipt.rs` — approve appends one chained line; second entry chains `prevHash`; mid-log edit breaks the chain on recompute; trailing-line deletion detected by head mismatch.
- [ ] 5.5 GREEN: implement until RED passes.

## 6. Spawn mints keypair + injects secret (MODIFIED spec: verifier-spawn)

- [ ] 6.1 In `src/spawn/` (orchestrator), before launching each V* process: call `mint_and_pin_pubkey(...)`, capture the signing key, and add `VERIFIER_LOOP_VERIFIER_SECRET=<hex>` to the process `Command` env alongside the existing identity env vars.
- [ ] 6.2 Ensure stub/custom backend path receives the env identically to the `pi` backend (no backend-specific branching).
- [ ] 6.3 RED: `tests/spawn_orchestrator.rs` + `tests/cli_e2e.rs` — spawned process env contains `VERIFIER_LOOP_VERIFIER_SECRET`; pinned pubkey exists before launch; stub-backend end-to-end `jewilo NEW` produces a signed APPROVE verdict + completion hash (full closed loop).
- [ ] 6.4 GREEN: wire the env injection; update stub scripts if needed. Until RED passes.

## 7. Consensus verifies signatures + folds receipt head (MODIFIED spec: completion-proof)

- [ ] 7.1 In `src/consensus/`, before counting an APPROVE verdict toward n/m: call `verify_record(record, read_pinned_pubkey(...)?, goal_id, verifier_id, round)`; on failure mark the verdict untrusted and add it to the rejection summary with the slot + failure reason.
- [ ] 7.2 Extend `compute_hash` inputs to append `read_receipt_head(root, goal_id)` after `matchedAtISO`. Update the doc-comment formula + the `matchingVerdicts`/`hash`/`fullDigest` unit tests to include a receipt-head fixture.
- [ ] 7.3 RED: `tests/consensus.rs` — in-flight edit of `status`/`notes`/`registeredAt` after registration but before eval → verdict not counted + named in rejection; verdict signed by non-pinned key fails closed; receipt-head included in inputs (two runs differing only in receipt head produce different hashes).
- [ ] 7.4 GREEN: implement until RED passes.

## 8. CLI + store wiring

- [ ] 8.1 `src/bin/verifier_loop.rs`: spawn path mints per-verifier keypair and injects secret (consumes §6); no change to the RESUME path beyond threading the secret into re-spawned V*.
- [ ] 8.2 `src/bin/verifier_verdict.rs`: resolve secret from env; sign + write + append receipt (consumes §4 + §5).
- [ ] 8.3 `src/store/`: no schema change to `Config`; new per-slot files (`verifier-pubkey.json`, `receipt-log.jsonl`) live under the goal dir, not the config dir.
- [ ] 8.4 Update `scripts/stub_approve.sh` (and any sibling stubs) to inherit `VERIFIER_LOOP_VERIFIER_SECRET` and pass it to the `jewije` invocation; verify stub e2e still passes.

## 9. Docs + threat model

- [ ] 9.1 Create top-level `THREAT-MODEL.md`: same-box limitation, per-verifier secret deterrent, out-of-process V* requirement for true prevention, what the signature/log does and does not guarantee.
- [ ] 9.2 README: add a "Threat model" section linking to `THREAT-MODEL.md`; update the fail-closed guarantees list to mention signature verification + receipt log.
- [ ] 9.3 USAGE.md: document the new `VERIFIER_LOOP_VERIFIER_SECRET` env var (injected by jewilo at spawn; manual `jewije` invocation now requires it); note the BREAKING change for direct scripted invocations.
- [ ] 9.4 `AGENTS.md`: add a pointer to `THREAT-MODEL.md` and the new spec capabilities.

## 10. Full verification + coverage gate

- [ ] 10.1 `cargo test` → 0 failures, 0 ignored, 0 skipped across all suites (store, verdict, receipt, consensus, spawn_orchestrator, cli_e2e, wiring, smoke).
- [ ] 10.2 `cargo build --release` → clean for both binaries.
- [ ] 10.3 `cargo llvm-cov --summary-only` → TOTAL ≥80% lines AND every touched src file (`crypto/mod.rs`, `verdict/mod.rs`, `receipt/mod.rs`, `consensus/mod.rs`, `spawn/*`, `bin/*`) ≥80%.
- [ ] 10.4 `cargo audit` → no advisories on `ed25519-dalek` / `rand` / transitive deps.
- [ ] 10.5 jewilo self-verify: run `jewilo NEW` with stub_approve backend against this change's goal text; confirm the produced `completion.json` hash reflects the new inputs (signed verdicts + receipt head), and that manually editing a `verdict.json` field after registration causes the next consensus eval to reject with a signature failure.

## 11. PR + deploy

- [ ] 11.1 Feature branch `feat/verifier-tamper-hardening`; invoke pr-creation skill; fix-forward any reviewer findings; merge to main.
- [ ] 11.2 `git pull origin main`; `./scripts/install.sh` redeploy; final smoke confirms deployed binary exhibits signature verification + receipt log + fail-closed on missing secret.
