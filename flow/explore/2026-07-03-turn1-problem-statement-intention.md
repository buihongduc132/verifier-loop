# Explore Turn 1 — problem-statement-and-intention

> Date: 2026-07-03
> Phase: explore (problem framing)
> Status: complete → turn2

## [user] (verbatim — trust chain L1)

> Tell me how to turns that verify loop into the cli tools as well ;
>
> Problem:
> - currently it is NOT enforced with the programmatically and would be easily to by pass by AI AGENT;
> - it can tailor the message on itself to giving bias / narrow scope , dodge the wrong parts;
>
> My intention:
> - turns this into cli. A single cmd usage , only invoke once , 1 initial prompt . Will only be able to appended comment (like what did it fixed ) but unable to change init prompt ; initial one is the north star goal;
> - this can configured to run these: -- acpx / pi / hermes cmd in non-interactive approach ;
> --- carefully search and understand how these --json mode works to be able to parse it correctly to have the SID for resume.
> -- use json mode for easily parsed result and also having the session id (SID)
> -- this cli is be able to configured to resume SID; can configured reuse-up-to-max-turn (default : 3). Which will reuse up to 3 turns max; then fresh context of verifier again ;
> --- configurable to have each turn having n/m approval. Usually it is 2/2 (unanimous) , but it can config as 2/3 , 3/5 as well. Denote each verifier is V<num>;
> -- invokation can have append (preconfigured ) prompt ;
> - EACH invokation will generate the 1 single signature including: (providedGoal , timestamp). this is for non-tampered.
> -- it also generate a ~/.<cliName>/verifier_goal/<1SpecifyDirectory of that whole verification>/*; All the artifact related to that turns stays here ;
> Hidden from the view of the agent whom invoke the <verifier loop cli>; (consider agent whom invoke <verifier loop cli> is the (A))
> -- initially it will generate: GOAL , [{ turnIndex: number , ...fieldsIfNeeds , verdicts: [<verifyObj>: { verifierId, status: null | APPROVE | REJECT , notes: string[] , ... any other useful fields}, ...] }; (*)]
> --- verifier status having null initially , sub agents will have to invoke APPROVE / REJECT , if REJECT , notes are required.
> ---- This null status to be able to identify if the cli agent is actually run or not. If it not , we can have fallback mechanism later (but defer for now).
> -- this (*) inner schema DO NOT expose to either: verifier (V*) or (A).
> --- V* will have the specific set of other cli that exposed the CRUD functionalities of (*) to it;
> --- BASE on configuration , V* cli will be able to determine if it match n/m or not; IF NOT then output all status , notes to the (A)
> --- consider each verification is (goal2v: goal to verify). This will have >=1 round of V*; EACH goal2v will have 1 single ID. Its for (A) to be able to resume. NEW / RESUME will be 2 separate sub cmd.
> --- on FINAL n/m match. cli must output a hash to (A). A can use it in the signoff documentation / commit to prove that it passed the <verifier loop cli>;
> ---- completion hash will have these nature:
> a. it must be able to find the correlate verify_goal of itself for audit.
> b. generation process of this must be hidden from A.
> c. it must be fool-proof. Even A try to tamper that hash. it will be invalid / non-match to any verified;

## [assistant]

Framed the core problem: current verifier-loop skill is bypassable by A (the agent being verified). A is both defendant AND judge.

### Core Problem Diagram
```
┌─────────────────────────────────────────────────────────────┐
│                    Current State                              │
├─────────────────────────────────────────────────────────────┤
│   Agent A ──────▶ spawns verifiers via teams/intercom      │
│       │                                                      │
│       ├─▶ Can craft biased prompts to V*                    │
│       ├─▶ Can skip verification entirely                    │
│       ├─▶ Can claim "verifier approved" without proof       │
│       └─▶ Controls the entire verification lifecycle        │
│                                                             │
│   Trust model: A is both defendant AND judge               │
└─────────────────────────────────────────────────────────────┘
```

### Proposed Architecture (initial sketch)
```
┌─────────────────────────────────────────────────────────────┐
│                 verifier-loop CLI                             │
├─────────────────────────────────────────────────────────────┤
│   ~/.verifier-loop/                                         │
│   ├── goals/                                                │
│   │   └── <goalId>/                                         │
│   │       ├── goal.json          (IMMUTABLE - north star)  │
│   │       ├── signature.json     (goal + timestamp + salt) │
│   │       ├── rounds/                                       │
│   │       │   ├── round-1/                                  │
│   │       │   │   ├── v1-verdict.json                       │
│   │       │   │   ├── v2-verdict.json                       │
│   │       │   │   └── fix-notes.json   (A's append-only)   │
│   │       │   └── round-2/                                  │
│   │       │       └── ...                                   │
│   │       └── completion.json    (hash + metadata)         │
│   │                                                         │
│   └── config.json              (n/m, max-turn, backend)    │
└─────────────────────────────────────────────────────────────┘
```

### Six Open Threads Surfaced
1. **Trust boundary** — what exactly are we protecting against?
2. **Session reuse** — per-verifier or per-round? After maxTurn, all refreshed or just used ones?
3. **n/m model** — static config or dynamic? meaning "need n out of m"?
4. **Completion hash** — fool-proof against tampering? what if A deletes files?
5. **Backend agnosticism** — acpx/pi/hermes adapters? how to detect APPROVE/REJECT?
6. **Verifier interface** — what does V* see? how to register verdict?

### Key Observations
- A is adversarial (incentive to skip/dodge verification)
- V* is honest but fallible (40% error rate per verifier)
- n/m model: 3/3 → wrong probability = 0.4^3 = 6.4% (purpose of V*)
- Goal immutability prevents goalpost moving
- Blind prompts prevent A biasing V*
- Fail-closed required: NULL verdict = no-pass, not silent-pass

Status at end of turn 1: Problem framed. Six threads to explore. → turn2
