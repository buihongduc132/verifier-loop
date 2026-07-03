// Integration tests live here (tasks.md §1 scaffolding).
//
// Behavioural tests are added group-by-group under strict TDD:
//   §2 salt/config, §3 goal-lifecycle, §4 acp parser, §5/§6 spawn+reuse,
//   §7 verdict, §8 consensus+hash, §9 prompt, §10 e2e.
//
// Each RED test is authored by a DIFFERENT fresh teammate than its GREEN implementation,
// per the project's TDD discipline. This file is the harness anchor; concrete test files
// (e.g. `goal_lifecycle.rs`, `verdict.rs`, `consensus.rs`, `prompt.rs`, `acp_parser.rs`,
// `spawn_orchestrator.rs`, `e2e.rs`) are added alongside their respective groups.

#[test]
fn harness_compiles() {
    // Smoke anchor: the test crate links against the lib + both binaries build.
    assert_eq!(verifier_loop::VERSION, env!("CARGO_PKG_VERSION"));
}
