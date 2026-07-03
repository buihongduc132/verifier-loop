// tasks.md §10 — CLI wiring (clap) for both binaries.
// RED phase: written first, against tasks.md §10 + the CLI subcommand contract, BEFORE
// the `verifier-loop` bin wiring exists. The scaffold bin (src/bin/verifier_loop.rs)
// currently ignores all args and prints an identity line, so every assertion here is
// expected to FAIL until §10 GREEN lands.
//
// Scope of THIS test (wiring only — no spawn, no I/O):
//   * `verifier-loop --help` exits 0 and advertises NEW / RESUME.
//   * `verifier-verdict --help` exits 0 and advertises approve / reject (already wired in §7;
//     asserted here to guard against regressions during §10).
//   * Missing/invalid subcommands and missing required args exit non-zero with a usage hint.
//
// Strategy: assert_cmd against the cargo-built binaries. Fast, hermetic, no temp stores.

use assert_cmd::Command;
use predicates::prelude::*;

/// `verifier-loop --help` lists both subcommands and exits 0.
#[test]
fn verifier_loop_help_lists_subcommands() {
    let mut cmd = Command::cargo_bin("verifier-loop").unwrap();
    cmd.args(["--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("NEW").or(predicate::str::contains("new")))
        .stdout(predicate::str::contains("RESUME").or(predicate::str::contains("resume")));
}

/// `verifier-verdict --help` lists approve / reject and exits 0 (§7 regression guard).
#[test]
fn verifier_verdict_help_lists_subcommands() {
    let mut cmd = Command::cargo_bin("verifier-verdict").unwrap();
    cmd.args(["--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("approve"))
        .stdout(predicate::str::contains("reject"));
}

/// No subcommand at all → non-zero exit + a usage message on stderr.
#[test]
fn no_subcommand_exits_non_zero_with_usage() {
    let mut cmd = Command::cargo_bin("verifier-loop").unwrap();
    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("usage").or(predicate::str::contains("Usage")));
}

/// `NEW` with no goal argument → non-zero exit + usage.
#[test]
fn new_without_goal_arg_exits_non_zero() {
    let mut cmd = Command::cargo_bin("verifier-loop").unwrap();
    cmd.args(["NEW"]).assert().failure();
}

/// `RESUME` with no goalId → non-zero exit + usage.
#[test]
fn resume_without_goal_id_exits_non_zero() {
    let mut cmd = Command::cargo_bin("verifier-loop").unwrap();
    cmd.args(["RESUME"]).assert().failure();
}
