# Merge Plan — Consolidate buihongduc branches → main

**Date:** 2026-07-09
**Repo:** `/home/bhd/Documents/Projects/bhd/verifier-loop` (base = `main` @ `dc8dbc9`)

## Branch landscape (5 remote branches)

| Branch | Ahead | Behind | Verdict | Evidence |
|--------|-------|--------|---------|----------|
| `feat/verifier-tamper-hardening` | 17 | 2 | **DEAD** — squash-merged PR #5 | main has `3d12392`; config.rs identical; only diff = `.hindsight.json` |
| `feat/cwd-runtime-source` | 3 | 3 | **DEAD** — squash-merged PR #3 | main has `b38dec8`; `config.rs` byte-identical; `deny_unknown_fields` on main L36 |
| `feat/verifier-prompt-file` | 5 | 4 | **DEAD** — squash-merged PR #2 | main has `543b696`; `prompt/mod.rs` byte-identical to branch |
| `fix/approve-notes-clean` | 8 | 1 | **ABSORBED** — merged into consolidate via PR #6 | 0 unique commits vs consolidate (`59c020a` merge) |
| `consolidate/all-work` | 12 | 1 | **LIVE — sole merge candidate** | 3 unique commits + absorbed approve-notes; only `.hindsight.json` behind |

## Stacking strategy

**Trivial — only 1 branch needs merging.** No stacking required.

```
main (dc8dbc9)
  ↑
  └── consolidate/all-work (62e5f47)  ← merge this
```

### What consolidate/all-work adds to main

**3 truly unique commits** (not on main, not squash-merged):

1. `d674050` — prompt: `git diff` → `git diff HEAD` (captures staged changes)
2. `35548bd` — spawn: tempfile sweep age threshold + leak-on-write-failure cleanup
3. `62e5f47` — prompt: fresh-repo fallback (captures unstaged diff when no commits)

**+ absorbed work** (via PR #6 merge `59c020a`, NOT on main):
- approve --notes optional (verifier-verdict CLI)
- verifier policy rendered exactly once (dedup)
- normalize_optional_notes returns Option<&str>
- RED tests for approve-notes + prompt dedup

**Merge dry-run:** 16 files added, 15 files merged, **0 conflicts** per `git merge-tree`.

## BLOCKER — dirty working tree

**Cannot execute merge yet.** Working tree has uncommitted changes:

| File | Change | Overlaps consolidate? |
|------|--------|-----------------------|
| `src/prompt/mod.rs` | MODIFIED | **YES** — same `@@ -243,7` region (WT has simpler `--cached` fallback; consolidate has complete dual staged+unstaged fallback + `head_exists()` helper) |
| `tests/prompt.rs` | MODIFIED | **YES** — same `@@ -246,6` region |
| `AGENTS.md` | MODIFIED | No (my jewilo-bloat reference) |
| `.claude/skills/*` | DELETED (5 files) | No |
| `flow/intentions/`, `flow/findings/` | untracked | No |
| `.agent/`, `.gemini/`, etc. | untracked | No |

**The WT changes to `prompt/mod.rs` are a SUBSET of what consolidate provides.** Consolidate has the more complete implementation (handles both staged AND unstaged on fresh repos). **Recommend: discard WT prompt changes, merge consolidate.**

## Execution steps (requires user/leader approval)

1. **Stash/discard WT prompt changes:**
   ```bash
   git checkout -- src/prompt/mod.rs tests/prompt.rs
   # (keep AGENTS.md + flow/ changes — they don't conflict)
   ```
2. **Merge consolidate/all-work:**
   ```bash
   git merge origin/consolidate/all-work
   # Expected: clean merge (0 conflicts per dry-run)
   ```
3. **Verify:**
   ```bash
   cargo test
   cargo llvm-cov --fail-under-lines 80
   ```
4. **Push:**
   ```bash
   git push origin main
   ```
5. **Clean up dead branches:**
   ```bash
   git push origin --delete feat/verifier-tamper-hardening feat/cwd-runtime-source feat/verifier-prompt-file fix/approve-notes-clean consolidate/all-work
   ```

## Final PR

Since `consolidate/all-work` can merge directly to main (no stacking), the "final PR" = direct merge. If a PR is still desired for audit trail:
```bash
gh pr create --base main --head consolidate/all-work --title "Consolidate all work: prompt diff + tempfile sweep + approve-notes"
```
