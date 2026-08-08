# Implementation Tasks

**TDD discipline (per AGENTS.md):** every group = RED test first (fresh author) → GREEN impl (different fresh author) → coverage gate `cargo llvm-cov --fail-under-lines 80`. Never same author for RED + GREEN.

## 1. Prompt bloat — fileEditTimes scoping (Group C, D1)

- [ ] 1.1 RED: test `capture_file_edit_times` returns entries only for changed files (`git status --porcelain`), not all tracked files. Fixture: a temp repo with 100 tracked files, 3 changed.
- [ ] 1.2 RED: test `fileEditTimesMaxChars` (default 8000) truncates the block with an indicator when exceeded.
- [ ] 1.3 GREEN: rewrite `capture_file_edit_times` (`src/prompt/mod.rs:346-369`) to use `git status --porcelain` instead of `git ls-files`; add `fileEditTimesMaxChars` config field + truncation mirroring `truncate_diff`.
- [ ] 1.4 GREEN: wire `fileEditTimesMaxChars` into config defaults (`src/store/` or config loader) and render path.
- [ ] 1.5 Verify coverage ≥80% on touched files; run `cargo llvm-cov --fail-under-lines 80`.

## 2. Prompt bloat — policy dedup (Group C, D2)

- [ ] 2.1 RED: test rendered prompt does NOT contain built-in `VERIFIER_POLICY` text when `verifierPromptFile` is set (override semantics).
- [ ] 2.2 RED: test rendered prompt contains built-in `VERIFIER_POLICY` exactly once when `verifierPromptFile` is null.
- [ ] 2.3 GREEN: in `src/prompt/mod.rs` render path, make `prepend_custom` and the embedded `DEFAULT_TEMPLATE` `VERIFIER_POLICY` mutually exclusive — skip embedded policy when custom file present.
- [ ] 2.4 Verify no regression to `default_template` null-path tests; coverage gate.

## 3. Prompt bloat — context cap + budget warning (Group C, D3, D4)

- [ ] 3.1 RED: test `--context` over `contextMaxChars` (default 20000) is truncated with indicator.
- [ ] 3.2 RED: test rendered prompt > `promptBudgetBytes` (default 50000) emits stderr warning with per-section breakdown; does NOT block spawn.
- [ ] 3.3 GREEN: add `contextMaxChars` config + cap in context render; add `promptBudgetBytes` config + post-render warning in `src/bin/verifier_loop.rs` (or render entrypoint).
- [ ] 3.4 Coverage gate.

## 4. Prompt bloat — regression validation

- [ ] 4.1 Integration: render a prompt against a fixture repo with 4,000 tracked files + 12 changed; assert total rendered size < 80KB (down from 686KB).
- [ ] 4.2 Integration: assert no duplicated policy section (grep count ≤1 for a policy marker).

## 5. Verdict enforcement — within-round nudge (Group B, D5)

- [ ] 5.1 RED: test `gather()` detects a child that exited with no verdict.json OR `status:null` AND `turnsUsed < maxTurn`; asserts a resume-nudge is issued on the same sid.
- [ ] 5.2 RED: test nudge loop respects `maxTurn - turnsUsed` ceiling; after exhaustion, slot stays null.
- [ ] 5.3 RED: test a slot with a non-null verdict is NOT nudged.
- [ ] 5.4 GREEN: add verdict-enforcement loop in `src/spawn/orchestrator.rs` after `child.wait()`; build minimal verdict-nudge prompt constant; reuse existing resume-spawn path with the nudge.
- [ ] 5.5 GREEN: record nudge attempts in `meta.json` (`nudgeAttempts: N`).
- [ ] 5.6 Coverage gate.

## 6. Strengthened prompt template (Group B, D7)

- [ ] 6.1 RED: test default template rendered output contains a fenced bash block with `verifier-verdict approve --notes` and `verifier-verdict reject --notes`.
- [ ] 6.2 GREEN: edit `src/prompt/default_template.txt` and `default_resume_template.txt` to end with explicit fenced verdict-command block.
- [ ] 6.3 Coverage gate.

