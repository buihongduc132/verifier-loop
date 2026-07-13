# Intention: jewilo STATS + AUDIT subcommands (run introspection + completion audit)

Date: 2026-07-14
Branch: `feat/stats-audit-subcmd`
Worktree: `.worktrees/wt-stats-audit` (off `origin/main` @ b21b7a5, post-PR #46)

## Goal

Two new `jewilo` subcommands so the outer driving process (and an auditor) can read
run statistics and verify that a final completion hash truly matches the requirement.

## a. `STATS <goalId>` — surface all stored JSON for a run

Other agents / operators sometimes need to read the stats / status / duration about a
given run without spelunking through `~/.verifier-loop/`. `STATS` aggregates everything we
currently store as JSON for a goal into one machine-readable JSON object to stdout:

- **Goal record** (`goal.json`) — id, text, context, createdAt.
- **Creation-time config snapshot** (`goal.json` → `config`) — n, m, maxTurn, backend, …
- **State** (`state.json`) — current round.
- **Per-round data** — for every round: verdict status per verifier, reject notes, null
  markers, fix notes, completion status.
- **Completion** (`completion.json`) — hash, fullDigest, matchedAt, matchingVerdicts.
- **Health** (`health.jsonl`) — count of unhealthy events in the last hour, cooldown flag.
- **Durations** — `createdAt` of the goal, `matchedAt` of the completion (if present), and
  the derived wall-clock duration between them.

Read-only, no goal lock taken (a stats probe must never block a round).

## b. `AUDIT <goalId>` — verify the final completion TRULY matches the requirement

At the time `jewilo` creates a goal, it snapshots the configuration (n, m, maxTurn, …)
into `goal.json` → `config`. This is the **requirement** the run must satisfy. `AUDIT`
re-checks the final completion against that creation-time snapshot:

- Reads the creation-time `config` (n/m) from `goal.json`.
- Reads `completion.json` matching verdicts.
- Verifies the completion TRULY matches: the number of matching APPROVE verdicts is
  `>= n` out of `m`, AND every matching verdict was registered under the pinned pubkey
  regime (signature verification), AND the recomputed completion hash matches the stored
  `fullDigest`.
- Prints a JSON report: `{ "valid": bool, "requiredN": .., "requiredM": ..,
  "matchingVerdicts": .., "hashRecomputed": .., "hashStored": .., "checks": [...] }`.
- Exit 0 if valid, non-zero otherwise.

This is the post-hoc tamper/audit check: "do these 2/X verdicts truly match the
requirement recorded at creation time?"

## Config snapshot at creation (already present)

`goal::GoalRecord` already carries `config: store::Config`, written into `goal.json` at
`NEW`. This intention keeps that as the single source of truth for the creation-time
requirement (no separate `config-snapshot.json` file — `goal.json` IS the snapshot).

## TDD discipline + ceremony

- Strict RED-then-GREEN; coverage gate `>=80%` lines.
- jewilo-dev deploy ceremony (per AGENTS.md): build WIP as `jewilo-dev`, run stable
  `jewilo`, run `jewilo-dev`, sync main, deploy stable.

## Out of scope

- Real-time streaming stats (polling). STATS is a point-in-time read.
- Re-running verifiers. AUDIT is read-only verification, never re-spawns.
