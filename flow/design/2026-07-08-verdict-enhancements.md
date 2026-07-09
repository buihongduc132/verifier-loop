# Verdict Record Enhancements — Design Notes

> Date: 2026-07-08
> Status: design (explore-mode artifact, NOT implemented)
> Scope: Problem C — will be BAKED INTO verifier-loop (not separate tool)

## Current schema

```
VerdictRecord {
  status: APPROVE | REJECT | null
  notes: Option<String>         // REJECT only
  registeredAt: Option<String>
  signature: Option<String>     // signed regime
  pubkeyId: Option<String>
}

CompletionRecord {
  hash: "mmddyy-XXXXXXXX"
  fullDigest: "64-hex SHA-256"
  goalId, roundNumber, matchedAt
  matchingVerdicts: [{verifierId, registeredAt}, ...]
}
```

## Decision: Option A (verifier self-reports items)

NOT Option C (N declared upfront — N rarely known).
NOT Option B (orchestrator parses notes — too fragile).

## New fields

### VerdictRecord gains: `items` array

```json
{
  "status": "REJECT",
  "notes": "Rows 30, 47 have wrong line cites",
  "items": [
    {"id": "row-30", "passed": false},
    {"id": "row-31", "passed": true},
    {"id": "row-47", "passed": false},
    {"id": "row-48", "passed": true}
  ]
}
```

Design decisions:

1. **Each item**: `{id, passed: YES/NO}` — atomic unit
2. **Per-verifier, per-round**: each round gets its own set. Total countable across rounds.
3. **Duplication OK for now**: v1 and v2 may check same items independently. Dedup/consolidate later.
4. **Future: random sampling** — "query randomly X items from the pool" for targeted spot-checks
5. **Purpose of random query**: TBD (possible uses: targeted re-verification, statistical sampling of unchecked items, cross-verifier disagreement detection)

```
┌──────────────────────────────────────────────────────┐
│  PER-ITEM VERDICT LEDGER                             │
│                                                      │
│  Round 1:                                            │
│    v1 items: [row-30✗, row-31✓, row-47✗, row-48✓]  │
│    v2 items: [row-30✗, row-31✓, row-47✓, row-48✓]  │
│                                                      │
│  Round 2 (after fix):                                │
│    v1 items: [row-30✓, row-47✗]                     │
│    v2 items: [row-30✓, row-47✓]                     │
│                                                      │
│  Cross-verifier: row-47 disagreement → flag for     │
│  targeted re-check                                   │
│                                                      │
│  Total checked (all rounds): unique(item.id) count  │
│  Total wrong: unique(item.id where passed=false)    │
└──────────────────────────────────────────────────────┘
```

### CompletionRecord gains: aggregate metrics

```json
{
  "hash": "...",
  "fullDigest": "...",
  "totalItemsChecked": 87,
  "wrongItemsFound": 3,
  "accuracyRatio": 0.965,
  "roundsUsed": 5,
  "schemaVersion": 1
}
```

- `totalItemsChecked` = unique item IDs across all rounds × all matching verifiers
- `wrongItemsFound` = unique item IDs where ANY verifier reported `passed: false`
- `accuracyRatio` = 1 - (wrongItemsFound / totalItemsChecked)
- `roundsUsed` = round number where consensus was reached

### Schema versioning

`schemaVersion: Option<u32>` with `#[serde(default, skip_serializing_if = "Option::is_none")]` on both records. NOT part of signature canonical bytes or hash formula — advisory only. Old records deserialize to `None` (implicit v0). See: `flow/design/2026-07-08-schema-versioning-research.md`

## Hash formula impact

`items` array is NOT part of the hash input. Only {status, notes, registeredAt, goalId, verifierId, round} are signed. Metrics ride alongside, unsigned but visible.

This preserves backward compatibility: existing hashes remain valid. New records with items still produce the same hash as old records without items (for the same status/notes/ids).

## CLI changes

```
verifier-verdict approve --items '[{"id":"row-1","passed":true},...]'
verifier-verdict reject --notes "..." --items '[{"id":"row-1","passed":false},...]'

# --items is optional (backward-compatible: no items = current behavior)
# --items accepts JSON array or @file.json
```

## Deferred

- Dedup/consolidation across verifiers (v1.0 accepts duplication)
- Random item sampling/query (v1.0 stores items, query API TBD)
- Per-item consensus evaluation (currently binary APPROVE/REJECT on whole goal)
