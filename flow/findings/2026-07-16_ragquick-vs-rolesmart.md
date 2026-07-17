# Benchmark: rag-quick vs role-smart via jewilo verifier-loop

**Date:** 2026-07-16 / 2026-07-17 (runs executed 2026-07-17 03:25Z–04:09Z ICT)
**Question:** When verifier-loop (`jewilo`) judges a real OpenSpec task, how do the two
pi-served models on provider `bhd-litellm` compare on (a) time-to-verdict and (b) correctness?

- **Baseline:** `role-smart` (Role Smart, reasoning, 500k ctx, 32k out) — prod pi config.
- **Alt:** `rag-quick` (RAG Quick, reasoning, 200k ctx, 32k out, qwen-chat-template thinking) — `~/.pi-bench-ragquick/` (PROD untouched).

## Target task

`Fission-AI/OpenSpec` change **`graceful-status-no-changes`** @ commit `0a99f41` (2026-07-10).
Clone lives at `/tmp/openspec-bench` (NOT committed into verifier-loop — see `flow/findings/2026-07-16_bench-openspec-task-scout.md`).

Same goal text (`/tmp/bench-openspec-goal.md`) used for BOTH runs — apples-to-apples.
Ground truth (known a priori, by reading spec + impl): the change is **mostly implemented**
but has **2 real spec divergences**:

| ID | Spec | Impl | Type |
|----|------|------|------|
| S2 | json mode outputs EXACT compact `{"changes":[],"message":"No active changes."}` | impl emits `{changes:[], message:'No active changes.', root:{path,source}}` pretty-printed (extra `root` field, indented) | REAL DEFECT |
| S3 | `--change non-existent` throws `Change 'non-existent' not found` | impl appends `. Available changes:\n  feat-x` suffix | REAL DEFECT |
| S1, S3-missing, S5 | text mode / "missing --change" / `instructions` unaffected | all match exactly | OK |

So the *correct* verdict is **REJECT** with findings on S2 + S3. (Any verifier that APPROVES
this is rubber-stamping; any that REJECTS on S2 + S3 is correct.)

## Raw results

| metric | baseline `role-smart` | alt `rag-quick` | delta |
|--------|------------------------|------------------|-------|
| wall-clock (s) | **618.63** | **983.27** | **+364.64 (alt is +58.9% slower)** |
| verdict | REJECT | REJECT | AGREE |
| consensus | 2/2 REJECT | 2/2 REJECT | AGREE |
| findings_count (parser) | 5 | 6 | +1 |
| goal_id | `c2100d1f-9dc9-4e7c-a3b9-dbf45b6853ae` | `cbbff12e-d8ab-43d1-b3ea-dff104bdbba1` | — |
| jewilo exit | 1 (REJECT) | 1 (REJECT) | same |
| v1 turnsUsed / nudgeAttempts | 1 / 0 | 6 / 5 | alt needed far more nudging |
| v2 turnsUsed / nudgeAttempts | 1 / 0 | 3 / 2 | alt needed more nudging |
| compaction observed | no | no | — |

Run transcripts:
- baseline: `scripts/bench/runs/baseline-role-smart-20260717T032515Z.{log,result.json}`
- alt: `scripts/bench/runs/alt-rag-quick-20260717T035250Z.{log,result.json}`

## Correctness analysis

**Verdict agreement:** YES — both models reached a 2/2 REJECT consensus. Neither
rubber-stamped. **Both are correct** given the ground truth above.

**Did each catch the 2 real defects?**

| Defect | role-smart | rag-quick |
|--------|-----------|-----------|
| S2 — extra `root` field in JSON | ✅ v1 D1 BLOCKER + v2 BLOCKER | ✅ v1 D1 + v2 D1 BLOCKER |
| S2 — pretty-printing vs compact | ✅ v1 D1 (rolled into root-field finding) | ✅ v2 D3 BLOCKER (split out explicitly) |
| S3 — `not found` message suffix | ✅ v1 D2 MAJOR | ✅ v1 D2 + v2 D2 BLOCKER |

