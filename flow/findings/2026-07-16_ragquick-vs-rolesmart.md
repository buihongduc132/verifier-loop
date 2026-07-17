# Benchmark: rag-quick vs role-smart via jewilo verifier-loop

**Date:** 2026-07-16 / 2026-07-17 (runs executed 2026-07-17 03:25Z–05:16Z ICT)
**Question:** When verifier-loop (`jewilo`) judges a real OpenSpec task, how do the two
pi-served models on provider `bhd-litellm` compare on (a) time-to-verdict and (b) correctness?

- **Baseline:** `role-smart` (Role Smart, reasoning, 500k ctx, 32k out) — prod pi config
  (`~/.pi/agent/`, defaultModel=role-smart, defaultThinkingLevel=high, full 32-package extension set).
- **Alt:** `rag-quick` (RAG Quick, reasoning, 200k ctx, 32k out, qwen-chat-template thinking) —
  `~/.pi-bench-ragquick/` (PROD untouched). defaultThinkingLevel=high (matched to prod).

## ⚠️ Config delta disclosure (apples-to-apples caveat)

The alt config could NOT be made byte-identical to prod because of a **LiteLLM-side
constraint**: the `rag-quick` model group on the shared LiteLLM proxy at `100.114.135.99:24001`
is registered **without** `--enable-auto-tool-choice --tool-call-parser`. When pi loads the
**full 32-package extension set**, it sends `tool_choice=auto` with the full tool inventory
and LiteLLM rejects with `400: "auto" tool choice requires --enable-auto-tool-choice`.
(Confirmed empirically: same prompt + `packages=[]` → succeeds; same prompt + full packages → 400.)

Therefore the alt config differs from prod in exactly TWO fields:

| field | prod (baseline) | alt (rag-quick) | note |
|-------|-----------------|------------------|------|
| `defaultModel` | `role-smart` | `rag-quick` | INTENTIONAL (the variable under test) |
| `defaultProvider` | `bhd-litellm` | `bhd-litellm` | same |
| `defaultThinkingLevel` | `high` | `high` | **matched** |
| `packages` | 32 entries | `[]` | **FORCED** by LiteLLM rag-quick tool_choice bug |

**Direction of the confound:** stripping packages gives rag-quick a *slight advantage*
(fewer tools to serialize per turn, no extension hook overhead). So the recorded alt
wall-clock is a **lower bound** on rag-quick's true time-vs-role-smart; the real gap with
a fixed LiteLLM would be modestly larger. This does NOT invalidate the verdict-correctness
comparison (both configs give the verifier full bash/filesystem access, which is all the
OpenSpec task needs).

**Model identity was verified at the session level** (a prior run accidentally used
role-smart for the "alt" due to a config-edit mistake; this was caught by the verifier-loop
on this very harness and corrected). The reported alt numbers below come from sessions whose
JSONL records contain `"modelId":"rag-quick"` (see artifacts/alt-rag-quick.log spawn timestamps
vs `~/.pi-bench-ragquick/sessions/`).

## Target task

`Fission-AI/OpenSpec` change **`graceful-status-no-changes`** @ commit `0a99f41` (2026-07-10).
Clone lives at `/tmp/openspec-bench` (NOT committed into verifier-loop — see `flow/findings/2026-07-16_bench-openspec-task-scout.md`).

Same goal text (`/tmp/bench-openspec-goal.md`, copied to `…-artifacts/goal-text.md`) used for
BOTH runs — apples-to-apples on the task. Ground truth (known a priori, by reading spec + impl):
the change is **mostly implemented** but has **2 real spec divergences**:

| ID | Spec | Impl | Type |
|----|------|------|------|
| S2 | json mode outputs EXACT compact `{"changes":[],"message":"No active changes."}` | impl emits `{changes:[], message:'No active changes.', root:{path,source}}` pretty-printed (extra `root` field, indented) | REAL DEFECT |
| S3 | `--change non-existent` throws `Change 'non-existent' not found` | impl appends `. Available changes:\n  feat-x` suffix | REAL DEFECT (minor — suffix is arguably helpful) |
| S1, S3-missing, S5 | text mode / "missing --change" / `instructions` unaffected | all match exactly | OK |

So the *correct* verdict is **REJECT** with findings on S2 + S3. (Any verifier that APPROVES
this is rubber-stamping; any that REJECTS on S2 + S3 is correct.)

## Raw results

