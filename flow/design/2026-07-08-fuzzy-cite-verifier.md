# Fuzzy Cite Verifier — Design Notes

> Date: 2026-07-08
> Status: design (explore-mode artifact, NOT implemented)
> Scope: Problem B — standalone tool, SEPARATE from verifier-loop

## Problem

Cite verification currently relies on LLM verifiers reading docs. We need a deterministic tool that can:

1. **NEGATE mode (PRIMARY)**: Prove "this cite IS WRONG" — cite claims line X says Y, but it doesn't
2. **FUZZY mode**: Accept ±5 lines, 90% match threshold

## Approach

### Skip exact matching entirely

No exact string match. Always fuzzy.

### Default: ±5 lines, 90% threshold

```
Cite says: "L1724: const fee = gross * rate"
Search range: L1719–L1729
Threshold: ≥90% fuzzy match (rapidfuzz ratio)
```

### NEGATE mode (the one we'll use most)

```
IF fuzzy_score < 5% across entire search range (±5 lines):
    THEN cite IS WRONG → CONFIRMED FABRICATION
ELSE:
    cite is plausibly present → NOT CONFIRMED WRONG

Purpose: "prove these cites ARE wrong"
         NOT "prove these cites ARE right"
```

The negation logic: if the cited quote doesn't appear within ±5 lines AT ALL (score <5%), the cite is definitively wrong. If it DOES appear (even fuzzily), we can't confirm wrongness — it might be there.

```
┌─────────────────────────────────────────────────┐
│  NEGATE MODE DECISION LOGIC                      │
│                                                  │
│  Cite: "L1724 says: <quote>"                     │
│  Search: L1719–L1729                             │
│                                                  │
│  max_fuzzy_score = max(rapidfuzz(quote, line)    │
│                         for line in L1719..L1729)│
│                                                  │
│  IF max_fuzzy_score < 5%:                        │
│     → VERDICT: WRONG (fabrication confirmed)     │
│  ELSE:                                           │
│     → VERDICT: PLAUSIBLE (cannot prove wrong)    │
└─────────────────────────────────────────────────┘
```

## Why negation is the primary mode

In the anton-calc-migration cite audit, we had 115 rows. The verifier's job was to find WRONG cites. We don't need to prove every cite is perfect — we need to **catch the fabricated ones**.

Negation is binary and deterministic:
- Score < 5% → WRONG (high confidence)
- Score ≥ 5% → move on (can't prove wrong, not worth more time)

## Implementation candidates

- **rapidfuzz** (Python) — fast fuzzy string matching, C++ backend
- **tree-sitter** — for "what function is at line X?" structural queries
- **GitNexus** — for "is there payment logic near this symbol?"

Likely: rapidfuzz for text matching (90% of cases). tree-sitter/GitNexus as optional enrichment.

## Output format

```json
{
  "cite_id": "row-30",
  "claimed_line": 1724,
  "search_range": [1719, 1729],
  "max_score": 3.2,
  "best_match_line": 1722,
  "verdict": "WRONG",
  "evidence": "max fuzzy score 3.2% < 5% threshold across L1719–L1729"
}
```

## Separate from verifier-loop

This is a **standalone tool**. It does NOT integrate with the verifier-loop consensus/hash machinery. It's a pre-filter that produces evidence that verifiers (or humans) can consume.
