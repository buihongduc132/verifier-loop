## Context

Hermes supports isolated profiles via `hermes -p <profile>`. Each profile has its own config, skills, SOUL.md, and environment. The current hermes adapter template (`hermes --mode json`) does not expose the profile flag.

Real-world use case: a "verifier" profile with specific skills, tools, or system prompts tailored for verification tasks. Without profile support, all hermes verifiers use the default profile, losing role specialization.

## Goals / Non-Goals

**Goals:**
- Hermes adapter supports optional `profile` field in config
- Profile flag injected into spawn/resume templates when configured
- Backward compatible (existing configs without profile work unchanged)
- Validation: profile field rejected for non-hermes backends

**Non-Goals:**
- Per-verifier profile configuration (out of scope; requires per-verifier adapter change)
- Profile creation/management (handled by `hermes profile` subcommand)
- Profile-specific timeout/model overrides (each profile already has its own config)

## Decisions

### D1: Add `hermesProfile` field to Config struct

**Decision**: Add optional `hermesProfile: Option<String>` field to `Config` struct in `src/store/config.rs`. When present and `backend == "hermes"`, the hermes adapter templates include `-p <profile>`.

**Rationale**:
- Minimal schema change (additive, not breaking)
- Clear semantics: `hermesProfile` only valid for hermes backend
- Backward compatible: existing configs work unchanged

**Alternatives considered**:
- Add `profile` field to generic adapter → ambiguous (pi doesn't have profiles)
- Add `hermesArgs` array → over-engineered, only need profile flag

### D2: Template rendering at adapter resolution time

**Decision**: When `adapter_for("hermes")` is called and `config.hermesProfile` is present, the returned `Adapter` has templates `hermes -p <profile> --mode json` and `hermes -p <profile> --session {sid} --mode json`.

**Rationale**:
- Adapter resolution is a one-time cost at spawn
- Keeps `Adapter` struct simple (just spawn/resume strings)
- No runtime branching in spawn layer

**Alternatives considered**:
- Inject profile at spawn time → duplicates logic across spawn_round/spawn_resume
- Store profile in Adapter struct → breaks Adapter's generic nature

### D3: Validation: `hermesProfile` only valid for hermes backend

**Decision**: When `hermesProfile` is present and `backend != "hermes"`, config loading fails with a hard error naming the offending field.

**Rationale**:
- Fail-closed: prevents silent misconfiguration
- Clear error message helps users migrate
- No ambiguity about which backend supports profiles

**Alternatives considered**:
- Ignore `hermesProfile` for non-hermes backends → hides config errors
- Warn but allow → user confusion

## Risks / Trade-offs

**[Risk] Config schema complexity** → Mitigation: `hermesProfile` is optional; existing configs work unchanged. Documentation emphasizes it's hermes-only.

**[Risk] Profile name typos** → Mitigation: Hermes itself validates profile existence at runtime. Verifier-loop doesn't need to duplicate this validation.

**[Trade-off] Hermes-specific field in generic Config** → Chose simplicity over purity. Adding `hermesProfile` to Config is slightly impure (Config is backend-agnostic), but avoids a more complex adapter-specific config layer. Acceptable trade-off for minimal schema change.

## Migration Plan

1. Add `hermesProfile: Option<String>` field to `Config` struct
2. Add validation: `hermesProfile` only valid when `backend == "hermes"`
3. Update `adapter_for("hermes")` to accept optional profile parameter
4. Update `resolve_adapter()` to pass `config.hermesProfile` to hermes adapter
5. Add unit tests: profile template rendering, validation, backward compat
6. Update docs: `AGENTS.md`, `README.md` with hermes profile examples

**Rollback**: Remove `hermesProfile` field. Existing configs with `hermesProfile` would fail validation (fail-closed). No data migration needed.

## Open Questions

**Q1**: Should per-verifier profile support be added later (via `verifiers` array)?
- Current scope: no. This change is hermes-only, single profile for all verifiers. Per-verifier profiles require the `per-verifier-adapter` change first.