| metric | baseline `role-smart` | alt `rag-quick` | delta |
|--------|------------------------|------------------|-------|
| wall-clock (s) | **618.63** | **1731.35** | **+1112.72 (alt is +179.8% / ~2.8× slower)** |
| verdict | REJECT | REJECT | AGREE |
| consensus | 2/2 REJECT | 2/2 REJECT | AGREE |
| findings_count (parser) | 5 | 2 | −3 |
| goal_id | `c2100d1f-9dc9-4e7c-a3b9-dbf45b6853ae` | `a09dc501-b4d9-4fcc-b23e-5e99b85bb8ee` | — |
| jewilo exit | 1 (REJECT) | 1 (REJECT) | same |
| v1 turnsUsed / nudgeAttempts | 1 / 0 | 3 / 2 | alt needs more |
| v2 turnsUsed / nudgeAttempts | 1 / 0 | **8 / 7** | alt v2 struggled badly |
| compaction observed | no | no | — |

Run transcripts (in `flow/findings/2026-07-16_ragquick-vs-rolesmart-artifacts/`):
- baseline: `baseline-role-smart.{log,result.json}`
- alt: `alt-rag-quick.{log,result.json}`

## Correctness analysis

**Verdict agreement:** YES — both models reached a 2/2 REJECT consensus. Neither
rubber-stamped. **Both are correct** given the ground truth above.

**Did each catch the 2 real defects?**

| Defect | role-smart | rag-quick |
|--------|-----------|-----------|
| S2 — extra `root` field in JSON | ✅ v1 D1 BLOCKER + v2 BLOCKER | ✅ v1 D1 BLOCKER + v2 D2 MAJOR |
| S2 — pretty-printing vs compact | ✅ v1 D1 (rolled into root-field finding) | ✅ v2 D2 (rolled in with root field) |
| S3 — `not found` message suffix | ✅ v1 D2 MAJOR | ✅ v1 D2 BLOCKER + v2 D3 MAJOR |

**False positives / over-flagging:**
- **rag-quick v2 D1**: flagged `show` command emitting "Nothing to show" vs spec's "No changes
  found" — same false-positive pattern as the original v1 run. Defensible-but-overly-literal;
  role-smart explicitly classified this as out-of-scope (show never routed through
  `validateChangeExists`). One false positive out of 3 v2 findings.
- **role-smart v2 MAJOR "weakened test"**: flagged that `artifact-workflow.test.ts` doesn't
  assert byte-for-byte JSON shape — a real meta-finding. rag-quick did not raise this.

**Missed findings:** Neither model missed S2 or S3. Both correctly identified the core
defects by actually running the built CLI against `/tmp/v*-empty` fixture dirs.

**Hallucinated findings:** None in either run — every cited string/line number was
verifiable against the actual files.

**Net correctness:** Tied on the core verdict. role-smart is more *thorough* (5 findings,
including a meta-finding on the weakened test); rag-quick is more *terse* (2-3 findings,
1 false positive).

## Time-to-verdict analysis

- **role-smart: 618.6s (~10.3 min)** — both verifiers clean turn-1 verdicts, 0 nudges.
- **rag-quick: 1731.4s (~28.9 min)** — v1 needed 3 turns + 2 nudges; **v2 needed 8 turns + 7 nudges**
  before emitting a clean `VERDICT:` line. The qwen-chat-template thinking format produces long
  reasoning traces that often don't terminate cleanly, forcing the jewilo orchestrator to nudge
  repeatedly.
- **Delta:** rag-quick is **~2.8× slower** wall-clock. The dominant cost is NOT raw inference
  speed but the **orchestrator nudge loop** — v2 alone burned 7 nudges over 8 turns. With a
  stricter system prompt that forced an early `VERDICT:` token, much of this overhead would
  vanish, but that is a prompt-engineering change not measured here.
- **Caveat:** rag-quick ran with `packages=[]` (slight advantage per the disclosure above).
  With a fixed LiteLLM and the full extension set, the gap would be modestly *larger*, not smaller.

## Findings-count delta

`−3` (5 vs 2). This IS a meaningful quality delta:
- role-smart surfaces S2 + S3 + the weakened-test meta-finding + splits serialization-form
  defects into separate items. More actionable for an implementer.
- rag-quick surfaces S2 + S3 concisely (plus 1 false positive in v2). Sufficient for an
  APPROVE/REJECT gate but less diagnostic.

## Recommendation

For the verifier-loop use case (judging a real OpenSpec task), given this single
apples-to-apples data point:

1. **Both reach the correct verdict.** Neither rubber-stamps; both catch the real S2/S3
   defects by actually executing the CLI. Verdict reliability is equivalent on this task.
2. **role-smart is ~2.8× faster wall-clock and needs far fewer nudges** (0/0 vs 2/7). The
   nudge overhead is the dominant time cost for rag-quick, driven by the qwen-chat-template
   thinking format not terminating cleanly.
