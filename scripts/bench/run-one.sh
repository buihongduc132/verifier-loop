#!/usr/bin/env bash
# run-one.sh — run a single jewilo NEW benchmark and capture the result.
#
# Usage: run-one.sh <label> <goal-file> [pi_config_dir]
#
# Runs `jewilo --json NEW --init-prompt-file <goal-file>`, capturing the
# combined stdout+stderr transcript, then parses it via parse-verdict.sh and
# writes one JSON result line to scripts/bench/runs/<label>-<ts>.result.json
# and the raw transcript to scripts/bench/runs/<label>-<ts>.log.
#
# Result schema:
#   {
#     "label":            <label>,
#     "goal_id":          "<uuid>" | null,
#     "verdict":          "NONE" | "APPROVE" | "REJECT",
#     "completion_hash":  null | "<hex>",
#     "findings_count":   <number>,
#     "wall_clock_sec":   <float, seconds>,
#     "started_at":       "<rfc3339>",
#     "ended_at":         "<rfc3339>",
#     "transcript_path":  "<abs path to .log>",
#     "store_dir":        "<~/.verifier-loop abs path>"
#   }
#
# jewilo is allowed to exit non-zero (e.g. REJECT verdicts, backend crash);
# the transcript is still parsed and a result is always written.
set -euo pipefail

if [ "$#" -lt 2 ]; then
  echo "usage: run-one.sh <label> <goal-file> [pi_config_dir]" >&2
  exit 2
fi

label="$1"
goal_file="$2"
pi_config_dir="${3:-}"

if [ ! -f "$goal_file" ]; then
  echo "run-one: goal file not found: $goal_file" >&2
  exit 2
fi

# Locate sibling scripts relative to this file's location.
script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
runs_dir="$script_dir/runs"
mkdir -p "$runs_dir"

# Canonical store dir (matches jewilo's default ~/.verifier-loop).
store_dir="${VERIFIER_LOOP_STORE_DIR:-$HOME/.verifier-loop}"
store_dir="$(cd "$store_dir" 2>/dev/null && pwd || printf '%s' "$store_dir")"

ts="$(date -u +%Y%m%dT%H%M%SZ)"
ts_sortable="$(date -u +%s)"
transcript_path="$runs_dir/${label}-${ts}.log"
result_path="$runs_dir/${label}-${ts}.result.json"

# Optional alternate pi config (e.g. point at a different model profile).
env_prefix=()
if [ -n "$pi_config_dir" ]; then
  env_prefix+=(env "PI_CODING_AGENT_DIR=$pi_config_dir")
fi

started_at="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
start_ts="$(date +%s.%N)"

# Run jewilo; tolerate non-zero exit (REJECT / crash / timeout) — we still want
# to capture whatever transcript it produced.
#
# `set -e` is neutralized for this one command via `|| true` so a crashed
# backend doesn't prevent us from writing the result JSON. We still want the
# exit status for the record, though.
run_exit=0
"${env_prefix[@]}" jewilo --json NEW --init-prompt-file "$goal_file" \
  >"$transcript_path" 2>&1 || run_exit=$?

ended_at="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
end_ts="$(date +%s.%N)"

wall_clock_sec="$(awk -v s="$start_ts" -v e="$end_ts" 'BEGIN{printf "%.3f", e - s}')"

# Parse the verdict from the captured transcript.
parsed="$("$script_dir/parse-verdict.sh" "$transcript_path")"
verdict="$(printf '%s' "$parsed" | jq -r '.verdict')"
completion_hash="$(printf '%s' "$parsed" | jq -r '.completion_hash // empty')"
findings_count="$(printf '%s' "$parsed" | jq -r '.findings_count')"

# Try to extract the goalId from jewilo's --json output. jewilo's JSON envelope
# carries a top-level `goalId` on NEW; fall back to null if absent / malformed.
goal_id="$(jq -r '.goalId // empty' "$transcript_path" 2>/dev/null || true)"
if [ -z "$goal_id" ]; then
  goal_id_json="null"
else
  goal_id_json="$(jq -nR --arg g "$goal_id" '$g' )"
fi

# completion_hash: build a JSON literal (null or quoted string).
if [ -z "$completion_hash" ]; then
  hash_json="null"
else
  hash_json="$(jq -nR --arg h "$completion_hash" '$h')"
fi

# Emit the result JSON.
jq -nc \
  --arg label "$label" \
  --argjson goal_id "$goal_id_json" \
  --arg verdict "$verdict" \
  --argjson completion_hash "$hash_json" \
  --argjson findings_count "$findings_count" \
  --argjson wall "$wall_clock_sec" \
  --arg started_at "$started_at" \
  --arg ended_at "$ended_at" \
  --arg transcript "$transcript_path" \
  --arg store "$store_dir" \
  --argjson run_exit "$run_exit" \
  '{label:$label, goal_id:$goal_id, verdict:$verdict,
    completion_hash:$completion_hash, findings_count:$findings_count,
    wall_clock_sec:$wall, started_at:$started_at, ended_at:$ended_at,
    transcript_path:$transcript, store_dir:$store, jewilo_exit:$run_exit}' \
  >"$result_path"

# Echo the result path + the JSON for convenience.
cat "$result_path"
echo "---"
echo "result_path=$result_path"
