#!/usr/bin/env bats
# RED test #2 — parse-verdict.sh on a REJECT transcript with three findings.
#
# Contract under test:
#   parse-verdict.sh <transcript-file>
#     -> { "verdict": "REJECT", "completion_hash": null, "findings_count": 3 }
#
# A "finding" is a body line beginning with `D<n>` (e.g. D1 BLOCKER, D2 MAJOR,
# D3 MINOR). The verifier-prompt template emits exactly this numbering, so the
# parser counts the `^D[0-9]+` lines.
#
# This must FAIL until scripts/bench/parse-verdict.sh exists and is executable.

load test_helper

setup() {
  # The header line uses the spelling/spacing the verifier prompt emits:
  #   "VERDICT: REJECT" (matched case-insensitively).
  TRANSCRIPT_REJECT="$(make_temp_file reject \
'Verifier review complete.

VERDICT: REJECT

D1 BLOCKER: src/foo.ts:42 null dereference on empty input — violates spec scenario 1.
D2 MAJOR: missing test for the empty-changes JSON path; spec scenario 2 uncovered.
D3 MINOR: error message string drifts from spec literal by one trailing space.
')"
}

teardown() {
  rm -f "$TRANSCRIPT_REJECT"
}

@test "parse-verdict: REJECT counts three findings" {
  run "$(bench_script parse-verdict.sh)" "$TRANSCRIPT_REJECT"
  echo "status=$status"
  echo "output=$output"

  [ "$status" -eq 0 ]

  echo "$output" | jq -e '
    .verdict           == "REJECT"
    and .completion_hash == null
    and .findings_count  == 3
  ' >/dev/null
}

@test "parse-verdict: findings_count is a JSON number, not a string" {
  run "$(bench_script parse-verdict.sh)" "$TRANSCRIPT_REJECT"
  echo "output=$output"

  # findings_count must be a JSON number, not a quoted string ("3" is wrong).
  echo "$output" | jq -e '.findings_count | type == "number"' >/dev/null
}