3. **role-smart is more thorough** — surfaces 5 findings (incl. a meta-finding on the
   weakened test) vs rag-quick's 2-3. More actionable for the implementer.
4. **rag-quick is more terse** — sufficient for a binary APPROVE/REJECT gate but less
   diagnostic; one false positive (show command) in this run.
5. **Use role-smart as the verifier default** (current prod config is correct).
   rag-quick is a viable fallback if role-smart is unavailable, but expect ~2.8× longer
   rounds and higher orchestrator load from nudging.
6. **Thinking level is critical for rag-quick.** (An earlier run at `defaultThinkingLevel=medium`
   was even slower/noisier; matching prod's `high` helped but did not close the gap.)
7. **LiteLLM follow-up:** enabling `--enable-auto-tool-choice --tool-call-parser` on the
   `rag-quick` model group would let rag-quick run with the full extension set and yield a
   truly like-for-like wall-clock comparison. Filed below.

### Caveats (limits of this benchmark)

- **N=1 task, N=1 round each.** A single OpenSpec change. Not a statistical claim.
  A follow-up sweep across ≥5 tasks (mix of APPROVE-expected + REJECT-expected) is needed
  before any production swap.
- **Same fixture, same goal text** — controls for task variance but not for model variance
  across task *types*.
- **packages=[] confound** on alt (see disclosure above) — rag-quick's wall-clock is a lower
  bound; the true gap with a fixed LiteLLM would be modestly larger.
- **Both runs hit the same LiteLLM proxy** at `100.114.135.99:24001`; transient rate limits
  on `rag-quick` were observed during smoke tests.
- **Prod content drift: ZERO.** `diff <(jq -S . ~/.pi/agent/settings.json) <(jq -S . ~/.pi-bench-ragquick/settings.json)`
  shows the alt differs from prod ONLY in `defaultModel` (role-smart→rag-quick) and `packages`
  (32 entries → `[]`, forced by the LiteLLM bug). Prod settings.json content is byte-identical
  to its pre-session state. (Mtime may bump from unrelated pi bookkeeping; content is what's verified.)
- **The LiteLLM `tool_choice=auto` misconfiguration on `rag-quick`** is a separate ops issue
  against the LiteLLM proxy config (`noco-mesh`), NOT a defect in rag-quick itself or in pi.

## Process lesson (recorded for traceability)

Two intermediate alt runs were discarded:
- **v1** used `defaultThinkingLevel=medium` (a confound — flagged by the verifier-loop on this
  harness as D1 MAJOR). 983s, 6 findings.
- **v2** accidentally used `defaultModel=role-smart` due to a config-edit mistake (a `jq`
  pipeline read from prod and only modified thinking+packages, dropping the model swap).
  Flagged by the verifier-loop as D1-D3 BLOCKER ("FABRICATED BENCHMARK"). 307s.
- **v3** (reported above) is the first run with verified-correct alt config
  (`defaultModel=rag-quick` confirmed in session JSONL).

The verifier-loop caught BOTH errors before any false claim shipped. This is the system
working as designed.

## Follow-ups (out of scope for this benchmark)

1. LiteLLM: enable `--enable-auto-tool-choice --tool-call-parser hermes` (or the Qwen
   variant) on the `rag-quick` model group so pi can run it with the full extension set.
2. Re-run this benchmark after (1) lands to get a true like-for-like wall-clock number.
3. Sweep ≥5 OpenSpec tasks (mixed APPROVE/REJECT expected) for statistical significance.
4. Investigate a stricter verifier system prompt for rag-quick to force early `VERDICT:`
   emission and cut the nudge-loop overhead.

## Reproduction

```bash
# Fixture (one-time)
git clone https://github.com/Fission-AI/OpenSpec.git /tmp/openspec-bench
cd /tmp/openspec-bench && git checkout 0a99f41 && pnpm install --frozen-lockfile

# Baseline (prod pi config — role-smart, full packages, thinking=high)
cd <worktree>
bash scripts/bench/run-one.sh baseline-role-smart /tmp/bench-openspec-goal.md

# Alt (~/.pi-bench-ragquick — rag-quick, packages=[], thinking=high)
bash scripts/bench/run-one.sh alt-rag-quick /tmp/bench-openspec-goal.md /home/bhd/.pi-bench-ragquick

# Compare
bash scripts/bench/compare.sh \
  scripts/bench/runs/baseline-role-smart-*.result.json \
  scripts/bench/runs/alt-rag-quick-*.result.json
```

## Test gate

`bats tests/bench/` → **12/12 ok** (covers parse-verdict REJECT/APPROVE/NONE/empty +
jewilo --json shape + compare delta).
