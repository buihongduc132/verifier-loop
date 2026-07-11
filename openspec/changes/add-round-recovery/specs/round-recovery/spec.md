## ADDED Requirements

### Requirement: RECOVER is a separate top-level command for cross-process round recovery
The CLI SHALL provide a `RECOVER <goalId>` top-level command, distinct from `RESUME`. After jewilo was killed or interrupted mid-round, `RECOVER` SHALL wait for already-emitted verdicts from the round's verifier slots and re-evaluate consensus WITHOUT spawning new processes, killing processes, re-rendering the verifier prompt, or re-capturing the working-tree snapshot. The per-round signing secrets are NOT persisted; `RECOVER` SHALL NOT mint, read, or require them.

#### Scenario: RECOVER harvests a verdict written by a still-running orphan
- **WHEN** round 1 has one APPROVE slot and one null slot whose orphan verifier process is still running, and that orphan then writes a signed APPROVE
- **AND** the user runs `jewilo RECOVER <goalId>`
- **THEN** RECOVER polls the slots, observes both verdicts non-null, re-evaluates consensus, and writes `completion.json` with a completion hash
- **AND** RECOVER did NOT spawn, kill, re-render, or re-capture

#### Scenario: RECOVER never spawns or kills
- **WHEN** `jewilo RECOVER <goalId>` runs against a round with a null slot
- **THEN** no new verifier process is spawned
- **AND** no process is killed
- **AND** the null slot's verdict is left untouched (fail-closed)

#### Scenario: RECOVER never re-renders or re-captures
- **WHEN** `jewilo RECOVER <goalId>` runs
- **THEN** no `initial-prompt.txt` is written for the current round
- **AND** no working-tree snapshot is captured

### Requirement: RECOVER reuses the consensus layer unchanged
`RECOVER` SHALL evaluate consensus by calling the existing `consensus::evaluate` over the round's verdict slots. The verdict file is the resumption contract: a verdict written by a verifier process during recovery counts exactly as it would during a normal round (it must be signed and verify against the slot's pinned pubkey). The completion-hash formula and inputs SHALL be identical to a normal passing round.

#### Scenario: RECOVER completion hash matches a normal round's
- **WHEN** round 1 reaches 2/2 APPROVE via RECOVER (both verdicts written by the original orphan processes)
- **THEN** the printed short hash and `completion.json` `fullDigest` are computed from the same inputs as a normal round (salt + goalId + goalSignature + round + canonical matchingVerdicts + matchedAt + receiptHead)

### Requirement: RECOVER degrades honestly to RESUME N+1 for dead-but-null slots
If after polling up to the configured timeout a slot is still null (its orphan died or never finished), `RECOVER` SHALL NOT mint a fresh key, SHALL NOT fabricate a verdict, and SHALL NOT write `completion.json`. It SHALL exit non-zero with user-visible guidance telling the user to run `RESUME N+1` for fresh slots and fresh keys.

#### Scenario: Dead-but-null slot falls through to RESUME guidance
- **WHEN** round 1 has one APPROVE slot and one null slot whose orphan has died, and `RECOVER` polls until the timeout elapses with the slot still null
- **THEN** RECOVER exits non-zero
- **AND** stderr contains guidance referencing `RESUME`
- **AND** no `completion.json` is written

#### Scenario: A round already decided (no nulls) fails closed without consensus
- **WHEN** round 1 has every slot non-null but the APPROVE count is below `n` (e.g. one APPROVE, one REJECT, n=2)
- **AND** the user runs `jewilo RECOVER <goalId>`
- **THEN** RECOVER determines the round is decided but failed without waiting for the timeout
- **AND** no `completion.json` is written
- **AND** the rejection is surfaced

### Requirement: RECOVER takes an exclusive goal lock for its full duration
`RECOVER` SHALL acquire an exclusive advisory lock on `goals/<goalId>/.lock` for its entire duration (polling included). A second concurrent `NEW`, `RESUME`, or `RECOVER` on the same goal SHALL exit non-zero with a "goal busy" message.

#### Scenario: Concurrent RECOVER is rejected
- **WHEN** one `jewilo RECOVER <goalId>` is running (holding the lock) and a second `jewilo RECOVER <goalId>` is invoked
- **THEN** the second invocation exits non-zero
- **AND** its stderr names the goal as busy

### Requirement: RECOVER warns when nothing needs recovering
When `RECOVER` is invoked on a round that has already reached consensus (`completion.json` exists), the CLI SHALL emit a warning directing the user to `RESUME N+1` and SHALL exit successfully without polling.

#### Scenario: RECOVER on a complete round is a no-op with guidance
- **WHEN** `completion.json` already exists for the current round and the user runs `jewilo RECOVER <goalId>`
- **THEN** the CLI prints a warning referencing `RESUME`
- **AND** exits 0
- **AND** does not poll
