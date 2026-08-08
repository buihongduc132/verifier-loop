## ADDED Requirements

### Requirement: Config supports per-verifier adapter array
The `config.json` SHALL support an optional `verifiers` array where each element specifies an `adapter` field. When present, the array length MUST equal `m` (verifier count). Each element's `adapter` value SHALL be a valid backend key (`pi`, `hermes`, `acpx`, or a custom adapter key).

#### Scenario: Heterogeneous backend configuration
- **WHEN** `config.json` contains `{"m": 2, "verifiers": [{"adapter": "pi"}, {"adapter": "hermes"}]}`
- **THEN** config loads successfully
- **AND** verifier index 0 uses the pi adapter
- **AND** verifier index 1 uses the hermes adapter

#### Scenario: Verifiers array length must match m
- **WHEN** `config.json` contains `{"m": 3, "verifiers": [{"adapter": "pi"}, {"adapter": "hermes"}]}`
- **THEN** config loading fails with an error indicating `verifiers` length (2) does not match `m` (3)

#### Scenario: Verifiers array with custom adapter
- **WHEN** `config.json` contains `{"m": 2, "verifiers": [{"adapter": "pi"}, {"adapter": "custom", "spawn": "my-tool run", "resume": "my-tool resume {sid}", "transport": "stdin"}]}`
- **THEN** config loads successfully
- **AND** verifier index 1 uses the custom adapter with the specified templates

### Requirement: Backward-compatible backend field
The existing `backend` field SHALL remain valid. When `backend` is present and `verifiers` is absent, all `m` verifiers SHALL use the same backend. When both are present, `verifiers` takes precedence and `backend` is ignored (with a warning). When neither is present, the default backend (`pi`) SHALL be used.

#### Scenario: Legacy backend field still works
- **WHEN** `config.json` contains `{"m": 2, "backend": "hermes"}`
- **THEN** config loads successfully
- **AND** both verifier slots use the hermes adapter

#### Scenario: Verifiers takes precedence over backend
- **WHEN** `config.json` contains `{"m": 2, "backend": "pi", "verifiers": [{"adapter": "hermes"}, {"adapter": "acpx"}]}`
- **THEN** config loads successfully
- **AND** a warning is emitted indicating `backend` is ignored when `verifiers` is present
- **AND** verifier index 0 uses hermes, index 1 uses acpx

#### Scenario: Default backend when neither specified
- **WHEN** `config.json` contains `{"m": 2}` with no `backend` or `verifiers`
- **THEN** config loads successfully
- **AND** both verifier slots use the default `pi` adapter

### Requirement: Adapter resolution per verifier index
The spawn orchestrator SHALL resolve the adapter for each verifier slot independently. Verifier index `i` SHALL use `verifiers[i].adapter` when the array is present, or `backend` (or default `pi`) when absent.

#### Scenario: Mixed backend spawn round
- **WHEN** `spawn_round` is called with config `{"m": 2, "verifiers": [{"adapter": "pi"}, {"adapter": "hermes"}]}`
- **THEN** verifier v1 is spawned using the pi adapter template
- **AND** verifier v2 is spawned using the hermes adapter template
- **AND** both are launched concurrently (non-blocking)

#### Scenario: Resume uses per-verifier adapter
- **WHEN** `spawn_resume` is called for round 2 with the same config
- **THEN** v1 resume uses the pi adapter template
- **AND** v2 resume uses the hermes adapter template
