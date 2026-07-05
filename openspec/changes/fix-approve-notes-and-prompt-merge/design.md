## Context

`verifier-verdict` (jewije) is the only path a V* uses to register a verdict. Two defects harm usability and correctness:

1. **`approve` arity.** `Cmd::Approve` is a unit variant (`src/bin/verifier_verdict.rs:46`). The verifier prompt's own duty block tells V* to use `approve / reject --notes "..."`, so verifiers reasonably call `approve --notes "evidence"`. Clap rejects the unknown flag → the verdict is never written → the slot stays null → no consensus. The verdict layer already supports optional notes on a record (`VerdictRecord.notes: Option<String>`); only the CLI omits it for approve.

2. **Prompt policy duplication.** `src/prompt/mod.rs:88` defines `DEFAULT_TEMPLATE` as `concat!("You are verifier ...", "# Verifier Detective Policy (canonical...)", include_str!("verifier_policy.txt"), "---", include_str!("default_template.txt"))`. But `default_template.txt` *also* opens with a full `# Verifier Detective Policy` block (the same detective bullets, paraphrased). The composed prompt therefore carries the policy text twice. The spawned backend (pi, via `pi --mode json` stdin transport) renders this as a leading standalone policy block, a `---` separator, then a second policy block + goal — which both humans and the verifier read as two prompts. The user observed the `<_unfold.md>` investigation block appearing ahead of the actual review duty.

Constraints:
- First-write-wins, fail-closed, and signature semantics must be preserved (D4/D9).
- The canonical signed bytes for a verdict record already bind `notes: Option<&str>` via `crypto::canonical_record_bytes`; signed-approve currently always passes `None`. Adding notes to approve flows through the existing canonical-bytes path — no crypto change.
- `VERIFIER_POLICY` const and `verifier_policy.txt` are the canonical policy source (LD10). The template body files are auxiliary structure (identity, goal, snapshot, duty).

## Goals / Non-Goals

**Goals:**
- `verifier-verdict approve` accepts an optional `--notes` (with `-n` alias) and stores trimmed notes on the verdict when supplied. Works on both unsigned and signed paths.
- The rendered round-1 and resume verifier prompts contain the verifier policy exactly once.
- Zero change to consensus, completion-proof, receipt-log, or signature verification beyond threading notes through existing canonical bytes.

**Non-Goals:**
- Changing `reject --notes` (still required, non-empty).
- Changing the policy text itself (`verifier_policy.txt`).
- Adding a new CLI subcommand or new env var.
- Changing adapter templates, transports, or spawn orchestration.
- Backfilling notes onto historical verdict records (immutable — first-write-wins).

## Decisions

### D1 — `approve --notes` is optional, stored verbatim (trimmed)

`Cmd::Approve` becomes `Approve { #[arg(long, short = 'n')] notes: Option<String> }`. Reject stays required. In `run()`, the approve match arms pass `notes.as_deref().map(str::trim)` into `register_approve` / `register_signed_approve`.

