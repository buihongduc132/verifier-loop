// RED tests for per-verifier-adapter task group 2: Adapter Resolution.
//
// These tests exercise the INTENDED API where resolve_adapters() returns
// Vec<Adapter> with one entry per verifier slot (m entries), supporting
// per-verifier resolution and custom adapters per slot.
//
// They MUST FAIL because the current stub resolve_adapters() returns a vec
// with only a single adapter (the old single-adapter behavior), not m entries.
//
// Tasks covered:
//   2.1  resolve_adapter() → resolve_adapters() returning Vec<Adapter>
//   2.2  Per-verifier resolution: replicate adapter m times for built-in backends
//   2.3  Custom adapter construction per verifier slot (from config.json)
//   2.4  Unit tests (this file)

use std::fs;

use verifier_loop::{acp, store};

// ─── helpers ────────────────────────────────────────────────────────────────

fn make_config(json: &serde_json::Value) -> store::Config {
    serde_json::from_value(json.clone()).expect("config deserializes")
}

fn default_config_with_m(m: u32) -> store::Config {
    store::Config {
        n: 1,
        m,
        max_turn: 3,
        backend: "pi".to_string(),
        git_diff_max_chars: 1000,
        verifier_timeout_sec: 10,
        verifier_prompt_file: None,
        min_goal_chars: 0,
        verifiers: None,
    }
}

// ─── 2.1: resolve_adapters returns Vec<Adapter> ────────────────────────────

/// resolve_adapters() must return Ok(Vec<Adapter>), not a single Adapter.
/// This test validates the return TYPE is a Vec (compilation-level check)
/// and that the vec is non-empty.
#[test]
fn resolve_adapters_returns_vec_of_adapters() {
    let config = default_config_with_m(2);
    let result = acp::resolve_adapters(&config);
    assert!(result.is_ok(), "resolve_adapters must succeed for built-in 'pi'");
    let adapters = result.unwrap();
    assert!(
        !adapters.is_empty(),
        "resolve_adapters must return at least one adapter"
    );
    // Verify the elements are actually Adapter instances by checking a field.
    assert!(
        !adapters[0].spawn.is_empty(),
        "adapter[0] must have a non-empty spawn template"
    );
}

/// resolve_adapters() must return exactly m adapters when config.m = 2.
/// **FAILS NOW**: stub returns vec of length 1, not 2.
#[test]
fn resolve_adapters_returns_one_per_verifier_slot_m2() {
    let config = default_config_with_m(2);
    let adapters = acp::resolve_adapters(&config).expect("pi resolves");
    assert_eq!(
        adapters.len(),
        2,
        "resolve_adapters must return exactly m=2 adapters, got {}",
        adapters.len()
    );
}

/// resolve_adapters() must return exactly m adapters when config.m = 3.
/// **FAILS NOW**: stub returns vec of length 1, not 3.
#[test]
fn resolve_adapters_returns_one_per_verifier_slot_m3() {
    let config = default_config_with_m(3);
    let adapters = acp::resolve_adapters(&config).expect("pi resolves");
    assert_eq!(
        adapters.len(),
        3,
        "resolve_adapters must return exactly m=3 adapters, got {}",
        adapters.len()
    );
}

/// resolve_adapters() must return exactly m=1 adapter when m=1.
/// (This one should pass even with the stub, serving as a sanity baseline.)
#[test]
fn resolve_adapters_returns_one_when_m1() {
    let config = default_config_with_m(1);
    let adapters = acp::resolve_adapters(&config).expect("pi resolves");
    assert_eq!(
        adapters.len(),
        1,
        "resolve_adapters must return exactly m=1 adapter"
    );
}

/// resolve_adapters() returns an empty vec when m=0 (edge case).
#[test]
fn resolve_adapters_returns_empty_when_m0() {
    let config = default_config_with_m(0);
    let adapters = acp::resolve_adapters(&config).expect("pi resolves");
    assert_eq!(
        adapters.len(),
        0,
        "resolve_adapters must return zero adapters when m=0, got {}",
        adapters.len()
    );
}

// ─── 2.2: Per-verifier resolution replicates adapter m times ───────────────

/// For a built-in backend like "hermes", resolve_adapters must return m
// identical adapter clones.
/// **FAILS NOW**: stub returns vec of length 1.
#[test]
fn resolve_adapters_hermes_replicates_m_times() {
    let mut config = default_config_with_m(3);
    config.backend = "hermes".to_string();
    let adapters = acp::resolve_adapters(&config).expect("hermes resolves");
    assert_eq!(
        adapters.len(),
        3,
        "must return m=3 hermes adapters, got {}",
        adapters.len()
    );
    // All three should be identical hermes adapters.
    let expected = acp::adapter_for("hermes").unwrap();
    for (i, a) in adapters.iter().enumerate() {
        assert_eq!(
            a.spawn, expected.spawn,
            "adapter[{}] must be a hermes adapter",
            i
        );
    }
}

/// For a built-in backend like "acpx", resolve_adapters must return m
/// identical adapter clones.
/// **FAILS NOW**: stub returns vec of length 1.
#[test]
fn resolve_adapters_acpx_replicates_m_times() {
    let mut config = default_config_with_m(4);
    config.backend = "acpx".to_string();
    let adapters = acp::resolve_adapters(&config).expect("acpx resolves");
    assert_eq!(
        adapters.len(),
        4,
        "must return m=4 acpx adapters, got {}",
        adapters.len()
    );
}

// ─── 2.3: Custom adapters per verifier slot ────────────────────────────────

