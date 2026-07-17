#!/usr/bin/env bats
# RED tests for parse-verdict.sh handling REAL jewilo --json output.
# These pin the contract extension: parser must handle jewilo's actual
# `--json` output shape (status: rejected/approved + rejectNotes / completion.hash),
# in addition to the synthetic "VERDICT:" text form covered by the other bats files.

load test_helper

# Fixture A — jewilo REJECT (round rejected, both verifiers REJECT with D1/D2 findings).
# Captured from a real jewilo --json run (goal graceful-status-no-changes bench).
JEWILO_REJECT_JSON='{"ok":false,"command":"new","goalId":"c2100d1f-9dc9-4e7c-a3b9-dbf45b6853ae","round":1,"status":"rejected","rejection":{"rejectNotes":[["v1","D1 (BLOCKER) - JSON output violates spec.\nD2 (MAJOR) - error message suffix.\n"],["v2","BLOCKER - Scenario 2 violates spec byte-for-byte.\nMAJOR - test does not catch violation.\n"]],"nullVerifiers":[],"signatureFailures":[]},"error":"round 1 rejected"}'

# Fixture B — jewilo APPROVE (round approved with completion hash).
JEWILO_APPROVE_JSON='{"ok":true,"command":"new","goalId":"abc12345-9dc9-4e7c-a3b9-dbf45b6853ae","round":1,"status":"approved","completion":{"goalId":"abc12345-9dc9-4e7c-a3b9-dbf45b6853ae","roundNumber":1,"hash":"a1b2c3d4","matchedAt":"2026-07-17T03:35:00Z","matchingVerdicts":["v1","v2"]}}'

# Fixture C — jewilo nullVerifiers (timeout, no verdict).
JEWILO_NULL_JSON='{"ok":false,"command":"new","goalId":"def67890-9dc9-4e7c-a3b9-dbf45b6853ae","round":1,"status":"rejected","rejection":{"rejectNotes":[],"nullVerifiers":["v1","v2"],"signatureFailures":[]},"error":"round 1 no verdicts"}'

@test "parse-verdict: jewilo --json REJECT is detected as REJECT" {
  local t; t="$(make_temp_file jewilo-reject "$JEWILO_REJECT_JSON")"
  run "$(bench_script parse-verdict.sh)" "$t"
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.verdict == "REJECT"' >/dev/null
  rm -f "$t"
}

@test "parse-verdict: jewilo --json REJECT counts findings across both verifiers" {
  local t; t="$(make_temp_file jewilo-reject "$JEWILO_REJECT_JSON")"
  run "$(bench_script parse-verdict.sh)" "$t"
  [ "$status" -eq 0 ]
  # v1 contributes D1 + D2 (2 lines) ; v2 contributes BLOCKER + MAJOR (2 lines).
  # Each finding is one line; count = 4 across both verifiers.
  echo "$output" | jq -e '.findings_count == 4' >/dev/null
  rm -f "$t"
}

@test "parse-verdict: jewilo --json APPROVE extracts completion hash" {
  local t; t="$(make_temp_file jewilo-approve "$JEWILO_APPROVE_JSON")"
  run "$(bench_script parse-verdict.sh)" "$t"
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.verdict == "APPROVE" and .completion_hash == "a1b2c3d4"' >/dev/null
  rm -f "$t"
}

@test "parse-verdict: jewilo --json nullVerifiers yields NONE" {
  local t; t="$(make_temp_file jewilo-null "$JEWILO_NULL_JSON")"
  run "$(bench_script parse-verdict.sh)" "$t"
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.verdict == "NONE" and .findings_count == 0' >/dev/null
  rm -f "$t"
}
