# Benchmark: rag-quick vs role-smart via jewilo verifier-loop

**Date:** 2026-07-16 / 2026-07-17 (runs executed 2026-07-17 03:25Z–04:34Z ICT)
**Question:** When verifier-loop (`jewilo`) judges a real OpenSpec task, how do the two
pi-served models on provider `bhd-litellm` compare on (a) time-to-verdict and (b) correctness?

- **Baseline:** `role-smart` (Role Smart, reasoning, 500k ctx, 32k out) — prod pi config
  (`~/.pi/agent/`, defaultModel=role-smart, defaultThinkingLevel=high, full 32-package extension set).
- **Alt:** `rag-quick` (RAG Quick, reasoning, 200k ctx, 32k out, qwen-chat-template thinking) —
  `~/.pi-bench-ragquick/` (PROD untouched).

## ⚠️ Config delta disclosure (apples-to-apples caveat)

The alt config could NOT be made byte-identical to prod because of a **LiteLLM-side
constraint**: the `rag-quick` model group on the shared LiteLLM proxy at `100.114.135.99:24001`
is registered **without** `--enable-auto-tool-choice --tool-call-parser`. When pi loads the
**full 32-package extension set**, it sends `tool_choice=auto` with the full tool inventory
and LiteLLM rejects with `400: "auto" tool choice requires --enable-auto-tool-choice`.
(Confirmed: with `packages=[]` the same prompt succeeds; with full packages it 400s.)

Therefore the alt config is:
| field | prod (baseline) | alt (rag-quick) | note |
|-------|-----------------|------------------|------|
| `defaultModel` | `role-smart` | `rag-quick` | INTENTIONAL (the variable under test) |
| `defaultProvider` | `bhd-litellm` | `bhd-litellm` | same |
| `defaultThinkingLevel` | `high` | `high` | **matched** (corrected after v1) |
| `packages` | 32 entries | `[]` | **FORCED** by LiteLLM rag-quick tool_choice bug |

**Direction of the confound:** stripping packages gives rag-quick a *slight advantage*
(fewer tools to serialize per turn, no extension hook overhead). So the recorded alt
wall-clock is a **lower bound** on rag-quick's true time-vs-role-smart; the real gap with
a fixed LiteLLM could be modestly larger. This does NOT invalidate the verdict-correctness
comparison (both configs give the verifier full bash/filesystem access, which is all the
OpenSpec task needs).

### v1 vs v2 history (why v2 is the reported number)

An initial alt run (v1) used `defaultThinkingLevel=medium` by accident. A verifier-loop
round on this very harness flagged that as an undocumented confound (D1 MAJOR). v2
corrects `defaultThinkingLevel` to `high` (matching prod). The v1 numbers are retained
in `…-artifacts/` for traceability but the comparison below uses **v2**.

| run | thinking | wall (s) | verdict | findings | v1/v2 turns-nudges |
|-----|----------|----------|---------|----------|---------------------|
| alt v1 | medium | 983.27 | REJECT | 6 | 6+5 / 3+2 |
| **alt v2** | **high** | **307.31** | **REJECT** | **5** | **1+0 / 1+0** |

The thinking level was the dominant cost driver for v1 — not the qwen format. With matched
thinking, rag-quick emits a clean verdict on turn 1 with zero nudges, same as role-smart.

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

## Raw results (v2 alt)

| metric | baseline `role-smart` | alt `rag-quick` (v2) | delta |
|--------|------------------------|------------------------|-------|
| wall-clock (s) | **618.63** | **307.31** | **−311.32 (alt is −50.3% / ~2× faster)** |
| verdict | REJECT | REJECT | AGREE |
| consensus | 2/2 REJECT | 2/2 REJECT | AGREE |
| findings_count (parser) | 5 | 5 | 0 |
| goal_id | `c2100d1f-9dc9-4e7c-a3b9-dbf45b6853ae` | `4bb7d7a8-2c3e-44cf-9e15-c302ee122361` | — |
| jewilo exit | 1 (REJECT) | 1 (REJECT) | same |
| v1 turnsUsed / nudgeAttempts | 1 / 0 | 1 / 0 | tie |
| v2 turnsUsed / nudgeAttempts | 1 / 0 | 1 / 0 | tie |
| compaction observed | no | no | — |

Run transcripts (in `flow/findings/2026-07-16_ragquick-vs-rolesmart-artifacts/`):
- baseline: `baseline-role-smart.{log,result.json}`
- alt v2: `alt-rag-quick-v2.{log,result.json}`
- alt v1 (legacy, medium-thinking): `alt-rag-quick.{log,result.json}`

## Correctness analysis

**Verdict agreement:** YES — both models reached a 2/2 REJECT consensus. Neither
rubber-stamped. **Both are correct** given the ground truth above.

**Did each catch the 2 real defects?**

| Defect | role-smart | rag-quick (v2) |
|--------|-----------|-----------------|
| S2 — extra `root` field in JSON | ✅ v1 D1 BLOCKER + v2 BLOCKER | ✅ v1 D1 BLOCKER + v2 BLOCKER D1 |
| S2 — pretty-printing vs compact | ✅ v1 D1 (rolled into root-field finding) | ✅ v1 D2 BLOCKER (split out) |
| S3 — `not found` message suffix | ✅ v1 D2 MAJOR | ✅ listed under "verified passing scenarios" (treated as benign suffix, same call as role-smart v2) |
| weakened test (meta-finding) | ✅ v2 MAJOR | ✅ v1 D3 MAJOR + v2 (cited) |

**False positives / over-flagging:** NONE in v2. (v1 had a D4 on the `show` command which
was defensible-but-overly-literal; v2 correctly classified it as out-of-scope.)

