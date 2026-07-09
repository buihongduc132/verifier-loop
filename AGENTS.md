# AGENTS.md — verifier-loop repo

Single source of truth pointers for any agent (human or CLI) working in this repo.

## What this is

A Rust implementation of the `verifier-loop` skill's contract as two out-of-process CLIs
(`verifier-loop`/`jewilo` and `verifier-verdict`/`jewije`) that produce a tamper-evident
completion hash on n/m verifier consensus. See [`README.md`](README.md).

## Design source of truth (read FIRST)

- **Specs (behavioural contract):** [`openspec/changes/add-verifier-loop-cli/specs/`](openspec/changes/add-verifier-loop-cli/specs/)
  - `goal-lifecycle`, `verifier-spawn`, `verdict-registration`, `consensus-check`,
    `completion-proof`, `verifier-prompt`
- **Design decisions D0–D10 + risks:** [`openspec/changes/add-verifier-loop-cli/design.md`](openspec/changes/add-verifier-loop-cli/design.md)
- **Locked decisions LD1–LD27 + rationale:** [`flow/explore/2026-07-03-locked-decisions.yaml`](flow/explore/2026-07-03-locked-decisions.yaml), [`flow/explore/`](flow/explore/)
- **Language choice (why Rust):** [`flow/findings/2026-07-03-language-choice.md`](flow/findings/2026-07-03-language-choice.md)
- **Implementation roadmap:** [`openspec/changes/add-verifier-loop-cli/tasks.md`](openspec/changes/add-verifier-loop-cli/tasks.md) §1–§11
- **ACP sample fixtures:** [`flow/fixtures/`](flow/fixtures/)

## Module map

| `src/` module | tasks.md | spec |
|---------------|----------|------|
| `store/`   | §2 | goal-lifecycle (salt + config) |
| `goal/`    | §3 | goal-lifecycle (NEW/RESUME/immutability) |
| `acp/`     | §4 | verifier-spawn (ACP parser + adapters) |
| `spawn/`   | §5,§6 | verifier-spawn (orchestration + reuse) |
| `verdict/` | §7 | verdict-registration |
| `consensus/` | §8 | consensus-check + completion-proof |
| `prompt/`  | §9 | verifier-prompt |
| `cli/`     | §10 | wiring |

## TDD discipline (hard constraint)

Every feature group follows strict RED-then-GREEN:
1. A **fresh** teammate authors the failing (RED) test against the spec.
2. A **different fresh** teammate authors the minimal GREEN implementation.
3. Coverage gate `>=80%` lines per new src file before the group is marked done.

Never implement without a test first. Never have the same author write both RED and GREEN for a group.

## Coverage gate

```bash
cargo llvm-cov --fail-under-lines 80
```

## Fail-closed invariants (must always hold)

- NULL verdict never → APPROVE.
- Missing store → no hash.
- `goalText` edit → signature mismatch → hash mismatch.
- Verdict edit → hash mismatch.

## Security / threat model

The `add-verifier-tamper-hardening` change adds per-verifier Ed25519 signing keys
(`VERIFIER_LOOP_VERIFIER_SECRET`), pinned pubkeys, signed verdict records, and a
hash-chained receipt log. This is a **deterrent + detection layer**, not a prevention
guarantee: on a single host, a process with write access to `~/.verifier-loop/` AND the
ability to read a V\*'s env can still forge. True prevention requires out-of-process V\*
on a separate host. Read the full model before relying on the completion hash:
[`THREAT-MODEL.md`](THREAT-MODEL.md). (Specs: `openspec/changes/add-verifier-tamper-hardening/specs/`;
design + risks: `openspec/changes/add-verifier-tamper-hardening/design.md`.)

## `.jewilo-*` CWD bloat (standing reminder)

The `jewilo`/`jewije` CLIs do NOT produce `.jewilo-*` files in CWD — they write only to `store_dir` (`~/.verifier-loop/`). Any `.jewilo-*-goal.txt`, `-r<N>-fixnotes.txt`, `-scout-*.md`, `-final-rejection.md`, `-new.sh`, `-resume.sh` in a target repo are produced by the **outer driving agent**, not the binary. These are hygiene artifacts (legit verifier rejections, not tamper). Detail: [`flow/findings/2026-07-09_jewilo-bloat-files-openspec-dashboard.md`](flow/findings/2026-07-09_jewilo-bloat-files-openspec-dashboard.md). **ALWAYS remind the user about this bloat whenever `.jewilo-*` files reappear in any repo's working tree.**

## Branch consolidation (2026-07-09)

- 4 of 5 remote branches are DEAD (squash-merged via PRs #2, #3, #5, #6). Only `consolidate/all-work` has unique work. Audit + merge plan: [`flow/findings/2026-07-09_dead-branch-audit.md`](flow/findings/2026-07-09_dead-branch-audit.md), [`flow/plans/2026-07-09_consolidate-branches-merge-plan.md`](flow/plans/2026-07-09_consolidate-branches-merge-plan.md).

## Branch consolidation (2026-07-09)

5 remote branches assessed: 4 DEAD (squash-merged to main via PRs #2/#3/#5 + absorbed into consolidate), 1 LIVE (`consolidate/all-work`, ~85-90%, clean merge to main, blocked by dirty WT). Analysis + merge plan + dead-branch doc: [`flow/findings/2026-07-09_branch-consolidation-analysis.md`](flow/findings/2026-07-09_branch-consolidation-analysis.md).

## Out of scope (do NOT implement)

Deferred: OT1 audit subcommand, OT2 per-verifier maxTurn refresh, OT3 `chattr +a` hardening,
OT4 skill→wrapper, OT6 fan-out scouts. See design.md Non-Goals.
