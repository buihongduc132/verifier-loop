# Explore Turn 3 — SID early-persistence probe

> Date: 2026-07-12
> Phase: probe + correction + refine
> Status: corrected + refined → new insight

## [user] (verbatim — trust chain L1)

> qThere's a beautiful property hiding in this architecture that most people miss: the verdict file IS the resumption contract.
>
>  ```
>    verdict.json   ← written by the CHILD (verifier-verdict CLI), independently of jewilo
>                     first-write-wins: {status:null} pre-created → APPROVE/REJECT atomic
> > isn't verdict should be per round?
>    meta.json      ← written by jewilo's gather() AFTER child exits (SID + turnsUsed)
>    completion hash ← computed over verdicts + signature + round  (NOT meta.json)
>  ```
>
>  So meta.json is purely for the "should I reuse this SID next round?" decision. It plays zero role in consensus or the completion hash. That means: jewilo's job at gather time is just to (a) wait for children, (b) reap meta, (c) read verdicts. If we separate "spawn" from "collect", resumption collapses into "collect again".
>
>  This is the whole game. Let me show you.
>
>  The two kill scenarios behave very differently
>
>  ```
>    SCENARIO A — Ctrl-C in a TTY (interactive)
>    ──────────────────────────────────────────
>      terminal sends SIGINT to whole foreground pgrp
>      jewilo dies  AND  all child verifiers die (same pgrp, no setsid)
>
>      disk state after kill, round N mid-gather:
>      ┌─────────────────────────────────────────────┐
>      │ rounds/N/v1/  verdict=APPROVE  meta=stale    │ ← child finished, wrote verdict, exited
>      │ rounds/N/v2/  verdict=null     meta=missing  │ ← child killed before writing
>      │ rounds/N/v3/  verdict=REJECT   meta=stale    │ ← child finished, wrote verdict, exited
>      └─────────────────────────────────────────────┘
>      processes: NONE alive
>      → round is "half-finished" on disk. No live work to reattach to.
>
>
>    SCENARIO B — SIGTERM / kill (non-TTY, e.g. outer agent)
>    ───────────────────────────────────────────────────────
>      jewilo dies. kill_on_drop is a no-op (Drop doesn't run on signal).
>      children are ORPHANED — reparented to init, still computing, still burning tokens.
>
>      disk state:
>      ┌─────────────────────────────────────────────┐
>      │ rounds/N/v1/  verdict=APPROVE  meta=stale    │
>      │ rounds/N/v2/  verdict=null     meta=missing  │ ← orphan pi STILL RUNNING
>      │ rounds/N/v3/  verdict=REJECT   meta=stale    │
>      └─────────────────────────────────────────────┘
>      processes: v2's backend is alive, maybe seconds from writing its verdict
>      → there IS live work, but jewilo can't reattach to pipes it didn't open
>  ```
>
>  Scenario B is the nasty one. Even though there's live work, jewilo cannot reattach — the tokio::process::Child handle, the stdout pipe, the SID discovery all died with the old process. The only durable signal an orphan leaves is verdict.json appearing on disk.
> > run the pi in json mode "say hi" , in the very initially , it is already having the SID.
>
> JUST probes then , then give me your thoughts again , more concise; less code.

## [assistant]

**Probes confirm:**

| Probe | Result |
|---|---|
| SID timing | First stdout line = `{"type":"session","id":"..."}`. Before any agent work. |
| Session file persistence | Survives `kill`. `--session <sid>` loads it after process death. |
| Concurrent same-SID | Second pi starts **independent** — does NOT attach/wait for live original. Just loads session file. |

And yes — verdict IS per round (`rounds/N/vN/verdict.json`). My framing stands; you're confirming it.

### Revised thoughts (concise)

**The early-SID changes one thing, not everything.**

```
OLD mental model:  SID captured at gather() end → meta.json after child exits
                   jewilo killed mid-gather → SID LOST

NEW (probe-proven): SID on stdout line 1 → can persist to meta.json IMMEDIATELY
                   jewilo killed mid-gather → SID SAFE on disk
```

So the orphan's session file is durable from t≈0. After jewilo dies, a new jewilo can `pi --session <sid>` to **resume that conversation** — load its state, ask "what's your verdict", harvest it.

**BUT — probe also showed the catch:** concurrent same-SID does NOT coordinate. Two pi processes sharing a session file = silent race. So:

```
  orphan ALIVE + you resume by SID  →  TWO processes writing same session file
                                       = corruption risk, undefined

  orphan DEAD + you resume by SID   →  SAFE. Loads state, continues clean.
```

**So same-round recovery has exactly two safe shapes:**

1. **Wait for verdict.json** (orphan finishes, writes verdict independently — no SID needed). Then evaluate. Zero reattach.

2. **Kill the orphan first, THEN resume by SID** (now safe — single writer). You salvage the orphan's *conversation context* (the thinking it already did) without re-running from scratch. The verdict-nudge is tiny: "you were interrupted; emit your verdict now."

Shape 2 is the real prize. It means an interrupted round doesn't lose the verifiers' accumulated reasoning. The cost is: **detect the orphan (PID file), kill it, resume-by-SID with a <2KB nudge.** Same fail-closed: no verdict → null → no hash.

```
  RECOVER <goalId>
    │
    ▼
  for each slot in round N:
    verdict non-null?  → KEEP
    verdict null?
      ├─ orphan alive (pid file)?  → kill → resume by SID → nudge → gather
      └─ no orphan                 → spawn fresh → gather
```

The early-SID persistence is what makes shape 2 possible at all. Without it, the orphan's session is orphaned in both senses — process and context both lost.

**One open wrinkle:** the orphan's session file may be mid-write (partial JSONL) when killed. `--session` resume needs to tolerate a truncated tail. That's a pi-side robustness question, not a jewilo one — worth a probe before committing to shape 2 as default.

## Status at end of turn 3

Corrected: verdict is per round (already true). Probed: SID emitted at t≈0, session file survives kill, concurrent same-SID unsafe. Refined to two safe shapes; shape 2 (kill orphan → resume by SID → nudge) emerges as the winner. One wrinkle (truncated session file) left open. Next: turn 4 locks shape 2.
