#!/usr/bin/env bash
# Shared helpers for the bench harness bats tests (RED phase).
#
# These tests pin the contract of three scripts that will live in
# scripts/bench/:
#   - parse-verdict.sh <transcript-file>
#   - run-one.sh       <label> <goal-file> [pi_config_dir]
#   - compare.sh       <result1.json> <result2.json>
#
# Until those scripts exist + are executable, every test that calls them
# MUST fail (RED). The GREEN author implements the scripts to turn these
# red tests green WITHOUT editing the tests.

# Resolve the repo root from this test file's location (tests/bench/).
BENCH_ROOT="$(cd "${BATS_FILE_DIRECTORY:-"$BATS_TEST_DIRNAME"}/../.." && pwd)"
export BENCH_ROOT
export BENCH_SCRIPTS_DIR="$BENCH_ROOT/scripts/bench"

# Echo the absolute path of a bench script under test.
bench_script() {
  printf '%s\n' "$BENCH_SCRIPTS_DIR/$1"
}

# Normalize a single-line / multi-line JSON document to a canonical
# compact form so order-independent comparison works regardless of how
# the script chose to pretty-print.
normalize_json() {
  jq -c -S . "$1"
}

# Write the given string content to a fresh temp file and echo its path.
# Usage: make_temp_file <name> <content>
make_temp_file() {
  local name="$1" content="$2"
  local f
  f="$(mktemp -t "bats-bench-${name}.XXXXXX")"
  printf '%s' "$content" >"$f"
  printf '%s\n' "$f"
}
