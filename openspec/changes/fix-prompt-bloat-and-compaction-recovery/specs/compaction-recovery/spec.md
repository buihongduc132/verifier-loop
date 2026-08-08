## ADDED Requirements

### Requirement: Compaction event is detected in the verifier session stream
The ACP stream parser SHALL detect `{"type":"compaction",...}` events emitted by the backend. When a compaction event is observed, it SHALL be recorded in the verifier's `meta.json` with a `compactionObserved: true` flag and the token counts from the event (`tokensBefore`, `tokensAfter`).

#### Scenario: Compaction event is recorded
- **WHEN** a verifier session emits `{"type":"compaction","tokensBefore":255106}`
- **THEN** the verifier's `meta.json` records `compactionObserved: true` and `tokensBefore: 255106`

#### Scenario: No compaction leaves flag false
- **WHEN** a verifier session completes with no compaction event
- **THEN** `meta.json` records `compactionObserved: false` (or omits the flag)

### Requirement: Post-compaction session is auto-resumed to harvest the verdict
When a compaction event is observed AND the verifier session ends without an `agent_end` event (i.e., terminated mid-investigation) AND no verdict was registered, the orchestrator SHALL auto-resume the same sid with a minimal verdict-nudge prompt to harvest the verdict. This recovery SHALL occur at most once per slot per round (no infinite compaction loop). If the post-recovery session also ends without a verdict, the slot fails-closed to null.

#### Scenario: Compaction then exit triggers recovery resume
- **WHEN** a verifier session emits a compaction event and then exits with no `agent_end` and no verdict
- **THEN** the orchestrator resumes the same sid with a verdict-nudge prompt once
- **AND** if the resumed session registers a verdict, it is used for consensus

#### Scenario: Recovery resume also fails leaves null
- **WHEN** the post-compaction recovery resume also ends without a verdict
- **THEN** the slot's `verdict.json` remains `status: null` (fail-closed)
- **AND** no further recovery resume is attempted for that slot in that round

#### Scenario: Compaction followed by successful agent_end does not trigger recovery
- **WHEN** a verifier session emits a compaction event but then continues and reaches `agent_end`
- **THEN** the orchestrator does not perform a recovery resume (the session self-recovered)

### Requirement: Recovery resume uses a minimal nudge prompt
The recovery resume prompt SHALL be small (target < 2KB) and SHALL instruct the verifier that compaction occurred, its prior investigation is preserved in the session, and it must register its verdict immediately via the `verifier-verdict` CLI. The prompt SHALL NOT re-include the goal, diff, or policy (these are already in the session context).

#### Scenario: Recovery nudge is minimal
- **WHEN** a recovery resume is triggered
- **THEN** the resume prompt is under 2KB
- **AND** the prompt does not re-embed the goal, git diff, or policy text
