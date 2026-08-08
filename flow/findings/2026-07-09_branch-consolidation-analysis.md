# Findings — Branch Consolidation Analysis (verifier-loop repo)

**Date:** 2026-07-09
**Base branch:** `main` (HEAD `dc8dbc9`)
**Merge-base with consolidate:** `3d12392` (tamper-hardening squash)

## TL;DR

5 remote branches assessed. **4 are dead** (work already on main via squash-merge PRs).
**1 is live** (`consolidate/all-work`) with genuine unmerged work. No stacking needed —
single-branch merge. Clean merge (0 conflicts per `git merge-tree`). Execution blocked by
dirty working tree (partial/older version of prompt-diff work applied locally).

## Branch verdicts

| Branch | Ahead | Status | Evidence |
|--------|-------|--------|----------|
| `feat/verifier-tamper-hardening` | 17 | **DEAD** | Squash-merged to main via PR #5 (`3d12392`). `merge-base --is-ancestor` confirmed. Only diff vs main: `.hindsight.json` (main added it in `dc8dbc9`). |
| `feat/cwd-runtime-source` | 3 | **DEAD** | Squash-merged to main via PR #3 (`b38dec8`). `deny_unknown_fields` + cwd runtime-derived fully on main. `config.rs` on main is a superset (has this + verifierPromptFile). |
| `feat/verifier-prompt-file` | 5 | **DEAD** | Squash-merged to main via PR #2 (`543b696`). `src/prompt/mod.rs` byte-identical to main. Proof hash `070426-94c3bc31` + artifacts on main. |
| `fix/approve-notes-clean` | 8 | **DEAD** | Absorbed into `consolidate/all-work` via PR #6 (`59c020a`). 0 unique commits vs consolidate. |
| `consolidate/all-work` | 12 | **LIVE** | Contains 2 openspec changes + 3 fix groups NOT on main. Clean merge to main. |

## consolidate/all-work — unique content (NOT on main)

### New files (git diff --diff-filter=A main..consolidate)
- `src/spawn/tempfile.rs` — stdin transport to fix E2BIG argv overflow
- `tests/spawn_stdin_transport.rs` (370 lines) — stdin transport tests
- `tests/spawn_goal_file_transport.rs` (460 lines) — goal-file transport tests
- `openspec/changes/fix-approve-notes-and-prompt-merge/` — full openspec change (proposal + design + 2 specs + tasks)
- `openspec/changes/fix-spawn-argv-overflow/` — full openspec change (proposal + design + 2 specs + tasks)
- `flow/findings/2026-07-05-pi-stdin-prompt.md` — finding doc

### Modified files vs main (32 total, 3142 insertions, 249 deletions)
- `src/verdict/mod.rs` — approve `--notes` optional + `normalize_optional_notes` refactor
- `src/prompt/mod.rs` — `git diff HEAD` capture (staged + unstaged) + fresh-repo fallback
- `src/spawn/orchestrator.rs` — stdin transport wiring (72 transport refs vs main's 7)
- `src/bin/verifier_loop.rs` + `src/bin/verifier_verdict.rs` — CLI wiring for approve --notes
- `tests/{verdict,wiring,prompt,consensus,acp_parser,spawn_orchestrator}.rs` — RED+GREEN tests

### Completion estimate: ~85-90%
- **fix-approve-notes-and-prompt-merge**: RED + GREEN + refactor + openspec change → ~95% (has spec, design, tests, implementation; proof hash not in a standalone commit)
- **fix-spawn-argv-overflow**: tempfile.rs + orchestrator + 830 lines transport tests + openspec change → ~90% (critical bug fix, well-tested)
- **prompt-diff capture**: test + implementation + fresh-repo fallback → ~85% (no standalone openspec change; folded into spawn-argv-overflow)

All three follow TDD RED-then-GREEN discipline per commit messages. Coverage report
exists at `flow/proof/coverage_report.txt`. Merge-tree confirms 0 conflicts.

## Merge-planner analysis

**Stacking strategy:** NONE needed. Only 1 branch (`consolidate/all-work`) has unique work.
The 4 dead branches should be deleted, not stacked.

**Merge plan:**
1. `consolidate/all-work` → `main` (clean merge, 0 conflicts per `git merge-tree --write-tree`)
2. Single PR, single merge — no stacking
3. After merge, delete all 5 remote branches (4 dead + 1 merged)

**Conflict risk:** LOW. `git merge-tree --write-tree main origin/consolidate/all-work` = clean tree `0b6a87e`.

## Execution blocker (CRITICAL)

**Cannot execute the merge** — working tree is dirty with foreign changes that overlap
consolidate's prompt-diff work:

| File | WT state | Conflict with consolidate? |
|------|----------|---------------------------|
| `src/prompt/mod.rs` | Modified: partial/older `git diff HEAD` capture (simpler `--cached` fallback; consolidate has complete staged+unstaged version) | **YES** — WT has older version of same feature |
| `tests/prompt.rs` | Modified: staged-changes test (but missing consolidate's §3 policy-dedup tests) | **YES** — subset of consolidate's tests |
| `AGENTS.md` | Modified: jewilo-bloat reference added this session | NO (independent) |
| `.claude/skills/openspec-*/SKILL.md` (5 files) | Deleted | NO (unrelated cleanup) |

**Safety hook blocked `git stash`** (block-git-stash-mutations rule: no mutations in shared
work tree). The merge cannot proceed without resolving these changes first.

### Required human decision (BLOCKER)
1. **`AGENTS.md`** — my jewilo-bloat edit from earlier this session. Commit it independently or include in merge.
2. **`src/prompt/mod.rs` + `tests/prompt.rs`** — DISCARD working-tree version (consolidate has the complete/superset version). Run:
   `git checkout -- src/prompt/mod.rs tests/prompt.rs`
3. **`.claude/skills/*`** — decide: commit deletions or restore. Run to restore:
   `git checkout -- .claude/skills/`
4. After clean tree: `git merge origin/consolidate/all-work` then `git push origin main`.

## Dead branches — documentation (Step 9)

All 4 dead branches have 0% unique remaining work:

| Branch | Completion | Killed by | Recommendation |
|--------|-----------|-----------|----------------|
| `feat/verifier-tamper-hardening` | 100% (on main) | Squash PR #5 | Delete remote |
| `feat/cwd-runtime-source` | 100% (on main) | Squash PR #3 | Delete remote |
| `feat/verifier-prompt-file` | 100% (on main) | Squash PR #2 | Delete remote |
| `fix/approve-notes-clean` | 100% (in consolidate) | PR #6 merge into consolidate | Delete remote |