**Why optional, not required:** approve with no notes is the documented happy path (`verifier-verdict approve`). Forcing notes would break existing verifiers and the spec scenario "Approve writes a verdict". Reject keeps `--notes` required because a REJECT without a reason is unactionable (D9-fail-closed for the author's fix loop).

**Why trimmed:** matches `register_reject`'s existing `notes.trim()` behavior. Avoids whitespace-only notes polluting the record and the receipt log.

**Alternatives considered:**
- *Reject `--notes` on approve entirely (status quo).* Rejected — it breaks the documented `approve / reject --notes "..."` call shape and silently drops the verdict.
- *Add a separate `approve-with-notes` subcommand.* Rejected — fragments the CLI surface; `--notes` is the obvious flag.

### D2 — `register_approve` / `register_signed_approve` take `notes: Option<&str>`

New signatures:
```rust
pub fn register_approve(root, goal_id, verifier_id, round, notes: Option<&str>) -> Result<...>
pub fn register_signed_approve(root, goal_id, verifier_id, round, notes: Option<&str>, secret) -> Result<...>
```

Inside, build the `VerdictRecord` with `notes: notes.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())` — so an empty/whitespace `--notes ""` is normalized to `None` (no key in the JSON), matching the existing `skip_serializing_if = "Option::is_none"` on the record. `build_signed_record` already takes `notes: Option<&str>` and passes it into `crypto::canonical_record_bytes` — no signature change.

**Why filter empty to None:** preserves byte-stable serialization for existing approve records (no new `"notes": null` key) and keeps the receipt log's approve entries uniform.

### D3 — Strip the inline policy block from the template body files

`default_template.txt` and `default_resume_template.txt` lose their leading `# Verifier Detective Policy` section. They retain: the identity line, `# Goal` / `# Context` / `# Author fix notes` (resume only) / `# Frozen artifact snapshot` / `# Your duty`. The `concat!` in `DEFAULT_TEMPLATE` / `DEFAULT_RESUME_TEMPLATE` is the sole composer of policy + body.

**Why edit the .txt and not the `concat!`:** the `concat!` is the right composition point (compile-time, single source = `verifier_policy.txt`). Editing the .txt keeps the policy's source-of-truth in one file and the template structure in another — each file does one thing.

**Alternatives considered:**
- *Drop the `concat!` and keep the inline policy.* Rejected — the canonical policy lives in `verifier_policy.txt` (LD10); inlining it into the body files would fork the text and drift again.
- *Add a runtime dedup pass.* Rejected — over-engineering; the duplication is a build-time mistake, not a runtime concern.

### D4 — Receipt log: approve entries stay notes-free

`append_receipt_for_signed_write` is called with the same arguments as today (`kind="approve"`, `status="APPROVE"`, `signed_by=pubkey_id`). The receipt log records identity + status + signer, not notes — so approve-with-notes does not change the receipt payload. Notes remain only on `verdict.json`. This keeps the hash chain inputs unchanged.

**Why not add notes to the receipt:** the receipt's job is tamper-evidence of *when/who/what-status*, not evidence content. Verdict content tamper-evidence is the signed `verdict.json` itself. Adding notes to the receipt would change the chain's canonical inputs and require a spec delta on `receipt-log` — out of scope.

## Risks / Trade-offs

- **[Risk] Signature over approve-with-notes differs from approve-without-notes.** → Mitigation: this is correct and intended — the canonical bytes bind notes, so a tampered-notes attack invalidates the signature. `verify_record` already canonicalizes `notes.as_deref()`, so verification works without code change. Documented in the spec delta.
- **[Risk] A verifier passes `--notes ""` expecting it to be stored.** → Mitigation: D2 normalizes empty/whitespace to `None`. The CLI exits 0 ("Verdict registered") either way — the verdict is still APPROVE. Documented.
- **[Risk] Editing `default_template.txt` breaks existing prompt tests that assert the inline policy substring.** → Mitigation: the affected tests are updated in the same change to assert the policy appears exactly once (via the `concat!`), not zero times. The `default_template_consts_are_nonempty_and_embed_policy` test in `src/prompt/mod.rs:509` is the canary.
- **[Risk] Changing `register_approve` signature breaks other call sites.** → Mitigation: TDD — the only production caller is `src/bin/verifier_verdict.rs`; tests are updated in the same commit. `grep` confirms no other caller.
- **[Trade-off] `approve` with notes is no longer byte-identical to legacy approve records.** → Acceptable: the `skip_serializing_if = "Option::is_none"` ensures approve-without-notes serializes identically to today.

## Migration Plan

1. Implement D2 (verdict layer signatures) + D1 (CLI flag) atomically — the CLI is the only caller.
2. Implement D3 (template body edits) independently.
3. Update affected tests (see proposal Impact).
4. `cargo fmt --check && cargo clippy -- -D warnings && cargo llvm-cov --fail-under-lines 80`.
5. No on-disk migration: existing verdict records are immutable (first-write-wins); they keep `notes: None`. New approve-with-notes verdicts simply carry the optional key.

**Rollback:** revert the commit. Existing verdicts are unaffected. The CLI returns to rejecting `--notes` on approve. No data migration needed.

## Open Questions

- Should the verifier prompt's duty line be updated to mention `approve --notes "..."` as the preferred shape (currently says `approve / reject --notes "..."` which is ambiguous about whether notes apply to approve)? → Yes, minor wording tweak in `default_template.txt` duty block, folded into D3. No spec implication.
