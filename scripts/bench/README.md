# scripts/bench — verifier-loop benchmark harness

Three small bash scripts for benchmarking two pi-served models (`rag-quick` vs
`role-smart`) on a single verifier-loop goal, and diffing the results.

## parse-verdict.sh

`parse-verdict.sh <transcript-file>` — parses a captured `jewilo` run
transcript (combined stdout+stderr) into a structured JSON verdict object:
`{verdict, completion_hash, findings_count}`. The verdict line
(`VERDICT: APPROVE|REJECT`) and the completion-hash line
(`Completion hash: <hex>`) are matched case-insensitively; "findings" are body
lines starting with `D<digits>` (D1 BLOCKER, D2 MAJOR, ...). Always exits 0 —
empty input or a missing verdict line is a valid `NONE` state, not an error.

## run-one.sh

`run-one.sh <label> <goal-file> [pi_config_dir]` — runs a single
`jewilo --json NEW --init-prompt-file <goal-file>` invocation, capturing wall
clock from `date +%s.%N` before/after. The combined transcript is written to
`scripts/bench/runs/<label>-<ts>.log`; `parse-verdict.sh` is then run over it
and the structured result (label, goal_id, verdict, completion_hash,
findings_count, wall_clock_sec, started_at, ended_at, transcript_path,
store_dir, jewilo_exit) is written to
`scripts/bench/runs/<label>-<ts>.result.json`. An optional third arg sets
`PI_CODING_AGENT_DIR` for the run (use it to point at a different model
profile). `jewilo` non-zero exit is tolerated (wrapped in `|| true`) so a REJECT
or crashed backend still produces a result row.

## compare.sh

`compare.sh <result1.json> <result2.json>` — diffs two `run-one.sh` result
files. The **first stdout line** is a machine-readable JSON object:
`{time_delta_sec, time_delta_pct, verdict_agreement, findings_delta, label1,
label2, verdict1, verdict2}`. The percentage is taken relative to the **second**
(faster) result's wall clock — i.e. `(r1.wall - r2.wall) / r2.wall * 100`,
rounded to 2dp — so it reads as "r2 is N% faster than r1". Subsequent lines are
a small human-readable ASCII table. The percentage basis (relative to r2) is
pinned by `tests/bench/test_compare_delta.bats` and intentionally differs from
the original task spec ("relative to r1"); the test is the contract.