**False positives / over-flagging:**
- **rag-quick v2 D4**: flagged `show` command emitting "Nothing to show" vs spec's
  "No changes found". This is technically a real spec-vs-impl gap, BUT the spec wording
  on scenario 5 is loose (the impl's `show` was never routed through
  `validateChangeExists`, so it was never in the changed code's blast radius). role-smart
  explicitly called this out as "loose spec, not flagged" — a more measured take. So
  rag-quick's D4 is a *defensible but overly literal* flag.
- **role-smart v2 MAJOR "weakened test"**: flagged that `artifact-workflow.test.ts`
  doesn't assert byte-for-byte JSON shape — a real meta-finding the spec doesn't strictly
  demand but is good engineering. rag-quick did not raise this.

**Missed findings:** Neither model missed S2 or S3. Both correctly identified the
core defects by actually running the built CLI against `/tmp/v*-empty` fixture dirs
(both models executed bash commands, not just static reads).

**Hallucinated findings:** None in either run — every cited string/line number was
verifiable against the actual files.

## Time-to-verdict analysis

- **role-smart: 618.6s (~10.3 min)** — both verifiers emitted a clean verdict on turn 1
  with zero nudges.
- **rag-quick: 983.3s (~16.4 min)** — v1 needed **6 turns + 5 nudge attempts** before
  emitting the final `VERDICT: REJECT` line; v2 needed 3 turns + 2 nudges. The qwen-chat-template
  thinking format produces long reasoning traces that often don't end in a clean
  `VERDICT:` token, so the jewilo orchestrator had to nudge repeatedly.
- **Delta:** rag-quick is **+58.9% slower** wall-clock, and its per-verifier turn
  count is **~3–6× higher**. The model itself isn't necessarily slower per-token; the
  overhead is the orchestrator nudging it to terminate.

## Findings-count delta

`+1` (5 vs 6). This is NOT a meaningful quality delta — it reflects counting style:
- role-smart v1 folded "extra root field + pretty-printing" into ONE D1 finding.
- rag-quick v2 split them into D1 (root field) + D3 (pretty-printing).
- rag-quick v2 added D4 (show command) which role-smart chose not to flag.

So rag-quick is *more granular / more literal*, role-smart is *more consolidated /
more interpretive*. Both surface the same core 2 defects.

## Recommendation

For the verifier-loop use case (judging a real OpenSpec task), given this single
apples-to-apples data point:

1. **Both are correct.** Neither rubber-stamps; both catch the real S2/S3 defects by
   actually executing the CLI. Verdict reliability is equivalent on this task.
2. **role-smart is ~1.6× faster wall-clock and needs far fewer nudges** (0 vs 5/2).
   The nudge overhead is the dominant time cost for rag-quick, not raw inference.
3. **rag-quick is more granular** — splits serialization-form defects into separate
   findings, which is *useful for the implementer* but *noisy for an APPROVE/REJECT gate*.
4. **Use role-smart as the verifier default** (current prod config is correct).
   rag-quick is a viable fallback if role-smart is unavailable, but expect ~60% longer
   rounds and higher orchestrator load from nudging.

### Caveats (limits of this benchmark)

- **N=1 task, N=1 round each.** A single OpenSpec change. Not a statistical claim.
  A follow-up sweep across ≥5 tasks (mix of APPROVE-expected + REJECT-expected) is
  needed before any production swap.
- **Same fixture, same goal text** — controls for task variance but not for model
  variance across task *types* (e.g. a task where the spec is genuinely satisfied may
  expose rubber-stamping that this REJECT-expected task cannot).
- **rag-quick's qwen-chat-template thinking format** drives the high nudge count;
  a different thinking format or a stricter system prompt that forces an early
  `VERDICT:` token could close most of the time gap.
- **Both runs hit the same LiteLLM proxy** at `100.114.135.99:24001`; transient rate
  limits on `rag-quick` were observed during smoke tests (one fallback chain burned
  claude-sonnet quota before settling). The recorded 983s includes any in-flight retries.
- **Prod drift:** ZERO. `~/.pi/agent/{settings,models,auth,config.toml,mcp.json}` mtimes
  all predate this session; only `~/.pi-bench-ragquick/` was created.

## Reproduction

```bash
# Fixture (one-time)
git clone https://github.com/Fission-AI/OpenSpec.git /tmp/openspec-bench
cd /tmp/openspec-bench && git checkout 0a99f41 && pnpm install --frozen-lockfile

# Baseline (prod pi config — role-smart)
cd <worktree>
bash scripts/bench/run-one.sh baseline-role-smart /tmp/bench-openspec-goal.md

# Alt (~/.pi-bench-ragquick — rag-quick)
bash scripts/bench/run-one.sh alt-rag-quick /tmp/bench-openspec-goal.md /home/bhd/.pi-bench-ragquick

# Compare
bash scripts/bench/compare.sh \
  scripts/bench/runs/baseline-role-smart-*.result.json \
  scripts/bench/runs/alt-rag-quick-*.result.json
```

## Test gate

`bats tests/bench/` → **12/12 ok** (covers parse-verdict REJECT/APPROVE/NONE/empty +
jewilo --json shape + compare delta).
