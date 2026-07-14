## ADDED Requirements

### Requirement: A global `--json` flag selects machine-readable output on both binaries
Both `jewilo` (`verifier-loop`) and `jewije` (`verifier-verdict`) SHALL accept a top-level boolean flag `--json` (short form `-j`). When the flag is present, the binary SHALL emit exactly one JSON object on stdout representing the command's result envelope and SHALL NOT emit any of the legacy free-text success lines (`goalId: …`, bare `mmddyy-XXXXXXXX`, `Verdict registered`, multi-line rejection summaries) on stdout. When the flag is absent, output SHALL be byte-identical to the behavior before this change. The flag SHALL be a no-op with respect to on-disk artifacts, hash inputs, verdict semantics, signature verification, and exit codes.

#### Scenario: `--json` suppresses legacy stdout lines on a passing NEW round
- **WHEN** `jewilo NEW "<goal>" --json` is invoked against a resolvable store and the round reaches n/m consensus
- **THEN** stdout contains exactly one line that is a valid JSON object
- **AND** stdout does NOT contain the legacy strings `goalId:` or the bare short hash printed without JSON wrapping
- **AND** the process exits 0

#### Scenario: Default (no `--json`) output is byte-identical to before the change
- **WHEN** `jewilo NEW "<goal>"` is invoked without `--json` and the round reaches consensus
- **THEN** stdout first line is `goalId: <id>` and the last line is the bare short hash `mmddyy-XXXXXXXX`
- **AND** no JSON object appears on stdout

#### Scenario: `--json` is a no-op for on-disk completion.json
- **WHEN** the same goal+round is driven once with `--json` and once without
- **THEN** the written `completion.json` is byte-identical across both invocations
- **AND** both invocations compute the same `hash` and `fullDigest`

#### Scenario: `--json` is a no-op for exit code
- **WHEN** a round is rejected and `jewilo … --json` is invoked
- **THEN** the process exits non-zero (identical to the no-`--json` path)
- **AND** the JSON envelope's `ok` field is `false`

### Requirement: JSON envelope schema is stable and camelCased
Every `--json` output object SHALL conform to a single envelope with camelCase field names (matching the on-disk artifact convention): `ok` (boolean), `command` (string, one of `new`, `resume`, `recover`, `status`, `approve`, `reject`), `goalId` (string when known), `round` (number when known), `status` (string enum, one of `consensus-passed`, `rejected`, `recover-null-after-timeout`, `verdict-registered`, `cooldown-fallback`, `already-done` — the `already-done` value is emitted by `RECOVER` when the round already reached consensus), `hash` (short hash string when present), `fullDigest` (64-hex string when present), `needs` (string `recover` | `resume` | `done` when the round is decided), `report` (object, present only on `STATS`/`AUDIT` success/invalid — carries the stats/audit body; mutually exclusive with `status`), `rejection` (object with `rejectNotes` array, `nullVerifiers` array, `signatureFailures` array — present only on rejection), and `error` (string, present only when `ok` is false). Unknown fields SHALL be tolerated by consumers; producers SHALL NOT add fields to the hash inputs. The envelope SHALL always be a single root object (not an array, not NDJSON). The `STATS` and `AUDIT` success envelopes carry `report` and SHALL omit `status` (the report body replaces the status signal); the `AUDIT`-invalid envelope carries BOTH `report` and `error` plus `ok:false`.

#### Scenario: Consensus-passed envelope carries goalId, round, hash, fullDigest
- **WHEN** a NEW round reaches consensus under `--json`
- **THEN** the JSON object has `ok:true`, `command:"new"`, `goalId` set, `round:1`, `status:"consensus-passed"`, `hash` matching the printed short form, and `fullDigest` set
- **AND** the object has no `error` field

#### Scenario: Rejection envelope carries the rejection breakdown
- **WHEN** a round fails to reach n/m consensus under `--json`
- **THEN** the JSON object has `ok:false`, `command` reflecting the invoked subcommand, `status:"rejected"`, `round` set, and a `rejection` object
- **AND** `rejection.rejectNotes`, `rejection.nullVerifiers`, and `rejection.signatureFailures` are present as arrays

#### Scenario: Error envelope is single object on stdout
- **WHEN** `jewilo` encounters a fatal error (e.g. missing store, unreadable config) under `--json`
- **THEN** stdout contains exactly one JSON object with `ok:false` and an `error` string
- **AND** no partial or duplicate JSON object appears on stdout
- **AND** the process exits non-zero

### Requirement: `STATUS` conforms to the same envelope under `--json`
Because `STATUS` already emits a JSON body today, when `--json` is set the body SHALL be wrapped in the standard envelope: `ok:true`, `command:"status"`, `goalId`, `round`, `state`, and `needs` SHALL appear at the envelope top level (lifted from the legacy body), and `verdicts` SHALL be preserved. When `--json` is absent, `STATUS` output SHALL remain byte-identical to today (a bare JSON object without the envelope wrapper).