## 7. Compaction detection (new capability, D6)

- [ ] 7.1 RED: test ACP parser (`src/acp/parser.rs`) detects `{"type":"compaction","tokensBefore":N}` and records `compactionObserved: true` + token counts in `meta.json`.
- [ ] 7.2 RED: test a session with no compaction event records `compactionObserved: false`.
- [ ] 7.3 GREEN: extend `extract_final_output` / stream parser to capture compaction events; surface via parser return type or callback; persist to `meta.json`.
- [ ] 7.4 Coverage gate.

## 8. Compaction recovery — auto-resume (new capability, D6)

- [ ] 8.1 RED: test a session that emits compaction then exits with no `agent_end` and no verdict triggers exactly ONE recovery resume on the same sid.
- [ ] 8.2 RED: test recovery resume uses a minimal nudge prompt (<2KB) that does NOT re-embed goal/diff/policy.
- [ ] 8.3 RED: test a second compaction+exit after recovery leaves the slot null (no infinite loop, fail-closed).
- [ ] 8.4 RED: test compaction followed by a successful `agent_end` does NOT trigger recovery (session self-recovered).
- [ ] 8.5 GREEN: add compaction-recovery branch in `src/spawn/orchestrator.rs` (after verdict-enforcement check, before declaring null); build minimal `<2KB` nudge prompt constant; hard cap 1 recovery per slot per round.
- [ ] 8.6 Coverage gate.

## 9. Integration + end-to-end

- [ ] 9.1 E2E: simulate a verifier session JSONL with compaction event + no agent_end; assert orchestrator performs recovery resume and harvests a verdict written by the resumed session.
- [ ] 9.2 E2E: run `jewilo NEW` against a large fixture repo; assert rendered prompt < budget and no silent null verdict from compaction.
- [ ] 9.3 Full suite: `cargo test` + `cargo llvm-cov --fail-under-lines 80` green.

## 10. Issue hygiene

- [ ] 10.1 Close gh issues #1, #4 with ref to `fix-spawn-argv-overflow` (already fixed — Group A verification).
- [ ] 10.2 On merge: close #10, #12, #13, #21, #22, #31 (Group C), #11, #14, #17, #20, #23, #24, #25, #26, #27, #32 (Group B), noting Group D (#8, #9, #16, #30) as not-bug feature gaps tracked separately.

## 11. Added post-plan: per-verifier signing secret persistence (fix-secret)

The original task plan (§1–§10) did not include the per-verifier signing-secret work that
landed alongside D5/D6. It is documented here for completeness and audit.

- **RED:** `tests/compaction_recovery.rs::nudge_resume_can_register_signed_verdict` —
  asserts a nudge-harvested verdict is a SIGNED APPROVE (non-empty `signature` +
  `pubkeyId`) bound to the slot's pinned pubkey, and that `verifier-secret.hex` exists
  with mode 0600.
- **GREEN:** `src/verdict/mod.rs` gains `SECRET_FILE = "verifier-secret.hex"`,
  `mint_and_pin_pubkey` persists the signing key hex (mode 0600, first-write-wins,
  atomic temp+rename alongside the pubkey pin), and `read_verifier_secret` reads it
  back. `src/spawn/orchestrator.rs::spawn_nudge_child` reads the persisted secret and
  re-injects it into the resume child env so nudge/recovery children can sign.
- **Coverage:** the secret-persistence + read paths are covered by unit tests in
  `src/verdict/mod.rs` (`mint_atomic_writes_both_files_or_neither`,
  `read_verifier_secret_returns_none_when_absent`,
  `read_verifier_secret_surfaces_permission_denied_error`) plus the integration test
  above. Atomic-persistence hardening (F1) and `fs::metadata`-vs-`exists()` (F2) were
  added in the cubic-dev PR review pass.
