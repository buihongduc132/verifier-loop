# Intention — verifier-loop skill: rationale split (2026-08-17)

- id: verifier-loop-skill-rationale-split
- scope: `~/.pi/agent/skills/verifier-loop/SKILL.md` (mirrored: `pi-plugins/profile/skills/verifier-loop/SKILL.md`)

## User intent (verbatim, condensed)

> check this skill then fix it ; a. it is currently containing the explanation of 60% thing ; all of
> these are the REASON why the human want AGENT to USE the skill ; the AGENT do not need to having
> to understand / even aware of it ; FUCKING STRIP that part;
> must identify: (1) MINIMAL part to ENFORCE and GUIDE the agent on HOW to do the verifier loop;
> (2) reason WHY human want to build it AND why agent MUST use it; (3) others.
> Strip skill, only leave (1). (2)(3) → project flow/intentions/.

## Decision

SKILL.md = part (1) only — operational playbook (prereqs, jewilo invocation, loop rules,
orchestrator role, no-reuse, blind review, escalation, examples, violation detection).
Everything below = stripped content, preserved verbatim.

## Part 2 — WHY human built it / WHY agent must use it

### WHY jewilo is primary

- Tamper-evident (signed verdicts, hash-chained receipt log)
- Deterministic consensus (n/m threshold, not "mostly approve")
- Survives process death (goal state persisted to disk)
- Audit trail (every verdict logged with timestamp + verifier identity)

Subagent spawning = legacy fallback. jewilo = production path.

### Why A MUST Be Intermediary

CRITICAL: sub-agents CANNOT pass torch to each other directly.

Humans hand torch to human. Sub-agents CANNOT do this.
If verifier talk directly to fixer:
- Verifier bias fixer: "fix X, Y, Z"
- Fixer know what verifier look for
- Blind review compromised
- Context leak between rounds

So A MUST be intermediary:
- A take REJECTION from verifier → throw raw rejection at fixer (no filter, no hint)
- A take FIX from fixer → pass to NEW verifier (no context about what fix)
- A ensure torch stay clean — no contamination between rounds

### Why blind review (bias rationale)

WHY: specifying what check bias verifier toward confirming fixes instead of
independently discovering issues. Biased verifier = USELESS verifier.
Defeat entire purpose of verification.

### Torch Metaphor (Olympic Relay)

```
                    GREECE (the goal)
                         ↑
                         │
    ┌────────────────────┼────────────────────┐
    │                    │                    │
    │     THE TORCH      │   (the work)       │
    │    [artifact]      │                    │
    │                    │                    │
    └────────┬───────────┴───────────┬────────┘
             │                       │
             ↓                       ↓
    ┌─────────────────┐     ┌─────────────────┐
    │    VERIFIER     │     │     FIXER       │
    │  (blind review) │     │  (fix problems) │
    │                 │     │                 │
    └────────┬────────┘     └────────┬────────┘
             │                       │
             └───────────┬───────────┘
                         ↓
                    ┌─────────┐
                    │   (A)   │
                    │ ORCHES- │
                    │ TRATOR  │
                    │         │
                    │ passes  │
                    │ torch   │
                    │between  │
                    │ them    │
                    └─────────┘
```

### How Torch Moves (relay walkthrough)

```
Round 1:
  A produce work → pass to VERIFIER-1
  VERIFIER-1 find problems → REJECT + issue list
  A take rejection → throw at FIXER face
  FIXER fix → produce fixed artifact
  A take fix → pass to VERIFIER-2 (NEW, unbiased)

Round 2:
  VERIFIER-2 blind review → REJECT + new issues
  A take rejection → throw at FIXER
  FIXER fix again
  A take fix → pass to VERIFIER-3 (NEW, unbiased)

Round N:
  VERIFIER-N blind review → APPROVE (both verifiers MUST approve)
  A confirm: unanimous approval → DONE
```

## Part 3 — others (mathematical justification, stripped from skill)

### The Math: Why Loop Works

Section explain WHY multiple rounds needed.
Mathematical proof: single pass NEVER enough.

### Variables

- c = worker correctness rate (~0.6 in practice — workers get ~60% right)
- v = verifier catch rate (~0.6 — one verifier catch ~60% of errors)
- W = total wrong items remaining

### Core Equation

Each round transform wrong count:

```
W_out = (1 - v*c) * W_in
```

Why: verifier catch v fraction of wrongs. Worker fix those, but only
c fraction correctly. (1-vc) survive = things verifier miss
PLUS things worker fail fix correctly. Both feed into next round.

GEOMETRIC DECAY:

```
W_n = W_0 * (1 - v*c)^n
```

### Concrete Example: 100 items, c=0.6, v=0.6

```
decay ratio per round = 1 - (0.6 * 0.6) = 0.64

  Round | Wrong remaining | % correct
  ------+-----------------+----------
    0   |      40         |   60%
    1   |    25.6         |   74%
    2   |    16.4         |   84%
    3   |    10.5         |   90%
    4   |     6.7         |   93%
    5   |     4.3         |   96%
    6   |     2.7         |   97%
    7   |     1.8         |   98%
    8   |     1.1         |   99%
    9   |     0.7         |  99.3%
```

~9 rounds drive errors below 1 item with 1 verifier.

### With 2 Verifiers (MANDATORY minimum)

```
combined catch rate = 1 - (1-v)^2 = 1 - 0.16 = 0.84
decay ratio = 1 - (0.84 * 0.6) = 0.496

  Round | Wrong remaining | % correct
  ------+-----------------+----------
    0   |      40         |   60%
    1   |    19.8         |   80%
    2   |     9.8         |   90%
    3   |     4.9         |   95%
    4   |     2.4         |   98%
    5   |     1.2         |   99%
    6   |     0.6         |  99.4%
```

2 verifiers converge ~1.5x FASTER than 1 verifier. 6 rounds vs 9 rounds.
THIS = why skill mandate AT LEAST 2 verifiers.

### ASCII: Decay Funnel

```
1 verifier (decay=0.64):          2 verifiers (decay=0.496):

   40 ┃██████████████████              40 ┃██████████████████
   25 ┃████████████                     20 ┃█████████
   16 ┃████████                          10 ┃████
   10 ┃█████                              5 ┃██
    7 ┃███                                2 ┃█
    4 ┃██                                 1 ┃
    1 ┃                                   0 ┃
      └────────────────                    └────────────────
       0  2  4  6  8                        0  2  4  6

  SLOW convergence                      FAST convergence
  9 rounds to <1 error                  6 rounds to <1 error
```

### Trap: Why People Stop Too Early

```
  Round 1: worker do 60% → "looks mostly done!"
  Round 2: verifier catch some → "oh, few fixes needed"
  Round 3: worker fix → "surely done now?"

  REALITY: round 3 still has 10% wrong (1 verifier)
                                  or 5% wrong (2 verifiers)

  10% of 100 items = 10 silent bugs ship to production.
  5% of 100 items = 5 silent bugs ship to production.

  Loop NON-NEGOTIABLE because math non-negotiable.
  You MUST NOT stop when "looks done." You MUST stop when W_n < 1.
```

### Practical Implication

- NEVER accept round 1 output as final — 40% wrong, guaranteed
- Round 2-3 = real quality emerge — but still 5-10% wrong
- With 2 verifiers, round 4-5 typically approach zero
- "Feels done" trap strongest at round 2-3 — resist it
- More verifiers (3+) = diminishing returns; 2 = sweet spot
