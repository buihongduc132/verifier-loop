## ADDED Requirements

### Requirement: Hermes adapter supports optional profile field
The hermes adapter configuration SHALL support an optional `profile` field. When present, the spawn and resume templates SHALL include the `-p <profile>` flag. When absent, templates SHALL remain unchanged (backward compatible).

#### Scenario: Hermes adapter with profile configured
- **WHEN** config contains `{"backend": "hermes", "hermesProfile": "verifier"}`
- **THEN** hermes spawn template becomes `hermes -p verifier --mode json`
- **AND** hermes resume template becomes `hermes -p verifier --session {sid} --mode json`

#### Scenario: Hermes adapter without profile (backward compatible)
- **WHEN** config contains `{"backend": "hermes"}` with no `hermesProfile`
- **THEN** hermes spawn template remains `hermes --mode json`
- **AND** hermes resume template remains `hermes --session {sid} --mode json`

### Requirement: Profile field rejected for non-hermes adapters
The `hermesProfile` field SHALL only be valid when `backend` is `"hermes"`. If `hermesProfile` is present with any other backend (`pi`, `acpx`, or custom), config loading SHALL fail with a hard error.

#### Scenario: Profile field with pi backend rejected
- **WHEN** config contains `{"backend": "pi", "hermesProfile": "verifier"}`
- **THEN** config loading fails with an error indicating `hermesProfile` is only valid for hermes backend

#### Scenario: Profile field with custom backend rejected
- **WHEN** config contains `{"backend": "custom", "hermesProfile": "verifier"}`
- **THEN** config loading fails with an error indicating `hermesProfile` is only valid for hermes backend

### Requirement: Profile flag injected into spawn command
When hermes adapter has `profile` configured, the spawn orchestrator SHALL inject `-p <profile>` into the hermes command argv. The profile value SHALL be passed as a single argument (no shell splitting).

#### Scenario: Spawn command includes profile flag
- **WHEN** hermes adapter has `profile: "verifier"` and spawn is invoked
- **THEN** the spawned command is `hermes -p verifier --mode json`
- **AND** the profile value is passed as a single argv element (not shell-split)

#### Scenario: Resume command includes profile flag
- **WHEN** hermes adapter has `profile: "verifier"` and resume is invoked with SID `abc-123`
- **THEN** the resumed command is `hermes -p verifier --session abc-123 --mode json`
