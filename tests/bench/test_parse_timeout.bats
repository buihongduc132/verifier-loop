#!/usr/bin/env bats
# RED test #4 — parse-verdict.sh on an EMPTY transcript (e.g. jewilo timed out
# or crashed before producing any output).
#
# Contract under test:
#   parse-verdict.sh <empty-file>
#     -> { "verdict": "NONE", "completion_hash": null, "findings_count": 0 }
#     -> exits 0 (must NOT crash on empty input)
#
# This must FAIL until scripts/bench/parse-verdict.sh exists and is executable.

load test_helper

setup() {
  # Genuinely empty file — zero bytes.
  TRANSCRIPT_EMPTY="$(mktemp -t bats-bench-empty.XXXXXX)"
  # intentionally no write -> 0 bytes
}

teardown() {
  rm -f "$TRANSCRIPT_EMPTY"
}

@test "parse-verdict: empty input does not crash and yields NONE" {
  # Sanity: the fixture really is empty.
  [ ! -s "$TRANSCRIPT_EMPTY" ]

  run "$(bench_script parse-verdict.sh)" "$TRANSCRIPT_EMPTY"
  echo "status=$status"
  echo "output=$output"

  # Empty input is a valid "no verdict" state — must exit 0.
  [ "$status" -eq 0 ]

  echo "$output" | jq -e '
    .verdict           == "NONE"
    and .completion_hash == null
    and .findings_count  == 0
  ' >/dev/null
}
