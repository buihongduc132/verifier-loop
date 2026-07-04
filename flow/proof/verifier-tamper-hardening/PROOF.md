# Self-Verify Proof — add-verifier-tamper-hardening

**Date:** 2026-07-04
**Hash:** `070426-5b6efc0f` (mmddyy-XXXXXXXX format, produced by jewilo)
**goalId:** `b211a531-2e50-44d2-8e71-e0ae7c42805b`

## How produced

```bash
VERIFIER_LOOP_HOME=/tmp/jewilo-selfverify \
VERIFIER_LOOP_BACKEND_CMD=scripts/stub_approve.sh \
jewilo NEW "<this goal's objective text>"
```

Output: `070426-5b6efc0f`

The jewilo binary used is the one JUST hardened (rebuilt from this branch). It:
- minted+signed per-verifier keypairs at spawn,
- injected VERIFIER_LOOP_VERIFIER_SECRET into V* env,
- the stub backend forwarded the secret to jewije which took the signed path,
- the signed APPROVE verdicts were verified against pinned pubkeys at consensus,
- a hash-chained receipt entry was appended per APPROVE,
- the receipt head was folded into the completion-hash inputs.

## Artifacts

- `completion.json` — hash `070426-5b6efc0f`, fullDigest, matchingVerdicts (v1, v2).
- `receipt-log.jsonl` — 2 chained entries: seq=1 prevHash='' → seq=2 prevHash=<seq1.entryHash>.
- `v1-verdict.json` — APPROVE + signature (128 hex) + pubkeyId (16 hex).
- `v1-verifier-pubkey.json` — pinned pubkey (64 hex) + mintedAt.
- `v2-verdict.json` — APPROVE + signature + pubkeyId.

## Tamper detection proof

Edited `v1/verdict.json` `registeredAt` after registration (no re-sign), then called
`consensus::evaluate` directly (no re-spawn):

```
passed: false
approve_count: 1  (tampered v1 no longer counts)
signature_failures: [("v1", "BadSignature: signature verification failed (signature does not verify over the canonical record bytes)")]
TAMPER DETECTION CONFIRMED: v1 signature failed, not counted toward n/m
```

This demonstrates the in-flight verdict edit attack path is closed: a post-registration
edit to status/notes/registeredAt invalidates the signature, and consensus treats the
verdict as untrusted (fail-closed).

## Coverage + audit

- `cargo llvm-cov --summary-only` → TOTAL 92.77% lines; all 7 touched src files ≥85%.
- `cargo audit` → exit 0, no advisories (130 deps).
- `cargo test` → 0 failures across 18 binaries (242 tests).

## Same-box honesty (per THREAT-MODEL.md)

This self-verify uses stub_approve (deterministic, same-box). It proves the CODE is
correct (signing, verification, receipt chaining, hash folding, tamper detection). It
does NOT prove same-box forgery is impossible — a fully-compromised A that reads V*'s
env can still forge. True prevention requires out-of-process V* on a separate host.
