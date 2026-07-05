## 1. `approve --notes` — verdict layer (RED first)

- [ ] 1.1 RED test: `register_approve` with `Some("evidence")` writes `verdict.json` with `status: APPROVE` and `notes: "evidence"` (unsigned path). Assert via `read_verdict`.
- [ ] 1.2 RED test: `register_approve` with `None` writes APPROVE with no `notes` key (regression guard — serialized JSON omits the key).
- [ ] 1.3 RED test: `register_approve` with `Some("   ")` normalizes to `notes: None` (whitespace-only treated as absent).
- [ ] 1.4 RED test: `register_signed_approve` with `Some("signed evidence")` writes a record whose `signature` verifies over the canonical bytes including the notes (`verify_record` returns Ok against the pinned pubkey).
- [ ] 1.5 RED test: tampering the `notes` field of a signed APPROVE on disk → `verify_record` returns `BadSignature`.
- [ ] 1.6 GREEN: change `register_approve` signature to `(root, goal_id, verifier_id, round, notes: Option<&str>)`; build the record with `notes: notes.map(str::trim).filter(|s| !s.is_empty()).map(str::to_string)`. Update `register_signed_approve` the same way; `build_signed_record` already takes `notes: Option<&str>` — thread through.
- [ ] 1.7 Coverage gate: `cargo llvm-cov --fail-under-lines 80` for `src/verdict/mod.rs` lines touched.

## 2. `approve --notes` — CLI wiring (RED first)

- [ ] 2.1 RED test: `verifier-verdict approve --notes "foo"` (via `Cli::parse_from`) parses to `Cmd::Approve { notes: Some("foo") }` and exits 0 with "Verdict registered" printed (use a temp store + env-set identity).
- [ ] 2.2 RED test: `verifier-verdict approve -n "bar"` (short alias) parses identically.
- [ ] 2.3 RED test: `verifier-verdict approve` (no notes) still works — APPROVE written with `notes: None`.
- [ ] 2.4 GREEN: in `src/bin/verifier_verdict.rs`, change `Cmd::Approve` to `Approve { #[arg(long, short = 'n')] notes: Option<String> }`; thread `notes.as_deref()` into both approve match arms (`register_approve`, `register_signed_approve`).
- [ ] 2.5 Confirm `reject --notes` is unchanged (required, non-empty) — no test change beyond a regression guard that reject-without-notes still errors.

## 3. Verifier prompt — single policy (RED first)

- [ ] 3.1 RED test: `prompt::render(None, &vars)` (default round-1 template) contains the `<_unfold.md>` marker substring exactly once, and `# Verifier Detective Policy` exactly once.
- [ ] 3.2 RED test: `prompt::render_resume(None, &vars)` contains the policy marker exactly once.
- [ ] 3.3 RED test (regression): rendered round-1 prompt still contains `# Goal`, `{{goalText}}` substitution, `# Frozen artifact snapshot`, `# Your duty` sections — body structure preserved.
- [ ] 3.4 GREEN: edit `src/prompt/default_template.txt` — remove the inline `# Verifier Detective Policy` block and the duplicate "You ARE the Verifier now..." preamble; keep only: identity line, `# Goal`, `# Context`, `# Frozen artifact snapshot`, `# Your duty`. (The identity line `You are verifier {{verifierId}}...` is also produced by the `concat!` preamble — drop the duplicate from the .txt.)
- [ ] 3.5 GREEN: edit `src/prompt/default_resume_template.txt` the same way — keep `# Author fix notes`, `# Your own previous-round notes` (resume-only sections) plus goal/context/snapshot/duty.
- [ ] 3.6 GREEN: update the duty line wording so `approve / reject --notes "..."` reads unambiguously (approve-with-notes is now valid): e.g. `register your verdict via the verifier-verdict CLI (approve [--notes "..."] / reject --notes "...")`.
- [ ] 3.7 Update `default_template_consts_are_nonempty_and_embed_policy` test (`src/prompt/mod.rs`) to assert the policy appears exactly once in `DEFAULT_TEMPLATE` / `DEFAULT_RESUME_TEMPLATE` (was: asserts presence — now asserts exactly-once).
- [ ] 3.8 Coverage gate: `cargo llvm-cov --fail-under-lines 80` for `src/prompt/mod.rs`.

## 4. Integration + gates

- [ ] 4.1 `cargo fmt --check`.
- [ ] 4.2 `cargo clippy -- -D warnings`.
- [ ] 4.3 `cargo test` — full suite green.
- [ ] 4.4 `cargo llvm-cov --fail-under-lines 80` — repo-wide gate.
- [ ] 4.5 Manual smoke: spawn a verifier round against a fixture goal, confirm the spawned backend's `initial-prompt.txt` shows the policy exactly once and the goal/duty follow in the same message.
- [ ] 4.6 Update `README.md` / `USAGE.md` only if the `approve` arity is documented there (grep first).

## 5. TDD discipline

- RED tests in §1, §2, §3 authored FIRST by a fresh teammate against the spec deltas. GREEN by a DIFFERENT fresh teammate. No same-author RED+GREEN for a group. Coverage gate per new src file before the group is marked done.
