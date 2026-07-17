#!/usr/bin/env bats
# RED test #3 — parse-verdict.sh on an APPROVE transcript that also carries
# the consensus completion hash.
#
# Contract under test:
#   parse-verdict.sh <transcript-file>
#     -> { "verdict": "APPROVE", "completion_hash": "a1b2c3d4", "findings_count": 0 }
#
# The completion-hash line looks like:
#   "Completion hash: a1b2c3d4"   (8 hex chars; accept 8+ hex digits)
# Matched case-insensitively on the "completion hash:" label.
#
# This must FAIL until scripts/bench/parse-verdict.sh exists and is executable.

load test_helper

setup() {
  TRANSCRIPT_APPROVE="$(make_temp_file approve \
'Verifier review complete.

VERDICT: APPROVE

All DOD scenarios verified. No defects.

Consensus reached: 2/2 verifiers APPROVE.
Completion hash: a1b2c3d4
')"
}

teardown() {
  rm -f "$TRANSCRIPT_APPROVE"
}

@test "parse-verdict: APPROVE extracts the completion hash" {
  run "$(bench_script parse-verdict.sh)" "$TRANSCRIPT_APPROVE"
  echo "status=$status"
  echo "output=$output"

  [ "$status" -eq 0 ]

  echo "$output" | jq -e '
    .verdict           == "APPROVE"
    and .completion_hash == "a1b2c3d4"
    and .findings_count  == 0
  ' >/dev/null
}

@test "parse-verdict: completion_hash is a JSON string, not a number" {
  run "$(bench_script parse-verdict.sh)" "$TRANSCRIPT_APPROVE"
  echo "output=$output"

  # completion_hash must be a JSON string, not a bare number.
  echo "$output" | jq -e '.completion_hash | type == "string"' >/dev/null
}
