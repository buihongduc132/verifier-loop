## MODIFIED Requirements

### Requirement: Verdict is registered via a separate deterministic CLI

Verifiers SHALL register their verdict exclusively by invoking the `verifier-verdict` (jewije) CLI. The `approve` subcommand accepts an OPTIONAL `--notes` (short alias `-n`); the `reject` subcommand REQUIRES a non-empty `--notes`. When `--notes` is supplied to either subcommand and is non-empty after trimming, the notes SHALL be stored verbatim (trimmed) on `verdict.json`. When `--notes` is omitted on `approve`, or supplied as empty/whitespace on `approve`, the verdict is written with `notes: null` (the key is absent from the JSON). The CLI SHALL locate the target verdict file via the `VERIFIER_LOOP_*` env vars and write `verdict.json` atomically. There MUST be no pattern, keyword, or regex matching on verifier output to infer a verdict.

#### Scenario: Approve without notes writes a bare APPROVE
- **WHEN** a verifier runs `verifier-verdict approve`
- **THEN** its `verdict.json` is written with `status: APPROVE`, `registeredAt` set, and no `notes` key
- **AND** the CLI prints "Verdict registered" and exits 0

#### Scenario: Approve with notes stores the notes
- **WHEN** a verifier runs `verifier-verdict approve --notes "All 10 DoD reqs verified"`
- **THEN** its `verdict.json` is written with `status: APPROVE`, `notes: "All 10 DoD reqs verified"`, and `registeredAt` set
- **AND** the CLI prints "Verdict registered" and exits 0

#### Scenario: Approve with empty notes normalizes to no notes
- **WHEN** a verifier runs `verifier-verdict approve --notes "   "`
- **THEN** its `verdict.json` is written with `status: APPROVE` and no `notes` key (whitespace-only notes are treated as absent)
- **AND** the CLI prints "Verdict registered" and exits 0

#### Scenario: Approve with short alias works
- **WHEN** a verifier runs `verifier-verdict approve -n "evidence"`
- **THEN** the notes are stored identically to `--notes "evidence"`

#### Scenario: Reject requires notes (unchanged)
- **WHEN** a verifier runs `verifier-verdict reject --notes "issue 1: missing test"`
- **THEN** its `verdict.json` is written with `status: REJECT` and the notes
- **AND** the CLI prints "Verdict registered" and exits 0

#### Scenario: Reject without notes is refused (unchanged)
- **WHEN** a verifier runs `verifier-verdict reject` with no `--notes`
- **THEN** no verdict is written
- **AND** the CLI exits non-zero with an error stating notes are required

### Requirement: Signed approve binds notes into the canonical bytes

When `verifier-verdict approve --notes "..."` is invoked on a slot with a pinned verifier pubkey (signed regime), the supplied notes SHALL be included in the canonical record bytes that the Ed25519 signature covers. A subsequent alteration of the stored notes SHALL invalidate the signature (hash mismatch). Verification (`verify_record`) SHALL canonicalize `notes.as_deref()` identically to registration, so an approve-with-notes verdict verifies against its pinned pubkey without code change.

#### Scenario: Signed approve with notes verifies
- **WHEN** a verifier runs `verifier-verdict approve --notes "evidence"` on a pinned slot with the correct secret
- **THEN** the verdict record carries a `signature` that verifies over the canonical bytes including the notes
- **AND** `verify_record` against the pinned pubkey returns Ok

#### Scenario: Tampering with approve notes invalidates the signature
- **WHEN** an attacker edits the `notes` field of a signed APPROVE record on disk
- **THEN** `verify_record` returns `BadSignature`
