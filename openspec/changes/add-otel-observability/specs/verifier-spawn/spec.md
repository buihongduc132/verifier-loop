## MODIFIED Requirements

### Requirement: Verifiers are spawned in parallel via ACP JSON stream
jewilo SHALL spawn `m` V* verifier processes in parallel as real ACP-JSON CLI agents. For each V*, the spawn layer SHALL mint a fresh Ed25519 keypair, persist the public key to `<slot>/verifier-pubkey.json` BEFORE launching the process, and inject the signing key into the V* process environment as `VERIFIER_LOOP_VERIFIER_SECRET` (hex). The identity env vars (`VERIFIER_LOOP_GOAL_ID`, `VERIFIER_LOOP_VERIFIER_ID`, `VERIFIER_LOOP_ROUND`) SHALL continue to be injected as before. The signing key SHALL NOT be persisted to disk by jewilo. The spawn layer SHALL additionally inject the goal's `traceId` into each V* process environment as `VERIFIER_LOOP_TRACE_ID` (hex), so that `jewije` calls made inside the V* process correlate to the spawning round; the `traceId` SHALL be the value resolved for the goal (see `trace-export` capability). The spawn layer SHALL open a per-verifier tracing span around each V* launch and gather, carrying `verifierId`, `round`, and `traceId` as structured fields.

#### Scenario: Spawn mints keypair and pins pubkey before launch
- **WHEN** jewilo spawns V* `v1` for goal `abc` round `1`
- **THEN** `<store>/goals/abc/rounds/1/v1/verifier-pubkey.json` is written BEFORE the V* process is launched
- **AND** the V* process env contains `VERIFIER_LOOP_VERIFIER_SECRET=<hex>` whose pubkey matches the pinned file
- **AND** no file under `<store>` contains the signing key

#### Scenario: Each V* gets a distinct secret
- **WHEN** jewilo spawns `v1` and `v2` for the same goal and round
- **THEN** the two pinned pubkeys are distinct
- **AND** each V* process env contains its own `VERIFIER_LOOP_VERIFIER_SECRET`, neither matching the other's pinned pubkey

#### Scenario: Stub backend receives the secret env automatically
- **WHEN** a stub/custom backend is configured (deterministic e2e)
- **THEN** the spawn layer injects `VERIFIER_LOOP_VERIFIER_SECRET` into the stub process env exactly as for a real `pi` backend
- **AND** the stub's `jewije approve` invocation (inheriting the env) produces a signed verdict that verifies against the pinned pubkey

#### Scenario: Spawn propagates the goal traceId to every V* process
- **WHEN** jewilo spawns `v1` and `v2` for goal `abc` round `1` whose persisted `traceId` is `T`
- **THEN** both V* process environments contain `VERIFIER_LOOP_TRACE_ID=T`
- **AND** the value equals the contents of `<store>/goals/abc/trace-id`

#### Scenario: Per-verifier gather span records the timeout-vs-exit outcome
- **WHEN** jewilo spawns V* `v1` and the run is killed by `verifierTimeoutSec`
- **THEN** the per-verifier gather span for `v1` records `timed_out=true`
- **AND** a sibling span for a normally-exiting `v2` records `timed_out=false`
- **AND** both spans are children of the round span carrying the same `traceId`
