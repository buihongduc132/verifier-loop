## ADDED Requirements

### Requirement: Verifier prompt is blind to consensus state
The rendered verifier prompt SHALL NOT include the round number, any other verifier's verdict, the n/m configuration, or the completion hash. V* SHALL receive only its own identity, the goal, optional context, optional fix/prev-notes, and a frozen artifact snapshot.

#### Scenario: Round number is hidden
- **WHEN** a verifier prompt is rendered for round 3
- **THEN** the rendered text does not contain the value "3" in any round-indicator position or template variable other than those explicitly opted in

#### Scenario: Other verdicts are hidden
- **WHEN** V2 has already REJECTed and V1's prompt is rendered in a later round
- **THEN** V1's prompt does not contain V2's verdict or notes (unless `{{prevNotes}}` is used, which only references V1's own prior notes)

### Requirement: Frozen artifact snapshot is captured at spawn
At spawn time the CLI SHALL capture `cwd`, `git status --porcelain`, file edit times, and `git diff` truncated to `gitDiffMaxChars` (default 10000). This snapshot is frozen for the round and rendered into every verifier's prompt.

#### Scenario: Diff is truncated
- **WHEN** the git diff exceeds `gitDiffMaxChars`
- **THEN** the rendered `{{gitDiff}}` is truncated to that many characters with an indicator that truncation occurred

#### Scenario: Snapshot is consistent within a round
- **WHEN** two verifiers are spawned in the same round
- **THEN** both receive byte-identical artifact snapshots

### Requirement: Initial prompt is persisted per verifier
The fully rendered prompt sent to each verifier SHALL be written to `rounds/<round>/<verifierId>/initial-prompt.txt` before the spawn, forming part of the trust trail.

#### Scenario: Initial prompt is stored
- **WHEN** a verifier is spawned
- **THEN** its rendered initial prompt is written to `initial-prompt.txt` in its slot

### Requirement: Resume prompt may include previous notes via template
The resume prompt template (`verifierResumePromptTemplate`) MAY include `{{fixNotes}}` (A's `--fix` text) and `{{prevNotes}}` (this verifier's own prior-round notes). If a template omits these variables, V* does not see them. There SHALL be no separate boolean flag; the template IS the configuration.

#### Scenario: Resume template includes fix notes
- **WHEN** `verifierResumePromptTemplate` contains `{{fixNotes}}` and A passed `--fix "fixed X"`
- **THEN** the rendered resume prompt contains "fixed X"

#### Scenario: Resume template omits previous notes
- **WHEN** `verifierResumePromptTemplate` does not contain `{{prevNotes}}`
- **THEN** the rendered resume prompt contains no previous-round notes for this verifier

### Requirement: Default templates bake in verifier policy
When `verifierPromptTemplate` or `verifierResumePromptTemplate` is null, the CLI SHALL use a baked-in default that includes the verifier detective policy text (sourced from the verifier-loop skill) and the standard review instructions.

#### Scenario: Null template uses baked-in default
- **WHEN** config has `verifierPromptTemplate: null`
- **THEN** the rendered round-1 prompt uses the baked-in default including the verifier policy

### Requirement: Env vars are injectable into templates
Templates SHALL support `{{process.env.*}}` interpolation so operators can pass auxiliary environment variables into the verifier prompt without hardcoding.

#### Scenario: Env var is interpolated
- **WHEN** the template contains `{{process.env.TICKET_URL}}` and `TICKET_URL` is set in the environment
- **THEN** the rendered prompt contains the ticket URL value
