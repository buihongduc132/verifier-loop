#!/usr/bin/env bash
# compare.sh — diff two run-one.sh result files.
#
# Usage: compare.sh <result1.json> <result2.json>
#
# Emits to stdout:
#   - line 1: a machine-readable JSON object:
#       {
#         "time_delta_sec":    <r1.wall - r2.wall>,
#         "time_delta_pct":    <(r1.wall - r2.wall) / r2.wall * 100, 2dp>,
#         "verdict_agreement": <r1.verdict == r2.verdict>,
#         "findings_delta":    <r2.findings_count - r1.findings_count>,
#         "label1": "<r1.label>", "label2": "<r2.label>",
#         "verdict1": "<r1.verdict>", "verdict2": "<r2.verdict>"
#       }
#   - subsequent lines: a small human-readable ASCII table.
#
# The percentage is taken relative to the SECOND (faster) result's wall clock,
# so it reads as "r2 is N% faster than r1" when r2 is faster. This is pinned by
# tests/bench/test_compare_delta.bats: (100-60)/60*100 = 66.67.
set -euo pipefail

if [ "$#" -lt 2 ]; then
  echo "usage: compare.sh <result1.json> <result2.json>" >&2
  exit 2
fi

r1="$1"
r2="$2"

for f in "$r1" "$r2"; do
  if [ ! -f "$f" ]; then
    echo "compare: result file not found: $f" >&2
    exit 2
  fi
done

# Merge both results under .r1 / .r2 and compute the delta in one jq pass.
# round-to-2dp: multiply by 100, round to nearest int, divide by 100.
delta_json="$(jq -n \
  --slurpfile a "$r1" \
  --slurpfile b "$r2" '
  ($a[0]) as $r1 | ($b[0]) as $r2
  | {
      time_delta_sec:    (($r1.wall_clock_sec - $r2.wall_clock_sec) * 100 | round | . / 100),
      time_delta_pct:    ((($r1.wall_clock_sec - $r2.wall_clock_sec) / $r2.wall_clock_sec * 100 * 100) | round | . / 100),
      verdict_agreement: ($r1.verdict == $r2.verdict),
      findings_delta:    ($r2.findings_count - $r1.findings_count),
      label1:            $r1.label,
      label2:            $r2.label,
      verdict1:          $r1.verdict,
      verdict2:          $r2.verdict
    }
  ')"

# Line 1: machine-readable compact JSON.
printf '%s\n' "$(printf '%s' "$delta_json" | jq -c .)"

# Lines 2+: human table.
printf '%s\n' "$delta_json" | jq -r '
  (["metric", .label1, .label2, "delta"] | @tsv),
  (["wall (s)", "—", "—", (.time_delta_sec|tostring)] | @tsv),
  (["verdict", .verdict1, .verdict2, (if .verdict_agreement then "AGREE" else "DISAGREE" end)] | @tsv),
  (["findings", "—", "—", (.findings_delta|tostring)] | @tsv)
'
