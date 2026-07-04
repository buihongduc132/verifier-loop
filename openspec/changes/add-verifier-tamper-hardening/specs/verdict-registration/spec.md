## MODIFIED Requirements

### Requirement: Verifier identity is read from env, not arguments
`verifier-verdict` SHALL resolve goalId, verifierId, and round from `VERIFIER_LOOP_GOAL_ID`, `VERIFIER_LOOP_VERIFIER_ID`, and `VERIFIER_LOOP_ROUND`. It SHALL NOT trust a goalId passed as a CLI argument over the env var, preventing a verifier from writing to another slot. Additionally, `verifier-verdict` SHALL resolve the per-verifier signing key from `VERIFIER_LOOP_VERIFIER_SECRET` (hex Ed25519 signing key injected by the spawn layer); a verdict write without a valid signing key matching the slot's pinned `verifier-pubkey.json` SHALL fail closed with `VerdictError::Unauthenticated`.

#### Scenario: Verdict writes to the env-derived slot
- **WHEN** `VERIFIER_LOOP_GOAL_ID=abc`, `VERIFIER_LOOP_VERIFIER_ID=v1`, `VERIFIER_LOOP_ROUND=1`, `VERIFIER_LOOP_VERIFIER_SECRET=<hex>` (matching the pinned pubkey) and `verifier-verdict approve` runs
- **THEN** the verdict is written to the `abc / round-1 / v1` slot regardless of any conflicting argument
- **AND** the written `verdict.json` carries a `signature` verifying against the pinned pubkey

#### Scenario: Missing signing secret fails closed
- **WHEN** `VERIFIER_LOOP_VERIFIER_SECRET` is unset or empty and `verifier-verdict approve` runs
- **THEN** the invocation exits non-zero with `VerdictError::Unauthenticated`
- **AND** no `verdict.json` is written and no receipt-log entry is appended

#### Scenario: Signing secret that does not match the pinned pubkey fails closed
- **WHEN** `VERIFIER_LOOP_VERIFIER_SECRET` is set to a key whose pubkey does not equal the slot's pinned `verifier-pubkey.json`
- **THEN** the invocation exits non-zero with `VerdictError::Unauthenticated`
- **AND** the slot is not modified

### Requirement: First verdict is final
The first non-null verdict written to a slot SHALL be immutable. A subsequent `verifier-verdict` invocation for an already-final slot SHALL fail with `VerdictError::AlreadyFinal`. First-fill of a `null` placeholder slot SHALL additionally require a valid signature from the slot's pinned pubkey; an unauthenticated caller SHALL fail with `VerdictError::Unauthenticated` regardless of slot state.

#### Scenario: First verdict registers and is signed
- **WHEN** V* `v1` registers APPROVE in a `null` slot with its pinned secret
- **THEN** the signed APPROVE verdict is written atomically
- **AND** a receipt-log entry is appended

#### Scenario: Second verdict for the same slot is rejected
- **WHEN** V* `v1` (or any caller) attempts to register a verdict in a slot that already holds a non-null verdict
- **THEN** the invocation fails with `VerdictError::AlreadyFinal`
- **AND** the original verdict is unchanged

#### Scenario: Null-slot first-fill without the pinned secret fails closed
- **WHEN** a caller without `VERIFIER_LOOP_VERIFIER_SECRET` (or with a non-matching secret) attempts to fill a `null` slot
- **THEN** the invocation fails with `VerdictError::Unauthenticated`
- **AND** the slot remains `{status: null}`
