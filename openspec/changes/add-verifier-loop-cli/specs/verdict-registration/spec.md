## ADDED Requirements

### Requirement: Verdict is registered via a separate deterministic CLI
Verifiers SHALL register their verdict exclusively by invoking the `verifier-verdict` (jewije) CLI: `approve` or `reject --notes "..."`. The CLI SHALL locate the target verdict file via the `VERIFIER_LOOP_*` env vars and write `verdict.json` atomically. There MUST be no pattern, keyword, or regex matching on verifier output to infer a verdict.

#### Scenario: Approve writes a verdict
- **WHEN** a verifier runs `verifier-verdict approve`
- **THEN** its `verdict.json` is written with `status: APPROVE` and the registered timestamp
- **AND** the CLI prints "Verdict registered" and exits 0

#### Scenario: Reject requires notes
- **WHEN** a verifier runs `verifier-verdict reject --notes "issue 1: missing test"`
- **THEN** its `verdict.json` is written with `status: REJECT` and the notes
- **AND** the CLI prints "Verdict registered" and exits 0

#### Scenario: Reject without notes is refused
- **WHEN** a verifier runs `verifier-verdict reject` with no `--notes`
- **THEN** no verdict is written
- **AND** the CLI exits non-zero with an error stating notes are required

### Requirement: First verdict is final
The first verdict registered for a given (goalId, round, verifierId) SHALL be the final value. Any subsequent attempt to register a verdict for the same slot SHALL be rejected without altering the stored verdict.

#### Scenario: Second verdict attempt is rejected
- **WHEN** a verdict already exists with `status: APPROVE` and `verifier-verdict reject` is called for the same slot
- **THEN** the stored verdict remains `APPROVE`
- **AND** the CLI exits non-zero with an error stating the verdict is already final

### Requirement: Verdict file is pre-created as null
At spawn time the CLI SHALL pre-create each verifier's `verdict.json` with `status: null`. A null status after the gather barrier SHALL be treated as no-pass (fail-closed), never silently promoted to APPROVE.

#### Scenario: Forgotten verdict stays null and fails the round
- **WHEN** a verifier exits without calling `verifier-verdict`
- **THEN** its `verdict.json` remains `status: null`
- **AND** the round is evaluated as not passing

### Requirement: Verifier identity is read from env, not arguments
`verifier-verdict` SHALL resolve goalId, verifierId, and round from `VERIFIER_LOOP_GOAL_ID`, `VERIFIER_LOOP_VERIFIER_ID`, and `VERIFIER_LOOP_ROUND`. It SHALL NOT trust a goalId passed as a CLI argument over the env var, preventing a verifier from writing to another slot.

#### Scenario: Verdict writes to the env-derived slot
- **WHEN** `VERIFIER_LOOP_GOAL_ID=abc`, `VERIFIER_LOOP_VERIFIER_ID=v1`, `VERIFIER_LOOP_ROUND=1` and `verifier-verdict approve` runs
- **THEN** the verdict is written to the `abc / round-1 / v1` slot regardless of any conflicting argument