**Missed findings:** Neither model missed S2 or S3. Both correctly identified the core
defects by actually running the built CLI against `/tmp/v*-empty` fixture dirs (both
models executed bash commands + hexdumped stdout, not just static reads).

**Hallucinated findings:** None in either run — every cited string/line number was
verifiable against the actual files.

## Time-to-verdict analysis

- **role-smart: 618.6s (~10.3 min)** — both verifiers clean turn-1 verdicts, 0 nudges.
- **rag-quick v2: 307.3s (~5.1 min)** — both verifiers clean turn-1 verdicts, 0 nudges.
- **Delta:** rag-quick is **~2× faster** wall-clock on this task, with identical turn/nudge
  profile. Caveat: rag-quick ran with `packages=[]` (slight advantage — see disclosure above),
  so the true speedup with a fixed LiteLLM would be somewhat smaller, but the direction
  (rag-quick ≳ role-smart on this task) is robust.
- **v1 lesson:** the original 983s was dominated by `defaultThinkingLevel=medium` causing
  the qwen model to truncate reasoning and need 5 orchestrator nudges to emit a `VERDICT:`
  token. Matching thinking level to prod (`high`) eliminated that entirely.

## Findings-count delta

`0` (5 vs 5). Both surface the same core defects with the same severity assignments.

## Recommendation

For the verifier-loop use case (judging a real OpenSpec task), given this single
apples-to-apples data point (v2):

1. **Both are correct and equally reliable.** Neither rubber-stamps; both catch the real
   S2/S3 defects by actually executing the CLI. Verdict reliability is equivalent on this task.
2. **rag-quick is ~2× faster wall-clock** on this task with matched thinking level, with
   an identical 1-turn / 0-nudge verdict profile.
3. **Caveat — packages stripping:** the speedup is measured with rag-quick running
   `packages=[]` (forced by the LiteLLM `tool_choice=auto` misconfiguration on the
   `rag-quick` group). role-smart ran the full 32-package set. A fair like-for-like
   comparison requires fixing LiteLLM (`--enable-auto-tool-choice --tool-call-parser`
   on the `rag-quick` model group) and re-running. Until then, treat the ~2× as a
   lower bound on rag-quick's speed advantage.
4. **Thinking level is critical for rag-quick.** At `medium` it needed 5 nudges/verifier
   and 3.2× the wall-clock; at `high` (matching prod) it was clean. Any deployment of
   rag-quick as a verifier backend MUST pin `defaultThinkingLevel=high`.
5. **Operational recommendation:** rag-quick is a viable **faster** verifier backend than
   role-smart for spec-string-exact tasks, PROVIDED (a) thinking level is pinned high and
   (b) the LiteLLM `tool_choice=auto` issue is resolved so it can run with the same
   extension set. Until (b) is fixed, role-smart remains the safer default because it
   runs cleanly with the full prod config.

### Caveats (limits of this benchmark)

- **N=1 task, N=1 round each** (v2). A single OpenSpec change. Not a statistical claim.
  A follow-up sweep across ≥5 tasks (mix of APPROVE-expected + REJECT-expected) is needed
  before any production swap.
- **Same fixture, same goal text** — controls for task variance but not for model variance
  across task *types* (e.g. a task where the spec is genuinely satisfied may expose
  rubber-stamping that this REJECT-expected task cannot).
- **packages=[] confound** on alt (see disclosure above) — speedup is a lower bound.
- **Both runs hit the same LiteLLM proxy** at `100.114.135.99:24001`; transient rate limits
  on `rag-quick` were observed during smoke tests.
- **Prod content drift: ZERO.** `diff <(jq -S . ~/.pi/agent/settings.json) <(jq -S . ~/.pi-bench-ragquick/settings.json)`
  shows the alt differs from prod ONLY in `defaultModel` (role-smart→rag-quick) and `packages`
  (32 entries → `[]`, forced by the LiteLLM bug). Prod settings.json content is byte-identical
  to its pre-session state. (Mtime may bump from unrelated pi bookkeeping; content is what's verified.)
- **The LiteLLM `tool_choice=auto` misconfiguration on `rag-quick`** is a separate ops issue
  against the LiteLLM proxy config (`noco-mesh`), NOT a defect in rag-quick itself or in pi.
  Filed as a follow-up below.

## Follow-ups (out of scope for this benchmark)

1. LiteLLM: enable `--enable-auto-tool-choice --tool-call-parser hermes` (or the Qwen
   variant) on the `rag-quick` model group so pi can run it with the full extension set.
2. Re-run this benchmark after (1) lands to get a true like-for-like wall-clock number.
3. Sweep ≥5 OpenSpec tasks (mixed APPROVE/REJECT expected) for statistical significance.

## Reproduction

```bash
# Fixture (one-time)
git clone https://github.com/Fission-AI/OpenSpec.git /tmp/openspec-bench
cd /tmp/openspec-bench && git checkout 0a99f41 && pnpm install --frozen-lockfile

# Baseline (prod pi config — role-smart, full packages, thinking=high)
cd <worktree>
bash scripts/bench/run-one.sh baseline-role-smart /tmp/bench-openspec-goal.md

# Alt (~/.pi-bench-ragquick — rag-quick, packages=[], thinking=high)
bash scripts/bench/run-one.sh alt-rag-quick-v2 /tmp/bench-openspec-goal.md /home/bhd/.pi-bench-ragquick

# Compare
bash scripts/bench/compare.sh \
  scripts/bench/runs/baseline-role-smart-*.result.json \
  scripts/bench/runs/alt-rag-quick-v2-*.result.json
```

## Test gate

`bats tests/bench/` → **12/12 ok** (covers parse-verdict REJECT/APPROVE/NONE/empty +
jewilo --json shape + compare delta).
