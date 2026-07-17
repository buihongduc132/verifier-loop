#!/usr/bin/env bats
# RED test #1 — parse-verdict.sh on a transcript with NO verdict line and
# NO completion hash line.
#
# Contract under test:
#   parse-verdict.sh <transcript-file>
#     -> prints JSON: { "verdict": "NONE", "completion_hash": null, "findings_count": 0 }
#
# This must FAIL until scripts/bench/parse-verdict.sh exists and is executable.

load test_helper

setup() {
  TRANSCRIPT_NO_VERDICT="$(make_temp_file no-verdict \
'spawning verifier v1 for goal 005a5f70-64b9-40e7-a3b1-bd2eb449b71e
round 1 consensus: pending
verifier v1 produced output but did not emit a verdict line
some random log noise: nothing to see here
')"
}

teardown() {
  rm -f "$TRANSCRIPT_NO_VERDICT"
}

@test "parse-verdict: no verdict line yields verdict=NONE and zero counts" {
  run "$(bench_script parse-verdict.sh)" "$TRANSCRIPT_NO_VERDICT"
  echo "status=$status"
  echo "output=$output"

  # Script must exit 0 even when there is nothing to parse.
  [ "$status" -eq 0 ]

  echo "$output" | jq -e '
    .verdict           == "NONE"
    and .completion_hash == null
    and .findings_count  == 0
  ' >/dev/null
}
