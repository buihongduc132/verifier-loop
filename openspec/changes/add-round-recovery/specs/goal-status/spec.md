## ADDED Requirements

### Requirement: STATUS emits machine-readable goal state
The CLI SHALL provide a `STATUS <goalId>` top-level command that emits a JSON object describing the goal's current round, lifecycle state, what action it needs, and the per-slot verdicts. `STATUS` SHALL NOT acquire the goal lock (it is a read-only probe that must not block on a long-running round) and SHALL NOT spawn, kill, or mutate any state.

#### Scenario: STATUS shape
- **WHEN** the user runs `jewilo STATUS <goalId>` for a goal in round 2 with v1 APPROVE and v2 null and no completion
- **THEN** stdout is a single JSON object containing `goalId`, `round`, `state`, `needs`, and a `slots` array
- **AND** each slot has an `id` and a `verdict` field

### Requirement: STATUS needs-field tells an outer agent what to do next
The `needs` field SHALL be one of `"done"`, `"recover"`, or `"resume"`, derived as follows:
- `"done"` — a `completion.json` exists for the current round.
- `"recover"` — no completion AND at least one slot is null (a live orphan may still emit a verdict).
- `"resume"` — no completion AND every slot is non-null (the round is decided but failed; a fresh round is required).

#### Scenario: needs=done after consensus
- **WHEN** round 1 has `completion.json`
- **THEN** `needs` is `"done"`

#### Scenario: needs=recover with a null slot
- **WHEN** round 1 has one APPROVE, one null, and no completion
- **THEN** `needs` is `"recover"`

#### Scenario: needs=resume when the round is decided but failed
- **WHEN** round 1 has one APPROVE and one REJECT (all non-null, below n), no completion
- **THEN** `needs` is `"resume"`

### Requirement: STATUS state-field reflects the round lifecycle
The `state` field SHALL be one of `"new"`, `"in_progress"`, `"consensus_pass"`, or `"consensus_fail"`:
- `"consensus_pass"` — `completion.json` exists.
- `"consensus_fail"` — every slot non-null AND below `n`.
- `"in_progress"` — at least one slot null and no completion.
- `"new"` — the round directory or slots do not yet exist (before the first spawn).

#### Scenario: in_progress with a null slot
- **WHEN** round 1 has one APPROVE and one null and no completion
- **THEN** `state` is `"in_progress"`

#### Scenario: consensus_pass after a hash
- **WHEN** `completion.json` exists
- **THEN** `state` is `"consensus_pass"`
