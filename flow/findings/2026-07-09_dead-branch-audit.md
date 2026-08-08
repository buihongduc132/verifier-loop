# Findings — Dead branch audit: 4 of 5 remote branches already on main

**Date:** 2026-07-09
**Repo:** `/home/bhd/Documents/Projects/bhd/verifier-loop` (base = `main` @ `dc8dbc9`)
**References:** merge plan `flow/plans/2026-07-09_consolidate-branches-merge-plan.md`

## TL;DR

Of 5 remote branches, **4 are dead** (fully absorbed into `main` via squash/merge PRs) and **1 is live** (`consolidate/all-work`). No stacking needed — only 1 branch requires merging.

## Dead branches (document, do NOT merge)

### 1. `feat/verifier-tamper-hardening` — DEAD (PR #5 squash)

- **17 commits** on branch, but main has `3d12392 feat: add-verifier-tamper-hardening (#5)` — the squash merge.
- `src/crypto/mod.rs`, `src/receipt/mod.rs`, signed verdicts, receipt log — ALL on main.
- Only diff vs main: `.hindsight.json` (branch lacks it; main has it via `dc8dbc9`).
- **Action:** delete remote branch.

### 2. `feat/cwd-runtime-source` — DEAD (PR #3 squash)

- **3 commits** on branch, but main has `b38dec8 feat(config): closed schema (#3)` — the squash merge.
- `src/store/config.rs` is **byte-identical** between branch and main (verified via `diff`).
- `deny_unknown_fields` present on main at line 36. All 5 config tests + 2 e2e tests on main.
- The -4163 line diffstat is a mirage — it shows what main has that the branch LACKS (tamper-hardening, added after this branch forked).
- **Action:** delete remote branch.

### 3. `feat/verifier-prompt-file` — DEAD (PR #2 squash)

- **5 commits** on branch, but main has `543b696 feat(config): verifierPromptFile override + minGoalChars (#2)` — the squash merge.
- `src/prompt/mod.rs` is **byte-identical** between branch and main (verified via `diff`).
- The PR-review fix (`78e5897` — newline normalization + config reuse in run_resume) was included in the squash (content parity confirmed).
- Branch lacks `deny_unknown_fields` from PR #3 (forked before it) — but that's main being ahead, not branch having unique work.
- **Action:** delete remote branch.

### 4. `fix/approve-notes-clean` — ABSORBED (PR #6 merge into consolidate)

- **8 commits** on branch, all absorbed into `consolidate/all-work` via merge commit `59c020a`.
- 0 unique commits vs consolidate (`git log origin/fix/approve-notes-clean..origin/consolidate/all-work` = empty).
- **Not** on main independently — only reachable via consolidate/all-work.
- **Action:** delete remote branch (work preserved in consolidate).

## Live branch

### `consolidate/all-work` — LIVE (sole merge candidate)

- **12 commits ahead** of main (3 unique + absorbed approve-notes via PR #6 merge).
- **1 commit behind** (`.hindsight.json` via `dc8dbc9`).
- **Unique work NOT on main:**
  1. Prompt diff capture: `git diff` → `git diff HEAD` (staged changes visible to verifiers)
  2. Tempfile sweep: age threshold + leak-on-write-failure cleanup
  3. Fresh-repo fallback: dual staged+unstaged capture when no commits exist
  4. Absorbed approve-notes: optional `--notes`, policy dedup, normalize_optional_notes
- **Merge dry-run:** 16 adds + 15 merges, 0 conflicts.
- **Action:** merge to main (see merge plan).

## Completion assessment

| Branch | Completion | Rationale |
|--------|-----------|-----------|
| `consolidate/all-work` | **>95%** | RED+GREEN+proof for all features; only cleanup remains |
| `feat/verifier-tamper-hardening` | N/A (dead) | Already on main |
| `feat/cwd-runtime-source` | N/A (dead) | Already on main |
| `feat/verifier-prompt-file` | N/A (dead) | Already on main |
| `fix/approve-notes-clean` | N/A (absorbed) | In consolidate |

No branches are in the 80-99% "complete first" range. No branches are <=80%.

## Cheat/tamper assessment: NONE

All branches follow clean TDD discipline (RED test → GREEN implementation → proof hash). The squash merges are legitimate GitHub PR merges (#2, #3, #5, #6). No signs of:
- Force-push history rewriting
- Verdict tampering
- Store manipulation
- Bypassed consensus

## Recommended cleanup

```bash
# Delete dead remote branches
git push origin --delete feat/verifier-tamper-hardening
git push origin --delete feat/cwd-runtime-source
git push origin --delete feat/verifier-prompt-file
git push origin --delete fix/approve-notes-clean
# After merge, delete consolidate too
git push origin --delete consolidate/all-work
```
