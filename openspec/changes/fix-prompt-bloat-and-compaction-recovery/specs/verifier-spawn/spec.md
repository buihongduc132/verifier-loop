## MODIFIED Requirements

### Requirement: Verifier timeout leaves a null verdict
Each verifier spawn SHALL be subject to `verifierTimeoutSec` (default 1800). On timeout the process SHALL be killed and the pre-created `verdict.json` left at `status: null`. Before declaring a slot null due to timeout, the orchestrator SHALL attempt verdict enforcement (see "Verdict is enforced after child exit" requirement) up to `maxTurn`.

#### Scenario: Timeout produces null verdict after enforcement exhausted
- **WHEN** a verifier runs longer than `verifierTimeoutSec` without registering a verdict AND verdict-enforcement nudges are exhausted
- **THEN** the process is killed
- **AND** its `verdict.json` remains `status: null`

## ADDED Requirements

### Requirement: Verdict is enforced after child exit
After `gather()` reaps a verifier child, if no `verdict.json` was written OR `verdict.json` has `status: null` AND the slot's `turnsUsed` is less than `maxTurn`, the orchestrator SHALL re-prompt the same session (sid reuse via resume) with a minimal verdict-nudge prompt instructing the verifier to register its verdict immediately via the `verifier-verdict` CLI. This SHALL repeat up to `maxTurn - turnsUsed` times per slot per round. A slot is only declared null when nudges are exhausted or the session cannot be resumed.

#### Scenario: Missing verdict triggers nudge
- **WHEN** a verifier child exits with no verdict.json AND `turnsUsed: 1`, `maxTurn: 3`
- **THEN** the orchestrator resumes the same sid with a verdict-nudge prompt
- **AND** if a verdict is registered after the nudge, it is used for consensus

#### Scenario: Nudge exhausted leaves null
- **WHEN** a verifier child exits with no verdict AND nudges are exhausted (`turnsUsed >= maxTurn`)
- **THEN** the slot's `verdict.json` remains `status: null` (fail-closed)

#### Scenario: Verdict present skips nudge
- **WHEN** a verifier child exits and `verdict.json` has a non-null status
- **THEN** the orchestrator does not nudge and proceeds to consensus check

### Requirement: Default prompt template ends with explicit verdict command
The default prompt template and default resume prompt template SHALL end with an explicit fenced bash block showing the exact `verifier-verdict approve --notes "..."` and `verifier-verdict reject --notes "..."` invocation pattern. The final instruction SHALL be a command, not prose.

#### Scenario: Template contains explicit verdict command
- **WHEN** the default template is rendered
- **THEN** the rendered prompt contains a fenced bash block with `verifier-verdict approve` and `verifier-verdict reject` examples
