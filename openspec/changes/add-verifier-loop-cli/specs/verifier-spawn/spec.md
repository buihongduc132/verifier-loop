## ADDED Requirements

### Requirement: Verifiers are spawned in parallel via ACP JSON stream
On `NEW` and on each `RESUME` round, the CLI SHALL spawn all `m` verifier sessions concurrently as separate ACP-process invocations (e.g. `pi -p "<prompt>" --mode json`), each with injected identity env vars. The spawns MUST be non-blocking relative to one another; the CLI blocks only at the gather barrier after all are launched.

#### Scenario: All verifiers start at once
- **WHEN** `verifier-loop NEW "goal"` runs with config `m: 3`
- **THEN** three verifier processes are launched concurrently
- **AND** none blocks the launch of another

#### Scenario: Identity env vars are injected per spawn
- **WHEN** a verifier process is spawned
- **THEN** its environment contains `VERIFIER_LOOP_GOAL_ID`, `VERIFIER_LOOP_VERIFIER_ID` (v1, v2, ...), and `VERIFIER_LOOP_ROUND`

### Requirement: ACP stream is parsed to capture SID and final output
The CLI SHALL parse the ACP JSON event stream (shared parser across all built-in backends). The session ID SHALL be extracted from the first `{"type":"session","id":"..."}` line for resume use. The verifier's final assistant message SHALL be captured from the `agent_end` event into `final-output.txt`.

#### Scenario: SID is captured for resume
- **WHEN** a verifier process emits `{"type":"session","id":"abc-123"}`
- **THEN** the SID `abc-123` is recorded in the verifier's `meta.json` for later resume

#### Scenario: Final output is captured
- **WHEN** a verifier process emits `{"type":"agent_end",...}`
- **THEN** the final assistant message is written to `rounds/<round>/<verifierId>/final-output.txt`

### Requirement: Session reuse up to maxTurn, then fresh spawn
On `RESUME`, a verifier whose `turnsUsed` is less than `maxTurn` SHALL be resumed via `--session <sid>` on the same SID. A verifier that has reached `maxTurn` SHALL be freshly spawned with a new SID, and the prior SID archived. v1 refreshes all verifiers together per round.

#### Scenario: Reused session continues on same SID
- **WHEN** `RESUME` runs and V1 has `turnsUsed: 1`, `maxTurn: 3`
- **THEN** V1 is resumed with `pi --session <v1-sid> -p "..." --mode json`
- **AND** the `VERIFIER_LOOP_ROUND` env var reflects the new round while `VERIFIER_LOOP_VERIFIER_ID` stays `v1`

#### Scenario: Exhausted session is freshly spawned
- **WHEN** `RESUME` runs and V1 has `turnsUsed: 3`, `maxTurn: 3`
- **THEN** V1 is spawned fresh with a new SID
- **AND** the prior V1 SID is archived under its originating round directory

### Requirement: Pluggable backends share one ACP parser
Built-in adapters (pi, hermes, acpx) SHALL each provide spawn/resume command templates but share the same ACP stream parser. Custom adapters SHALL be configurable via `config.json` with spawn/resume templates and a JSON flag, and MUST also conform to the ACP output format.

#### Scenario: pi backend spawns
- **WHEN** config `backend: "pi"`
- **THEN** spawn uses `pi -p "<prompt>" --mode json` and resume uses `pi --session <sid> -p "<prompt>" --mode json`

#### Scenario: custom backend is configured
- **WHEN** config `backend: "custom"` with spawn/resume templates
- **THEN** the CLI renders the templates and parses their output with the shared ACP parser

### Requirement: Verifier timeout leaves a null verdict
Each verifier spawn SHALL be subject to `verifierTimeoutSec` (default 1800). On timeout the process SHALL be killed and the pre-created `verdict.json` left at `status: null`.

#### Scenario: Timeout produces null verdict
- **WHEN** a verifier runs longer than `verifierTimeoutSec` without registering a verdict
- **THEN** the process is killed
- **AND** its `verdict.json` remains `status: null`
