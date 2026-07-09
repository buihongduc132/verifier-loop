# Findings — `.jewilo-*` CWD bloat in openspec-dashboard

**Date:** 2026-07-09
**Source repo:** `/home/bhd/Documents/Projects/bhd/openspec-dashboard` (working tree, all 36 files UNTRACKED — `git status` `??`)
**Affected files:** 3 wrapper scripts + 33 generated artifacts = 36 total

## TL;DR

The `verifier-loop` / `jewilo` Rust CLIs do **NOT** produce these files (confirmed:
zero matches for `jewilo|fixnotes|scout-|final-rejection|goal.txt` across all `*.rs`).
The CLIs write only structured paths into their `store_dir` (`~/.verifier-loop/`):
`rounds/<round>/<verifierId>/{verdict.json, meta.json, final-output.txt}`,
`completion.json`, `receipt.jsonl`, signed goal record.

The bloat is produced by the **outer driving agent** (a pi session running a UX/UI
verifier-loop against openspec-dashboard), which hand-wrote 3 thin wrapper scripts in
CWD and dropped every intermediate rejection / scout / fixnotes artifact flat into CWD.

## What the 3 wrapper scripts do

| Script | Purpose | Cheat risk |
|--------|---------|------------|
| `.jewilo-new.sh` | `exec jewilo NEW "$(cat $GOAL_FILE)" [--context $CTX]` after activating mise. Pure quoting-safe wrapper. | NONE |
| `.jewilo-resume.sh` | `exec jewilo RESUME $GOAL_ID --fix "$(cat $FIX_NOTES_FILE)"`. Quoting-safe wrapper. | NONE |
| `.jewilo-run-ux-r1.sh` | Same as `-new.sh` but without `exec` (older copy). | NONE |

All three: `set -euo pipefail`, `cd` into the repo, activate mise, read a goal/fixnotes
file, pass contents to `jewilo`. **No env injection, no verdict writing, no store
manipulation, no `VERIFIER_LOOP_*` override.** They cannot influence consensus.

## What the 33 generated artifacts capture

| Pattern | Count | Contents | Source |
|---------|-------|----------|--------|
| `.jewilo-*-goal*.txt` | 5 (ux-goal, ux-goal-v2, ux-goal-v3, bug-goal) | Goal text fed to `jewilo NEW` | driving agent |
| `.jewilo-*-r<N>-fixnotes.txt` | 13 | Fix summaries per round; what was fixed, commits, done-criteria | driving agent (fixer) |
| `.jewilo-*-scout-*.md` | 9 | Defect-class enumeration scans (same-class / related-class): `rg` + headless-browser audit, file:line evidence | scout sub-agent |
| `.jewilo-*-final-rejection.md` / `-consolidated-rejection.md` | 6 | Verifier REJECT verdicts: deduped defect lists, severity, fix instructions, done-criteria | verifier sub-agents |

## Cheat / tamper assessment: **NONE**

Evidence the loop is behaving fail-closed and honestly:
1. **Rejections are real REJECTs.** The consolidated + final rejections document genuine
   defects (kanban status/checked divergence, scenario parser regex miss, a11y label +
   target-size failures, silent mutation failures, internal-path leaks). They REJECT, not
   rubber-stamp — the opposite of cheating.
2. **Fail-closed intact.** r5-fixnotes notes "v1 timed out null" → null verdict → no
   consensus → no hash. Exactly the spec invariant (`NULL verdict never → APPROVE`).
3. **Rigorous, not lenient.** r5-fixnotes explicitly switched from jsdom axe (false
   "0 violations") to LIVE Chrome CDP, and **called out its own false positives** (O2
   hero white-on-white, O3 schema-editor CSS bleed) as NOT real → not fixed. A cheating
   agent would have silently claimed credit.
4. **No store/verdict mutation** in any script. The only writes the CLIs perform go to
   `store_dir` under first-write-wins pre-created slots (D9). The wrapper scripts never
   touch the store.

**Verdict:** hygiene problem (CWD litter), NOT a tamper / cheat vector.

## Root cause of the bloat

The driving agent treats CWD as its scratchpad instead of writing into `flow/` or a
`.tmp/` dir. Naming convention `.jewilo-<task>-r<N>-<kind>.<ext>` is the agent's own
round-tagging, not anything the CLI emits. The CLIs give no help managing these.

## Recommended hygiene (CA — not yet implemented)

- Driving agent should write loop artifacts under `flow/artifacts/jewilo-<goalId>/` or a
  `.jewilo-tmp/` dir, never flat in repo root.
- Add `.jewilo-*` to openspec-dashboard `.gitignore` as a safety net (currently nothing
  ignores them — they show as untracked `??`).
- Do NOT strip this behavior from the `jewilo` binary — it is not the binary's fault
  (rule: fix root cause, never strip).

## Action taken this session

- Removed all 36 `.jewilo-*` files from openspec-dashboard working tree.
- AGENTS.md (openspec-dashboard) updated with 1-line reference to this finding.
- `verifier-loop` repo AGENTS.md: standing reminder added (these are NOT produced by the
  CLI — do not chase the binary when they reappear).
