# Explore Turn 5 — code-location-cli-aliases

> Date: 2026-07-03
> Phase: explore (final decisions)
> Status: complete (explore done, ready for proposal)

## [user] (verbatim — trust chain L1)

> Make the other decision: code stays in ../verifier-loop , cli will be verifier-loop and verifier-verdict (aliased respectively to: jewilo , jewije);

## [assistant]

### Final Decisions Locked

```
CODE LOCATION:
  ../verifier-loop/          (sibling repo to pi-plugins)

CLI BINARIES:
  verifier-loop              → aliased to: jewilo
  verifier-verdict           → aliased to: jewije
```

### Complete Decision Summary (all turns consolidated)

```
┌──────────────────────────────────────────────────────────────────┐
│                    ALL LOCKED DECISIONS                           │
├──────────────────────────────────────────────────────────────────┤
│                                                                  │
│  ARCHITECTURE                                                    │
│  ├── Two CLIs: verifier-loop (jewilo) + verifier-verdict (jewije)│
│  ├── Code: ../verifier-loop/ (sibling repo)                     │
│  ├── ACP JSON stream parsing (universal across backends)        │
│  ├── Parallel non-blocking verifier spawns                      │
│  └── Fail-closed on all failure modes                           │
│                                                                  │
│  TRUST MODEL                                                     │
│  ├── A is adversarial (incentive to skip/dodge)                │
│  ├── V* is honest but fallible (40% error rate)                │
│  ├── n/m compensates: 3/3 → wrong prob = 0.4³ = 6.4%           │
│  ├── Goal immutability prevents goalpost moving                 │
│  ├── Blind prompts prevent A biasing V*                         │
│  └── Completion hash prevents forgery                           │
│                                                                  │
│  VERIFIER IDENTITY                                               │
│  ├── Both env var AND prompt {{}}                               │
│  ├── ENV: machine-readable, for verifier-verdict CLI            │
│  │   VERIFIER_LOOP_GOAL_ID, _VERIFIER_ID, _ROUND                │
│  └── PROMPT: human-readable, V* sees its identity               │
│                                                                  │
│  VERDICT REGISTRATION                                            │
│  ├── Deterministic CLI (no pattern/regex matching)              │
│  ├── V* calls: verifier-verdict approve | reject --notes "..."  │
│  ├── First verdict is FINAL (no changes after)                  │
│  ├── Forgotten verdict = NULL = no-pass (fail-closed)           │
│  └── V* naturally exits after verdict (agent_end)               │
│                                                                  │
│  SESSION REUSE                                                   │
│  ├── maxTurn default 3                                          │
│  ├── All verifiers refreshed after maxTurn (round-based)        │
│  ├── Per-verifier refresh = deferred improvement                │
│  └── Resume via pi --session <sid> -p "..." --mode json         │
│                                                                  │
│  CONSENSUS                                                       │
│  ├── n/m = need n approvals out of m total verifiers            │
│  ├── Static config (human-configured, not dynamic)              │
│  └── On match: output hash; else: output rejection to A         │
│                                                                  │
│  ARTIFACTS CAPTURED                                              │
│  ├── cwd (working directory)                                    │
│  ├── git status --porcelain                                     │
│  ├── file edit times                                            │
│  ├── git diff top 10k chars (configurable)                      │
│  ├── initial-prompt.txt (what was sent to V*)                   │
│  └── final-output.txt (V*'s last message)                       │
│                                                                  │
│  COMPLETION HASH                                                 │
│  ├── vl: + 40 hex chars (160 bits)                              │
│  ├── Inputs: salt + goalId + goalSignature + roundNumber +      │
│  │          matchingVerdicts + matchedAtISO                     │
│  ├── goalSignature = SHA256(salt + goalText + createdAt)        │
│  ├── salt = mode 600, CLI-generated, never exposed to A         │
│  └── Salt gap (deferred): chattr +a hardening                   │
│                                                                  │
│  CONFIG                                                          │
│  ├── ~/.verifier-loop/config.json                               │
│  ├── n, m, maxTurn, backend, model, gitDiffMaxChars             │
│  ├── cwd, verifierTimeoutSec                                    │
│  ├── verifierPromptTemplate (null = baked-in default)           │
│  ├── verifierResumePromptTemplate (null = baked-in default)     │
│  └── {{process.env.*}} injection in templates                   │
│                                                                  │
│  BACKEND ADAPTERS                                                │
│  ├── Built-in: pi, hermes, acpx                                 │
│  ├── Custom: config.json with spawn/resume templates            │
│  ├── All share ACP stream parser                                │
│  └── spawnCmd + resumeCmd per adapter                           │
│                                                                  │
│  HIDDEN FROM A                                                   │
│  ├── Verifier internal state                                    │
│  ├── Other verifiers' verdicts                                  │
│  ├── n/m configuration                                          │
│  ├── Completion hash (until match)                              │
│  ├── Round number (V* doesn't need to know)                    │
│  └── Salt / hash generation process                             │
│                                                                  │
│  DEFERRED ITEMS                                                  │
│  ├── audit subcmd (verifier-loop audit <hash>)                 │
│  ├── Per-verifier maxTurn refresh (not just round-based)        │
│  ├── chattr +a hardening on .salt                               │
│  └── Skill → thin wrapper around CLI (after CLI stable)        │
│                                                                  │
└──────────────────────────────────────────────────────────────────┘
```

### Next Steps
1. Create OpenSpec change `add-verifier-loop-cli` in `../verifier-loop` repo
2. Artifacts: proposal.md, design.md, specs/, tasks.md
3. Implement verifier-loop CLI (jewilo) + verifier-verdict CLI (jewije)
4. Bake in verifier.md content (from skill)
5. ACP adapter for pi (primary backend)
6. Test with real verification scenarios

Status at end of turn 5: Explore complete. All decisions locked. Ready for OpenSpec proposal in `../verifier-loop`. Explore session ends here.