#### Scenario: `STATUS --json` wraps the body in the standard envelope
- **WHEN** `jewilo STATUS <goalId> --json` is invoked
- **THEN** stdout is one JSON object with `ok:true`, `command:"status"`, `goalId`, `round`, `state`, `needs`
- **AND** the `verdicts` array is preserved as a field of the object

#### Scenario: `STATUS` without `--json` is unchanged
- **WHEN** `jewilo STATUS <goalId>` is invoked without `--json`
- **THEN** stdout is the bare JSON object (round, state, needs, verdicts) byte-identical to before this change
- **AND** there is no `ok` / `command` envelope wrapper

### Requirement: `jewije` verdict registration emits a JSON envelope under `--json`
When `jewije approve` or `jewije reject --notes "…"` is invoked with `--json` and registration succeeds, stdout SHALL contain exactly one JSON object with `ok:true`, `command` (`approve` or `reject`), `goalId`, `verifierId`, `round`, `status:"verdict-registered"`. When registration fails under `--json`, stdout SHALL contain one JSON object with `ok:false`, an `error` string, and the relevant `goalId`/`verifierId`/`round` when known; the process exits non-zero as today. The legacy `Verdict registered` line SHALL NOT appear on stdout when `--json` is set.

#### Scenario: `jewije approve --json` success envelope
- **WHEN** `jewije approve --json` is invoked inside a V* process with valid identity env and registration succeeds
- **THEN** stdout contains exactly one JSON object with `ok:true`, `command:"approve"`, `goalId`, `verifierId`, `round`, `status:"verdict-registered"`
- **AND** stdout does NOT contain the legacy string `Verdict registered`
- **AND** the process exits 0

#### Scenario: `jewije reject --notes "..." --json` carries status
- **WHEN** `jewije reject --notes "broken" --json` is invoked and registration succeeds
- **THEN** the JSON object has `ok:true`, `command:"reject"`, `status:"verdict-registered"`
- **AND** the process exits 0

#### Scenario: `jewije` failure under `--json` emits error envelope and exits non-zero
- **WHEN** `jewije reject` is invoked with empty notes (notes-required error) under `--json`
- **THEN** stdout contains one JSON object with `ok:false` and an `error` field describing the notes-required failure
- **AND** the process exits non-zero
- **AND** the human-readable error text is NOT also printed on stdout

### Requirement: Human-readable diagnostics stay on stderr under `--json`
Under `--json`, the binary SHALL keep human-oriented diagnostic messages (cooldown notices, recoverable-round hints, captured V* stderr previews, prompt-budget warnings, tracing init notes) on stderr and SHALL NOT place them inside or alongside the stdout JSON object. The stdout JSON object SHALL carry the structured equivalents needed by automation (e.g. `status:"cooldown-fallback"` and `hash` set to the fallback hash) so a consumer parsing stdout never needs to parse stderr.

#### Scenario: Cooldown fallback is reflected in the envelope, not only stderr
- **WHEN** `jewilo NEW "<goal>" --json` is invoked while the store is in cooldown (>3 unhealthy runs in the last hour)
- **THEN** the stdout JSON object has `ok:true`, `status:"cooldown-fallback"`, and `hash` equal to the fallback `<mmddyy>-ffffff`
- **AND** the human-readable cooldown notice remains on stderr only

#### Scenario: Recover-still-null is reflected in the envelope
- **WHEN** `jewilo RECOVER <goalId> --json` times out with null slots still present
- **THEN** the stdout JSON object has `ok:false`, `status:"recover-null-after-timeout"`, and the offending null slots surfaced in a structured field
- **AND** the human-readable guidance remains on stderr only

### Requirement: `--json` output is deterministic for the same logical result
For a given command outcome (consensus-passed / rejected / verdict-registered / error), the set of envelope fields present SHALL be deterministic; arrays SHALL be sorted by a stable key (`rejectNotes` and `nullVerifiers` and `signatureFailures` sorted by verifierId). Two runs with identical logical inputs SHALL produce envelope objects whose field order may differ but whose parsed content is identical. The envelope SHALL be emitted exactly once per process invocation regardless of how many internal phases ran.

#### Scenario: Rejection arrays are sorted by verifierId
- **WHEN** a round rejects with notes from `v3` and `v1`
- **THEN** the `rejection.rejectNotes` array is ordered by verifierId ascending
- **AND** the same inputs always yield the same parsed envelope content

#### Scenario: Exactly one JSON object regardless of internal phase count
- **WHEN** a RESUME round spawns m=5 verifiers and reaches consensus under `--json`
- **THEN** stdout contains exactly one JSON object (one root), not one object per verifier
- **AND** no extra JSON object is printed for the snapshot/render/spawn phases
