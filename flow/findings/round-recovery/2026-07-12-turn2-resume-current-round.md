# Explore Turn 2 — resume the CURRENT round (not a new round)

> Date: 2026-07-12
> Phase: deep explore (explore mode)
> Status: opened design space → 5 open threads

## [user] (verbatim — trust chain L1)

> Enter explore mode. Think deeply. Visualize freely. Follow the conversation wherever it goes.
>
> **Provided arguments**: For these kill , usually jewilo have already done some progress ; how do we be able to quickly resume the CURRENT running round instead of another separate round?

## [assistant]

### The key insight first

There's a beautiful property hiding in this architecture that most people miss: **the verdict file IS the resumption contract.**

```
verdict.json   ← written by the CHILD (verifier-verdict CLI), independently of jewilo
                 first-write-wins: {status:null} pre-created → APPROVE/REJECT atomic
meta.json      ← written by jewilo's gather() AFTER child exits (SID + turnsUsed)
completion hash ← computed over verdicts + signature + round  (NOT meta.json)
```

So `meta.json` is **purely** for the "should I reuse this SID next round?" decision. It plays *zero* role in consensus or the completion hash. That means: jewilo's job at gather time is just to (a) wait for children, (b) reap meta, (c) read verdicts. If we separate "spawn" from "collect", resumption collapses into "collect again".

This is the whole game. Let me show why.

### The two kill scenarios behave very differently

```
SCENARIO A — Ctrl-C in a TTY (interactive)
──────────────────────────────────────────
  terminal sends SIGINT to whole foreground pgrp
  jewilo dies  AND  all child verifiers die (same pgrp, no setsid)

  disk state after kill, round N mid-gather:
  ┌─────────────────────────────────────────────┐
  │ rounds/N/v1/  verdict=APPROVE  meta=stale    │ ← child finished, wrote verdict, exited
  │ rounds/N/v2/  verdict=null     meta=missing  │ ← child killed before writing
  │ rounds/N/v3/  verdict=REJECT   meta=stale    │ ← child finished, wrote verdict, exited
  └─────────────────────────────────────────────┘
  processes: NONE alive
  → round is "half-finished" on disk. No live work to reattach to.


SCENARIO B — SIGTERM / kill (non-TTY, e.g. outer agent)
───────────────────────────────────────────────────────
  jewilo dies. kill_on_drop is a no-op (Drop doesn't run on signal).
  children are ORPHANED — reparented to init, still computing, still burning tokens.

  disk state:
  ┌─────────────────────────────────────────────┐
  │ rounds/N/v1/  verdict=APPROVE  meta=stale    │
  │ rounds/N/v2/  verdict=null     meta=missing  │ ← orphan pi STILL RUNNING
  │ rounds/N/v3/  verdict=REJECT   meta=stale    │
  └─────────────────────────────────────────────┘
  processes: v2's backend is alive, maybe seconds from writing its verdict
  → there IS live work, but jewilo can't reattach to pipes it didn't open
```

Scenario B is the nasty one. Even though there's live work, jewilo **cannot reattach** — the `tokio::process::Child` handle, the stdout pipe, the SID discovery all died with the old process. The only durable signal an orphan leaves is `verdict.json` appearing on disk.

### Why this is actually two questions in disguise

Your question — "resume the current round" — splits:

| | Scenario A (children dead) | Scenario B (children alive) |
|---|---|---|
| **What's salvageable** | only finished verdicts | finished verdicts + in-flight orphan work |
| **Can we reattach?** | nothing to reattach | NO (pipes are gone) |
| **Recovery primitive** | re-spawn null slots | wait for verdict.json to appear, else re-spawn |

The temptation in scenario B is "reattach to the orphan". I'd push back on that hard — it's a rabbit hole:

- Reattaching requires knowing the orphan's PID (jewilo would have to persist PIDs per slot before gathering)
- Even with the PID, you can't get the stdout pipe back — you'd have to `strace`/`/proc/<pid>/fd` hackery, which is Linux-specific and fragile
- The orphan doesn't know jewilo left; it'll happily write its verdict to disk and exit
- So the **simplest** "reattach" is just: *poll the verdict file*. The orphan writes it; a new jewilo instance reads it. The filesystem is the IPC channel.

That reframes everything. There's really only **one recovery primitive** that handles both scenarios:

> **"Collect-only" pass**: for round N, walk every slot, read verdict.json, and for any slot still null, decide whether to re-spawn or keep waiting.

### Visualizing the recovery flow

