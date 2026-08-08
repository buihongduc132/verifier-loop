## MODIFIED Requirements

### Requirement: Verifiers are spawned in parallel via ACP JSON stream
On `NEW` and on each `RESUME` round, the CLI SHALL spawn all `m` verifier sessions concurrently as separate ACP-process invocations, each with injected identity env vars. The prompt SHALL be delivered out-of-band (stdin pipe or goal-file) per the adapter's declared transport — never inlined into argv. The spawns MUST be non-blocking relative to one another; the CLI blocks only at the gather barrier after all are launched.

#### Scenario: All verifiers start at once
- **WHEN** `verifier-loop NEW "goal"` runs with config `m: 3`
- **THEN** three verifier processes are launched concurrently
- **AND** none blocks the launch of another

#### Scenario: Identity env vars are injected per spawn
- **WHEN** a verifier process is spawned
- **THEN** its environment contains `VERIFIER_LOOP_GOAL_ID`, `VERIFIER_LOOP_VERIFIER_ID` (v1, v2, ...), and `VERIFIER_LOOP_ROUND`

#### Scenario: Prompt is delivered via adapter transport, not argv
- **WHEN** a verifier is spawned with any transport
- **THEN** the rendered prompt bytes never appear in the child's argv
- **AND** the prompt reaches the verifier through the transport declared by the adapter

### Requirement: Pluggable backends share one ACP parser
Built-in adapters (pi, hermes, acpx) SHALL each provide spawn/resume command templates and a `transport` declaration, but share the same ACP stream parser. Custom adapters SHALL be configurable via `config.json` with spawn/resume templates, a transport field, and a JSON flag, and MUST also conform to the ACP output format. Custom adapter templates MUST NOT use the `{prompt}` placeholder.

#### Scenario: pi backend spawns via stdin
- **WHEN** config `backend: "pi"`
- **THEN** spawn uses `pi --mode json` with the prompt piped to stdin
- **AND** resume uses `pi --session <sid> --mode json` with the prompt piped to stdin

#### Scenario: custom backend is configured with goal-file transport
- **WHEN** config `backend: "custom"` with `transport: "goal-file"` and a `{goalFile}` spawn template
- **THEN** the CLI renders the template by substituting the tempfile path for `{goalFile}`
- **AND** parses the verifier's output with the shared ACP parser
