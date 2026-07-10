## 1. Config Schema Extension

- [x] 1.1 Add `hermes_profile: Option<String>` field to `Config` struct in `src/store/config.rs` with `#[serde(rename = "hermesProfile")]`
- [x] 1.2 Add validation in `load_config_in`: when `hermesProfile` is present and `backend != "hermes"`, fail with error naming the field
- [x] 1.3 Add unit tests: `hermesProfile` parses correctly, rejected for pi/acpx/custom backends, optional (backward compat)

## 2. Adapter Resolution

- [x] 2.1 Update `adapter_for("hermes")` in `src/acp/adapters.rs` to accept optional `profile: Option<&str>` parameter
- [x] 2.2 When `profile` is present, hermes templates become `hermes -p <profile> --mode json` and `hermes -p <profile> --session {sid} --mode json`
- [x] 2.3 Update `resolve_adapter()` in `src/bin/verifier_loop.rs` to pass `config.hermes_profile.as_deref()` to `adapter_for("hermes")`
- [x] 2.4 Add unit tests: hermes with profile renders correct templates, hermes without profile unchanged, pi/acpx ignore profile param

## 3. Spawn Layer Integration

- [x] 3.1 Verify spawn_round and spawn_resume use the resolved adapter (no changes needed if adapter resolution is correct)
- [x] 3.2 Add integration test: hermes spawn with profile includes `-p <profile>` in command

## 4. Documentation

- [x] 4.1 Update `AGENTS.md` with hermes profile config example
- [x] 4.2 Update `README.md` with hermes profile usage example
- [x] 4.3 Add config.json example showing `hermesProfile: "verifier"`

## 5. Coverage Gate

- [x] 5.1 Run `cargo llvm-cov --fail-under-lines 80` and ensure all new code meets coverage threshold
- [x] 5.2 Add tests for edge cases: empty profile string, profile with spaces, profile with special chars
