# Explore Turn 2 — locking-decisions-six-threads

> Date: 2026-07-03
> Phase: explore (decision locking)
> Status: complete → turn3

## [user] (verbatim — trust chain L1)

> The Core Problem [diagram reproduced] > correct
>
> Your Proposed Architecture [diagram reproduced] > correct
>
> Interesting Threads to Explore
>
> ### 1. The Trust Boundary — What Exactly Are We Protecting Against?
> [assistant question about (a) prevent A lying vs (b) prevent V* bias]
> > V* is authority over A.
> > V* is LLM itself (cli agents).
> > chance of V incorrectly APPROVE a goal is 40% each time. IF with 3/3 model , the chance of V* being wrong is 0.4^3 , must lower than single A or single V*. This is the entire purpose of V*.
>
> ### 2. Session Reuse — Why?
> [assistant question per-verifier vs per-round]
> > CORRECT. [all verifiers refreshed after maxTurn]
> or just the ones that have been used 3 times? > this is more improvement later , not now. Just note this as well.
>
> ### 3. The n/m Model — When is Unanimous Not Required?
> > I will configured this. It is NOT smart / dynamic chosen. Human configured in the json.
> [assistant question meaning]
> > correct. [n/m = need n approvals out of m total verifiers]
>
> ### 4. The Completion Hash — Fool-Proof Against Tampering
> [assistant question about A deleting ~/.verifier-loop/]
> > if it delete , then audit will fail. Because there is no proof.
> > if it is not hard to have write-once log. Then make it.
> > also , verdicts should also stored the: actual message being sent to the verifier initially ; AND it last output message in each verdict as well.
>
> ### 5. Backend Agnosticism — acpx / pi / hermes
> [assistant question (a)/(b)/(c) for verdict detection]
> > a SEPARATE cli for verifier. Denote it is verifier-verdict reject --goalId <id> --notes "...".
> > that cli automate works with the json files store we are have
> > **NO**. ABSOLUTE no "pattern / regex" match.
> > yep (explanation in (a)) [verifier calls CLI command]
>
> ### 6. The Verifier's Interface — What Does V* See?
> > default: cwd
> > git status on that directory.
> > files edited time.
> > top 10k chars of the git status in diff mode.
> > no , it do not need to are how many round.
> > optional / configurable. [previous round summary]
> > yes , none of these should be shown (except previous round , if configured)
> > it output is the tools call in the cli.
> > suggestion is in the "notes" itself.
> > C: *ABSOLUTE* no. it must not need to worry how the process works internally.
> > B: must be a 100% deterministic cli.
> > if it forgot. Then it is NOT setting NULL -> APPROVE. Then that round is invalid.
> > V* always have access to CLI.

## [assistant]

### Two-CLI Design (the key architectural decision)
```
┌──────────────────────────────────────────────────────────────────┐
│                     Process Separation                           │
├──────────────────────────────────────────────────────────────────┤
│   A's process                    V*'s process (each verifier)    │
│   ┌──────────────┐               ┌──────────────────┐           │
│   │ verifier-    │   spawns      │ pi -p / acpx /   │           │
│   │ loop CLI     │──────────────▶│ hermes session   │           │
│   │              │◀──────────────│                  │           │
│   │              │   hash or     │  ┌────────────┐  │           │
│   │              │   rejection   │  │ verifier-  │  │           │
│   └──────────────┘               │  │ verdict    │  │           │
│                                  │  │ CLI        │  │           │
│   A can ONLY:                    │  └─────┬──────┘  │           │
│   - NEW <goal>                   │        ▼         │           │
│   - RESUME <goalId> --fix "..."  │  writes verdict  │           │
│   - Read rejection output        │  to JSON store   │           │
│                                  └──────────────────┘           │
│                                                                  │
│   A CANNOT:                                                      │
│   - Write verdicts                                               │
│   - Modify goal.json                                             │
│   - See V*'s internal state                                      │
│   - Delete ~/.verifier-loop/ (would invalidate hash)             │
└──────────────────────────────────────────────────────────────────┘
```

### Verdict File Format (expanded with stored messages)
```
~/.verifier-loop/goals/<goalId>/
├── goal.json                    (IMMUTABLE - written once at NEW)
│   ├── goalId, goalText, signature, createdAt, context, config
│
├── rounds/
│   ├── round-1/
│   │   ├── v1/
│   │   │   ├── verdict.json     (APPROVE/REJECT/null + notes)
│   │   │   ├── initial-prompt   (what was sent to V1 — LOCKED)
│   │   │   └── final-output     (V1's last message before verdict)
│   │   ├── v2/ ...
│   │   └── fix-notes.json       (A's --fix from RESUME, append-only)
│   └── round-2/ ...
│
├── completion.json              (ONLY written on n/m match)
│   ├── hash, matchedAt, verdicts, rounds
│
└── .salt                        (CLI-generated secret, mode 600)
```

### Trust Model — Confirmed
- **A is adversarial** (incentive to skip/dodge verification)
- **V* is honest but fallible** (40% error rate, compensated by n/m)
- n/m compensates: 3/3 → wrong prob = 0.4³ = 6.4%
- No need to protect against V* cheating — V* has no incentive

### Verdict Registration — Must Be Deterministic CLI
- NO pattern/regex matching on V* output (forbidden)
- V* calls `verifier-verdict reject --notes "..."` — deterministic
- First verdict is FINAL; second call rejected
- Forgotten verdict = NULL = no-pass (fail-closed)
- V* does NOT write files directly — must use CLI

### What V* Sees (confirmed)
- Blind prompt (verifier.md baked in)
- Goal (immutable)
- cwd, git status, file edit times, git diff top 10k chars
- Does NOT see: round number, other verdicts, n/m config, completion hash, A's fix notes (unless configured)
- Previous round summary = optional/configurable via template

### Full Flow Sketch
```
1. A runs: verifier-loop NEW "fix auth"
   → generate goalId, signature, write goal.json (immutable)
   → spawn V1, V2, ... Vm in parallel
   → each V* reviews, calls verifier-verdict
   → CLI collects verdicts, checks n/m match
   → if match: output hash; else: output rejection to A
2. A reads rejection, fixes issues
3. A runs: verifier-loop RESUME <goalId> --fix "..."
   → load state, increment round, spawn NEW V* (or reuse if <maxTurn)
   → ...repeat until n/m or A gives up
4. A includes hash in commit/PR
5. (deferred) audit subcmd
```

### Questions Remaining
1. How does V* know its identity (goalId/verifierId/round)?
2. Backend adapter abstraction details?
3. What does verifier-verdict output to V*?
4. How does V* exit after verdict?
5. Can V* change verdict after registering?

Status at end of turn 2: All six threads locked. Two-CLI design confirmed. Need to ground in actual pi --mode json behavior. → turn3
