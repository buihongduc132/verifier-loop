# Explore Turn 4 — config-schema-completion-hash-templates

> Date: 2026-07-03
> Phase: explore (finalization)
> Status: complete → turn5

## [user] (verbatim — trust chain L1)

> This feels close to complete. The key remaining threads:
> - The RESUME prompt template (what V* sees on round 2+)
> > it just like the first. But base on configuration , it might have the previous verdict notes.
> > this configuration could be done in the template as well.
> - The config.json final schema
> - The completion hash exact formula
> > these to , judge yourself then finalize schema , completion hash formula.

## [assistant]

Assistant made judgment calls on config schema + hash formula + templates.

### Config.json Final Schema
```json
{
  "$schema": "verifier-loop-config-v1",
  "n": 2,
  "m": 2,
  "maxTurn": 3,
  "backend": "pi",
  "model": null,
  "gitDiffMaxChars": 10000,
  "cwd": ".",
  "verifierTimeoutSec": 1800,
  "verifierPromptTemplate": null,
  "verifierResumePromptTemplate": null
}
```

Config field reference:
```
Field            │ Purpose
─────────────────┼──────────────────────────────────────────────
n                │ Approvals needed to pass (numerator)
m                │ Total verifiers spawned per round (denom)
maxTurn          │ Rounds a V* session can be reused before fresh
backend          │ "pi" | "hermes" | "acpx" | "custom"
model            │ Model for V* sessions. null = backend default
gitDiffMaxChars  │ Truncate git diff capture. Default 10000.
cwd              │ Working dir for V*. Default = "." (inherit)
verifierTimeout  │ Kill hung V* after N sec → verdict NULL.
                 │ Default 1800 (30 min). Fail-closed.
verifierPrompt   │ null = baked-in default. Override for custom
verifierResume   │ null = baked-in default for round 2+
```

**Design decision**: `verifierResumePromptTemplate` separate from `verifierPromptTemplate` because round 1 and round 2+ have fundamentally different context (fix notes, prev notes, round number). Clearer than conditional blocks.

**Design decision**: NO `showPrevNotes` boolean — `{{prevNotes}}` is just a variable. If template includes it, V* sees it; if not, V* doesn't. Template IS the configuration.

### Template Variables (complete reference)
```
IDENTITY
{{goalId}}              abc-123-def (auto UUID)
{{verifierId}}          v1, v2, v3...
{{round}}               1, 2, 3...
{{prevRound}}           0, 1, 2... (empty string if round 1)

GOAL
{{goalText}}            A's original goal (immutable)
{{context}}             A's --context (may be empty)

ROUND CONTEXT (resume only)
{{fixNotes}}            A's --fix from RESUME (empty on NEW)
{{prevNotes}}           This V*'s own notes from prev round
                        Empty if fresh spawn (maxTurn exceeded)

ARTIFACTS
{{cwd}}                 working directory captured at spawn
{{gitStatus}}           git status --porcelain
{{fileEditTimes}}       file modification times
{{gitDiff}}             git diff, top N chars
{{gitDiffMaxChars}}     the N value (10000 default)

ENVIRONMENT
{{process.env.*}}       any environment variable
```

### Built-in Default Templates

**Round 1 (verifierPromptTemplate):**
```
You are @Verifier-{{verifierId}}.

<verifier.md content — baked in>

GOAL:
{{goalText}}

CONTEXT:
{{context}}

ARTIFACTS:
Working directory: {{cwd}}
Git status:
{{gitStatus}}
File edit times:
{{fileEditTimes}}
Git diff (top {{gitDiffMaxChars}} chars):
{{gitDiff}}

Review the work against the GOAL. Find ALL issues.
Be brutal. Zero trust. Verify everything yourself.

When done, register your verdict:
  verifier-verdict approve
  verifier-verdict reject --notes 'issue 1: ...'

Your verdict is FINAL. You cannot change it.
```

