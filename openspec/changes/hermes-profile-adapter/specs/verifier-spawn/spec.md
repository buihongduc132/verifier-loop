## MODIFIED Requirements

### Requirement: Pluggable backends share one ACP parser
Built-in adapters (pi, hermes, acpx) SHALL each provide spawn/resume command templates but share the same ACP stream parser. Custom adapters SHALL be configurable via `config.json` with spawn/resume templates and a transport field, and MUST also conform to the ACP output format. The hermes adapter SHALL support an optional `profile` field that injects `-p <profile>` into spawn/resume templates.

#### Scenario: pi backend spawns
- **WHEN** config `backend: "pi"` (no `hermesProfile`)
- **THEN** all verifiers spawn using `pi --mode json` and resume using `pi --session <sid> --mode json`

#### Scenario: hermes backend with profile spawns
- **WHEN** config `backend: "hermes", hermesProfile: "verifier"`
- **THEN** all verifiers spawn using `hermes -p verifier --mode json`
- **AND** resume uses `hermes -p verifier --session <sid> --mode json`

#### Scenario: hermes backend without profile spawns
- **WHEN** config `backend: "hermes"` (no `hermesProfile`)
- **THEN** all verifiers spawn using `hermes --mode json`
- **AND** resume uses `hermes --session <sid> --mode json`

#### Scenario: custom backend is configured
- **WHEN** config `backend: "custom"` with spawn/resume templates
- **THEN** the CLI renders the templates and parses their output with the shared ACP parser
