## ADDED Requirements

### Requirement: Per-verifier signing keypair minted at spawn
The spawn layer SHALL mint a fresh Ed25519 keypair per V* immediately before launching the V* process. The public key SHALL be persisted to `<slot>/verifier-pubkey.json` (`{pubkey, mintedAt}`). The signing key SHALL be injected into V*'s environment as `VERIFIER_LOOP_VERIFIER_SECRET` (hex) and SHALL NOT be persisted to disk by jewilo.

#### Scenario: Spawn writes pinned pubkey and injects secret
- **WHEN** jewilo spawns V* `v1` for goal `abc` round `1`
- **THEN** `<store>/goals/abc/rounds/1/v1/verifier-pubkey.json` exists with `{pubkey: <hex>, mintedAt: <iso>}` BEFORE the V* process is launched
- **AND** the V* process environment contains `VERIFIER_LOOP_VERIFIER_SECRET=<hex>` matching the pinned pubkey
- **AND** no file under `<store>` contains the signing key

#### Scenario: Keypair is fresh per verifier slot
- **WHEN** jewilo spawns `v1` and `v2` for the same goal and round
- **THEN** the two `verifier-pubkey.json` files contain distinct `pubkey` values
- **AND** each V*'s `VERIFIER_LOOP_VERIFIER_SECRET` matches its own pinned pubkey and not the other's

### Requirement: Pinned pubkey is immutable for the slot lifetime
Once `verifier-pubkey.json` is written for a slot, any subsequent attempt to overwrite it (by jewilo, jewije, or any process) SHALL fail closed. Consensus evaluation and verdict verification SHALL trust ONLY the originally pinned pubkey.

#### Scenario: Overwrite of pinned pubkey is rejected
- **WHEN** a process writes a new `verifier-pubkey.json` to an existing slot
- **THEN** jewilo/jewije treat the slot's pinned pubkey as the original (mtime-earliest) entry
- **AND** a verdict signed by the replacement key fails signature verification at consensus

### Requirement: Missing pinned pubkey fails closed
If a verdict slot lacks `verifier-pubkey.json` (e.g. a pre-change goal dir or a tampered slot), any attempt to register a verdict in that slot SHALL fail with `VerdictError::Unauthenticated`. Consensus evaluation SHALL treat the slot's verdict as untrusted (not matching).

#### Scenario: Pre-change goal dir cannot receive new verdicts
- **WHEN** `jewije approve` runs against a slot with no `verifier-pubkey.json`
- **THEN** the invocation exits non-zero with an `Unauthenticated` error
- **AND** no `verdict.json` is written

### Requirement: Honest same-box limitation is documented
The repository SHALL ship a `THREAT-MODEL.md` and a README section stating plainly that same-box jewilo raises the bar to "must possess the per-verifier spawn-time secret" but cannot prevent a process with `~/.verifier-loop/` write access from forging if it can also read the spawned secret out of V*'s env; true prevention requires out-of-process V* on a separate host.

#### Scenario: THREAT-MODEL.md exists and names the limitation
- **WHEN** an operator reads `THREAT-MODEL.md`
- **THEN** the document names (a) the per-verifier secret deterrent, (b) the same-box fundamental limit, and (c) the out-of-process V* requirement for true prevention