**Round 2+ (verifierResumePromptTemplate):**
```
You are @Verifier-{{verifierId}}.

<verifier.md content — baked in>

GOAL:
{{goalText}}

CONTEXT:
{{context}}

This is round {{round}}. Previous issues have supposedly been addressed.

Fix notes from the worker:
{{fixNotes}}

Your previous notes (round {{prevRound}}):
{{prevNotes}}

ARTIFACTS:
[same as round 1]

Re-review the work against the GOAL. Find ALL issues.
Be brutal. Zero trust. Verify everything yourself.
Do NOT trust that fixes are correct — verify yourself.

When done, register your verdict:
  verifier-verdict approve
  verifier-verdict reject --notes 'issue 1: ...'

Your verdict is FINAL. You cannot change it.
```

### Completion Hash — Exact Formula
```
INPUTS:
1. salt
   ~/.verifier-loop/.salt (64 random hex chars, mode 600)
   Generated once at first run. Never exposed to A.

2. goalId
   The UUID identifying this verification

3. goalSignature
   SHA256(salt + goalText + createdAtISO)
   Written to signature.json at NEW time.
   Ties completion to original creation event.

4. roundNumber
   Which round achieved n/m match

5. matchingVerdicts
   Array of the n APPROVE verdicts from the matching round,
   sorted by verifierId for determinism:
   [{ verifierId, status, notes, registeredAt, sid }, ...]

6. matchedAtISO
   Timestamp when n/m was achieved

FORMULA:
completionHash = "vl:" + first40hex(
  SHA256(
    salt +
    goalId +
    goalSignature +
    String(roundNumber) +
    JSON.stringify(matchingVerdicts) +
    matchedAtISO
  )
)

OUTPUT FORMAT:
vl:a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2
(prefix "vl:" + 40 hex chars = 160 bits)
```

### Why Each Hash Input
```
Input              │ Protects against
───────────────────┼────────────────────────────────────────────
salt               │ A forging a hash (doesn't know salt)
goalId             │ Audit trail (find goal from hash)
goalSignature      │ Goal text tampering (covers goalText +
                   │ creation time — can't move the goalpost)
roundNumber        │ Replaying an earlier round's verdicts
matchingVerdicts   │ Verdict tampering (changing APPROVE
                   │ notes or timestamps)
matchedAtISO       │ Timestamp forgery
```

### Tamper Resistance Proof
```
Attack                                    │ Result
──────────────────────────────────────────┼──────────────────────────
A presents forged hash                    │ recompute ≠ → INVALID
A tampers goal.json (goalText)            │ goalSignature breaks → hash invalid
A tampers verdict.json (add fake APPROVE) │ registeredAt changed → hash won't match
A copies completion.json from another goal│ goalId mismatch → invalid
A deletes ~/.verifier-loop/               │ no files → no proof → fail-closed
A reads .salt and forges                  │ salt mode 600; gap (deferred: chattr +a)
```

### The Salt Gap (deferred hardening)
A runs as same user → can technically read `.salt`. Forging requires knowing salt + all inputs + formula. For production hardening later: `chattr +a` on `.salt` (append-only) or separate UID.

### Complete Architecture — Final
```
~/.verifier-loop/
├── .salt                         (mode 600, CLI-generated)
├── config.json                   (human-edited)
└── goals/
    └── <goalId>/
        ├── goal.json              (IMMUTABLE)
        │   { goalId, goalText, context, createdAt, config snapshot }
        ├── signature.json          (IMMUTABLE)
        │   { signature: SHA256(salt + goalText + createdAt) }
        ├── rounds/
        │   ├── round-1/
        │   │   ├── v1/
        │   │   │   ├── verdict.json  (pre-created: null)
        │   │   │   ├── initial-prompt.txt (captured)
        │   │   │   ├── final-output.txt   (captured)
        │   │   │   └── meta.json       { sid, turnsUsed }
        │   │   ├── v2/  ...
        │   │   └── fix-notes.json     (A's --fix, append)
        │   └── round-2/ ...
        └── completion.json        (ONLY on n/m match)
            { hash, goalId, roundNumber, matchedAt, matchingVerdicts }
```

Status at end of turn 4: Config schema, templates, and hash formula finalized. All architectural decisions locked. → turn5