```
                      ┌──────────────────────────────┐
                      │  jewilo killed mid round N   │
                      └──────────────┬───────────────┘
                                     │
                                     ▼
                      ┌──────────────────────────────┐
                      │  user runs: RECOVER <goalId> │   ← new command, or overload RESUME
                      │            (no --fix)        │
                      └──────────────┬───────────────┘
                                     │
                                     ▼
                      ┌──────────────────────────────┐
                      │  round = current_round (N)   │   ← NOT incremented
                      │  no prompt re-render         │   ← same round, same snapshot
                      │  no fix-notes append         │
                      └──────────────┬───────────────┘
                                     │
                                     ▼
              ┌──────────────────────────────────────────┐
              │  for each slot v1..vN:                    │
              │    read verdict.json                      │
              │    ┌─ non-null ──────────▶ KEEP (done)    │
              │    └─ null ──────────────▶ needs work     │
              └──────────────────────┬───────────────────┘
                                     │
                    ┌────────────────┴────────────────┐
                    │                                 │
                    ▼                                 ▼
        ┌────────────────────┐          ┌─────────────────────────┐
        │ OPTION 1: POLL     │          │ OPTION 2: RE-SPAWN      │
        │ (cheap, B-friendly)│          │ (deterministic, A-safe) │
        │                    │          │                         │
        │ wait up to T for   │          │ spawn fresh child for   │
        │ verdict.json to    │          │ null slot, gather as    │
        │ appear from orphan │          │ normal (kills orphan if │
        │                    │          │ one exists)             │
        └─────────┬──────────┘          └──────────┬──────────────┘
                  │                                │
                  └────────────┬───────────────────┘
                               ▼
                ┌──────────────────────────────┐
                │  evaluate consensus on N     │
                │  PASS → hash → done          │
                │  FAIL → tell user to RESUME  │
                │         (real round N+1)     │
                └──────────────────────────────┘
```

The two options are the real design fork. Let me lay out the tradeoffs because this is where it gets interesting.

### The design fork: poll vs re-spawn

```
                    POLL (wait for orphan)            RE-SPAWN (fresh child)
                    ──────────────────────            ──────────────────────
handles scenario A  ✗ (nothing to wait for →          ✓ (deterministic)
                      times out, then re-spawns)

handles scenario B  ✓ (salvages the orphan's work)    ⚠ (orphans new child; duplicate
                                                          work; must kill orphan first)

tokens spent         minimal (the orphan finishes)     double (original orphan + new
                                                          child both run to completion)

latency              up to T (poll timeout)            fresh verifierTimeoutSec

determinism          low (orphan may never finish)     high

complexity           low (poll + read file)            medium (must detect+kill orphan)
```

My instinct: **re-spawn is the correct default, poll is the optimization.** Here's why — re-spawn is scenario-A-safe and deterministic; it fails loudly. Poll is a nice-to-have for scenario B when you know orphans exist and you want to avoid burning tokens. But poll alone is unsafe in scenario A (you'd poll forever against a dead child).

A hybrid suggests itself:

> On RECOVER, first **poll briefly** (e.g. `min(verifierTimeoutSec, 60s)`) for null slots' verdicts to appear. Whatever's still null after that → re-spawn. This salvages orphan work when it exists, and degrades to clean re-spawn when it doesn't.

But — and here's where I'd slow down — there's a hazard in the poll path.

### The orphan-collision hazard

If you poll AND an orphan is still running when you give up and re-spawn, you now have **two verifiers writing to the same slot**. The verdict layer's first-write-wins saves you from corruption, but:

```
  orphan v2 (slow) ───...───▶ writes APPROVE at t=120s
  new v2 (fast)  ──▶ writes REJECT  at t=45s   ← this wins (first-write)
  orphan's APPROVE ──▶ AlreadyFinal, discarded
```

So the **re-spawned** child's verdict wins deterministically (it's faster), and the orphan's work is silently discarded. That's... actually fine for correctness (one signed verdict per slot, first-write-wins is the contract). But it means you paid for the orphan's tokens AND the re-spawn's tokens. The poll timeout needs to be short enough that this collision window is small, or you need to **kill the orphan before re-spawning**.

Killing the orphan requires knowing its PID. Which means jewilo would need to persist PIDs before gathering. Which is... actually cheap:

```
rounds/N/v2/pid     ← written at spawn time, before gather
                      on RECOVER: if pid alive && verdict null → kill before re-spawn
```

A 4-byte file. That's the missing piece for clean orphan handling.

### What changes in the codebase

Let me map this to the actual modules, because the shape is surprisingly small.