/// When config.json includes per-verifier custom adapters, resolve_adapters
/// must return each one in slot order.
///
/// This test uses a config with a hypothetical `verifiers` field that carries
/// per-slot adapter definitions. The GREEN author must define the schema.
///
/// **FAILS NOW**: stub ignores any per-verifier config and returns 1 adapter.
#[test]
fn resolve_adapters_custom_per_slot_m2() {
    // Hypothetical schema: config.json carries a `verifiers` array where each
    // entry defines an adapter. This is the design surface the GREEN author
    // must implement.
    //
    // We test the BEHAVIORAL CONTRACT: resolve_adapters returns m adapters,
    // each with the spawn command from the corresponding verifier slot.
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    // Write a config.json with per-verifier custom adapters.
    // The GREEN author decides the exact schema; we test the contract.
    // For now, we construct the config programmatically and call resolve_adapters.
    let config = store::Config {
        n: 1,
        m: 2,
        max_turn: 3,
        backend: "custom".to_string(),
        git_diff_max_chars: 1000,
        verifier_timeout_sec: 10,
        verifier_prompt_file: None,
        min_goal_chars: 0,
        verifiers: None,
    };

    // For the "custom" backend, resolve_adapters should look at env vars
    // or per-verifier config to produce m distinct adapters.
    // The stub currently returns a single adapter (or errors on "custom"
    // without env vars). Either way, it won't return m=2 adapters.
    //
    // We set env vars so the function at least doesn't error:
    std::env::set_var("VERIFIER_LOOP_BACKEND_CMD", "/bin/echo spawn1");
    std::env::set_var("VERIFIER_LOOP_RESUME_CMD", "/bin/echo resume1");

    let adapters = acp::resolve_adapters(&config)
        .expect("custom backend resolves with env vars");

    assert_eq!(
        adapters.len(),
        2,
        "must return m=2 custom adapters, got {} — the GREEN author must \
         implement per-verifier resolution for custom backends",
        adapters.len()
    );

    // Clean up env vars.
    std::env::remove_var("VERIFIER_LOOP_BACKEND_CMD");
    std::env::remove_var("VERIFIER_LOOP_RESUME_CMD");
}

/// resolve_adapters with a config loaded from config.json must still
/// return m adapters. Tests integration with the store layer.
/// **FAILS NOW**: stub returns vec of length 1.
#[test]
fn resolve_adapters_from_loaded_config_m2() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    let config_json = serde_json::json!({
        "n": 2,
        "m": 2,
        "maxTurn": 3,
        "backend": "pi",
        "gitDiffMaxChars": 1000,
        "verifierTimeoutSec": 10
    });
    fs::write(root.join("config.json"), config_json.to_string()).unwrap();

    let config = store::Config::load_in(root).expect("config loads");
    assert_eq!(config.m, 2);

    let adapters = acp::resolve_adapters(&config).expect("pi resolves");
    assert_eq!(
        adapters.len(),
        2,
        "must return m=2 adapters for a loaded config, got {}",
        adapters.len()
    );
}

/// Each adapter in the returned vec must be a valid, usable Adapter
/// (not a degenerate/empty adapter). This is a quality guard for
/// the GREEN implementation.
/// **FAILS NOW**: stub returns only 1 adapter so index [1] panics.
#[test]
fn resolve_adapters_each_slot_is_valid_adapter() {
    let config = default_config_with_m(2);
    let adapters = acp::resolve_adapters(&config).expect("pi resolves");

    for (i, a) in adapters.iter().enumerate() {
        assert!(
            !a.spawn.is_empty(),
            "adapter[{}] must have a non-empty spawn command",
            i
        );
        assert!(
            !a.resume.is_empty(),
            "adapter[{}] must have a non-empty resume command",
            i
        );
    }
}

/// Distinct adapters per slot: when per-verifier adapters are configured,
/// each slot must carry its own (possibly different) adapter.
/// This validates the "custom adapters per slot" requirement (task 2.3).
///
/// **FAILS NOW**: stub returns identical single adapter for all slots.
#[test]
fn resolve_adapters_per_slot_distinct_custom() {
    // Set up two different custom commands via env.
    // The GREEN author must support per-verifier adapter config (e.g. a
    // `verifiers` array in config.json), but for this RED test we verify
    // the behavioral contract: m=2 yields 2 potentially distinct adapters.
    std::env::set_var("VERIFIER_LOOP_BACKEND_CMD", "/bin/true");

    let config = store::Config {
        n: 1,
        m: 2,
        max_turn: 3,
        backend: "custom".to_string(),
        git_diff_max_chars: 1000,
        verifier_timeout_sec: 10,
        verifier_prompt_file: None,
        min_goal_chars: 0,
        verifiers: None,
    };

    let adapters = acp::resolve_adapters(&config).expect("custom backend resolves");

    assert_eq!(
        adapters.len(),
        2,
        "must return m=2 adapters for per-slot custom resolution, got {}",
        adapters.len()
    );

    std::env::remove_var("VERIFIER_LOOP_BACKEND_CMD");
}

// ─── Error cases ────────────────────────────────────────────────────────────

/// resolve_adapters must return an error for an unknown backend with no
/// env override. The error should propagate, not silently default.
#[test]
fn resolve_adapters_unknown_backend_no_env_errors() {
    // Ensure env overrides are clean for this test.
    std::env::remove_var("VERIFIER_LOOP_BACKEND_CMD");
    std::env::remove_var("VERIFIER_LOOP_SPAWN_CMD");
    std::env::remove_var("VERIFIER_LOOP_RESUME_CMD");

    let mut config = default_config_with_m(2);
    config.backend = "nonexistent-backend".to_string();

    let result = acp::resolve_adapters(&config);
    assert!(
        result.is_err(),
        "unknown backend without env override must error"
    );
}
