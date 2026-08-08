## MODIFIED Requirements

### Requirement: Verifiers are spawned in parallel via ACP JSON stream
On `NEW` and on each `RESUME` round, the CLI SHALL spawn all `m` verifier sessions concurrently as separate ACP-process invocations, each with injected identity env vars. Each verifier slot SHALL use its own resolved adapter (from the `verifiers` array if present, or the global `backend` field / default `pi` otherwise). The spawns MUST be non-blocking relative to one another; the CLI blocks only at the gather barrier after all are launched.

#### Scenario: All verifiers start at once
- **WHEN** `verifier-loop NEW "goal"` runs with config `m: 3`
- **THEN** three verifier processes are launched concurrently
- **AND** none blocks the launch of another

#### Scenario: Identity env vars are injected per spawn
- **WHEN** a verifier process is spawned
- **THEN** its environment contains `VERIFIER_LOOP_GOAL_ID`, `VERIFIER_LOOP_VERIFIER_ID` (v1, v2, ...), and `VERIFIER_LOOP_ROUND`

#### Scenario: Mixed backends spawn concurrently
- **WHEN** config contains `{"m": 2, "verifiers": [{"adapter": "pi"}, {"adapter": "hermes"}]}`
- **THEN** v1 is spawned via `pi --mode json` (stdin transport)
- **AND** v2 is spawned via `hermes --mode json` (stdin transport)
- **AND** both are launched concurrently before any is awaited

### Requirement: Pluggable backends share one ACP parser
Built-in adapters (pi, hermes, acpx) SHALL each provide spawn/resume command templates but share the same ACP stream parser. Custom adapters SHALL be configurable via `config.json` with spawn/resume templates and a transport field, and MUST also conform to the ACP output format. Per-verifier adapter resolution SHALL use the `verifiers` array when present, falling back to the global `backend` field.

#### Scenario: pi backend spawns
- **WHEN** config `backend: "pi"` (no `verifiers` array)
- **THEN** all verifiers spawn using `pi --mode json` and resume using `pi --session <sid> --mode json`

#### Scenario: per-verifier backend spawns
- **WHEN** config `verifiers: [{"adapter": "pi"}, {"adapter": "hermes"}]`
- **THEN** v1 spawns using `pi --mode json`
- **AND** v2 spawns using `hermes --mode json`

#### Scenario: custom backend is configured
- **WHEN** config `verifiers: [{"adapter": "custom", "spawn": "my-tool run", "resume": "my-tool resume {sid}", "transport": "stdin"}]`
- **THEN** the CLI renders the templates and parses their output with the shared ACP parser