```
CURRENT                                  PROPOSED (RECOVER)
─────────                                ──────────────────
goal::resume()                           goal::recover()  ← NEW
  - increments round                       - does NOT increment round
  - appends fix-notes                      - does NOT append fix-notes
  - returns new round N+1                  - returns current round N

spawn::spawn_resume()                     spawn::spawn_recover()  ← NEW
  - reads prev_round (N-1) meta            - reads CURRENT round (N) verdicts
  - spawns all slots                       - spawns ONLY null slots
  - archives exhausted SIDs                - kills orphan by pid if present
                                           - reuses current-round SID if meta has one
                                           - (degenerate case: all null = re-spawn all)

cli: RESUME <id> [--fix]                  cli: RECOVER <id>  ← NEW
  - always new round                       - same round, collect-only semantics
```

The elegant part: **`consensus::evaluate` doesn't change at all.** It already reads verdicts from disk. It doesn't care whether they were written by the original spawn or a recovery spawn. The completion hash inputs are identical. The audit trail is identical. This is the architecture paying off.

### The thing that bugs me

There's an ambiguity I want to flag before any of this becomes a proposal. **What does "the current round" mean when jewilo was killed?**

```
  scenario X: jewilo killed DURING gather()
    → round N dirs exist, some verdicts written, state.json says current_round = N
    → RECOVER target = N. Clear.

  scenario Y: jewilo killed AFTER gather(), DURING evaluate/hash-write
    → all verdicts written, maybe completion.json partially written
    → RECOVER target = N, but work is essentially done — just re-evaluate

  scenario Z: jewilo killed BEFORE any child spawned (mid snapshot capture)
    → round N dir may not exist, or verdicts all null, no orphan
    → RECOVER target = N, degenerates to "run round N from scratch"
    → but is that even "recover"? it's just NEW-with-known-goalId
```

Scenario Z is the uncomfortable one. If nothing was spawned, is RECOVER distinguishable from re-running NEW against an existing goalId? The clean answer is probably: **RECOVER is just "ensure round N reaches consensus, by whatever means"** — it's idempotent. If nothing's been done, it does everything. If everything's done except the hash, it just writes the hash. If half is done, it does the other half.

That idempotency framing is clean, but it also means RECOVER ≈ "RESUME without the round increment and without fix-notes". The existing `RESUME` is almost the right shape — it's just overloaded with two concerns (round advancement + work driving) that RECOVER wants to split apart.

### Open questions worth sitting with

1. **Is RECOVER a separate command, or a flag on RESUME?** `RESUME <id> --recover` (no round increment) vs `RECOVER <id>`. Separate command is clearer; flag risks users typing `RESUME` reflexively and silently getting the wrong semantics.

2. **Orphan detection: persist PIDs or not?** Without PIDs, scenario B re-spawn double-pays for tokens. With PIDs, you can kill cleanly. The PID file is cheap but it's a new on-disk artifact that needs versioning/cleanup.

3. **Does RECOVER re-render the prompt?** The snapshot was captured at round start. By RECOVER time, the working tree may have changed (if an outer agent kept editing). Re-capturing = different prompt = different audit trail for the same round. Not re-capturing = stale context. This is a real tension — the current "frozen snapshot per round" invariant was deliberate (D5-ish). I lean toward **not** re-capturing: RECOVER is "finish what was started", not "restart with fresh context". If you want fresh context, that's what round N+1 (RESUME) is for.

4. **What if consensus already failed (all verdicts in, n/m not met)?** Then RECOVER is a no-op — there's nothing to salvage, the round genuinely failed. RECOVER should detect this and tell the user "round N is complete but failed; use RESUME for round N+1". This is the boundary between RECOVER and RESUME, and it needs to be crisp.

5. **The interaction with the in-progress `fix-prompt-bloat-and-compaction-recovery` change.** That change adds compaction-recovery (D6: auto-resume same SID after compaction). That's *within-round, same-process* recovery. RECOVER is *cross-process* recovery. They're orthogonal but the naming will collide — "compaction recovery" vs "round recovery" vs "process recovery". Worth nailing down vocabulary now.

---

The shape that's emerging for me: **RECOVER is a thin, idempotent "drive round N to consensus" command that reuses the existing spawn+gather+evaluate pipeline but skips round increment and fix-notes, and spawns only the null-verdict slots.** The verdict file as resumption contract makes this almost free to build. The hard parts are (a) orphan handling if you care about token waste in scenario B, and (b) the prompt-snapshot staleness tension.

Want me to keep going on any of these threads — the orphan-PID question, the RECOVER-vs-RESUME command shape, or the snapshot-staleness tension? Or does this feel solid enough to draft as an openspec change proposal?

## Status at end of turn 2

Design space opened. Core insight: verdict file = resumption contract; consensus::evaluate unchanged. Two-scenario framing (A=children dead, B=orphans alive). Design fork: poll vs re-spawn. Orphan-collision hazard surfaced. PID-file proposal. Idempotent RECOVER framing. 5 open questions raised. Next: turn 3 probes pi SID timing and confirms early-SID persistence.
