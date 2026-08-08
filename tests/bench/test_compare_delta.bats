#!/usr/bin/env bats
# RED test #5 — compare.sh on two synthetic run result files.
#
# Contract under test:
#   compare.sh <result1.json> <result2.json>
#     -> first line of stdout is a JSON object with the delta fields:
#          {
#            "time_delta_sec":     <number = r1.wall - r2.wall>,
#            "time_delta_pct":     <number, 2dp>,
#            "verdict_agreement":  <bool = r1.verdict == r2.verdict>,
#            "findings_delta":     <number = r2.findings - r1.findings>
#          }
#        followed (optionally) by a human-readable table.
#
# NUMERIC CONTRACT (pinned by this test):
#   time_delta_sec = 100 - 60            = 40
#   time_delta_pct = (r1-r2)/r2 * 100    = (40/60)*100 = 66.67
#                    ^ percentage is taken relative to the SECOND (faster)
#                      result's wall clock — i.e. a "speedup %" of r2 vs r1.
#
# NOTE: the bench task spec phrased pct as "relative to r1", but the explicit
# expected value given was 66.67, which only resolves to delta/r2. The TDD test
# is the contract: GREEN must implement (r1-r2)/r2*100 to make this green.
# (Flagged to team lead in the task summary.)
#
# This must FAIL until scripts/bench/compare.sh exists and is executable.

load test_helper

setup() {
  # Synthetic results matching the run-one.sh result schema.
  RESULT1="$(make_temp_file r1 '{
    "label": "rag-quick",
    "goal_id": "005a5f70-64b9-40e7-a3b1-bd2eb449b71e",
    "verdict": "APPROVE",
    "completion_hash": "a1b2c3d4",
    "findings_count": 0,
    "wall_clock_sec": 100,
    "started_at": "2026-07-16T10:00:00Z",
    "ended_at": "2026-07-16T10:01:40Z",
    "transcript_path": "/tmp/r1.log",
    "store_dir": "/home/bhd/.verifier-loop"
  }')"

  RESULT2="$(make_temp_file r2 '{
    "label": "role-smart",
    "goal_id": "005a5f70-64b9-40e7-a3b1-bd2eb449b71e",
    "verdict": "REJECT",
    "completion_hash": null,
    "findings_count": 2,
    "wall_clock_sec": 60,
    "started_at": "2026-07-16T10:05:00Z",
    "ended_at": "2026-07-16T10:06:00Z",
    "transcript_path": "/tmp/r2.log",
    "store_dir": "/home/bhd/.verifier-loop"
  }')"
}

teardown() {
  rm -f "$RESULT1" "$RESULT2"
}

@test "compare: delta fields match the pinned numeric contract" {
  run "$(bench_script compare.sh)" "$RESULT1" "$RESULT2"
  echo "status=$status"
  echo "output=$output"

  [ "$status" -eq 0 ]

  # First stdout line is the machine-readable delta JSON.
  first_line="$(printf '%s\n' "$output" | head -n1)"
  echo "first_line=$first_line"

  echo "$first_line" | jq -e '
    (.time_delta_sec    | tonumber) == 40
    and (.time_delta_pct   | tonumber) == 66.67
    and (.verdict_agreement            == false)
    and (.findings_delta   | tonumber) == 2
  ' >/dev/null
}

@test "compare: APPROVE vs REJECT is reported as disagreement" {
  run "$(bench_script compare.sh)" "$RESULT1" "$RESULT2"
  echo "output=$output"
  first_line="$(printf '%s\n' "$output" | head -n1)"

  # APPROVE vs REJECT must NOT agree.
  echo "$first_line" | jq -e '.verdict_agreement == false' >/dev/null
}
