## Context

jewilo (verifier-loop 0.1.0) currently produces null verdicts on the majority of real verification runs. Three independent root causes were confirmed by dedicated verification sub-agents against source + live `~/.verifier-loop/goals/` dirs + session JSONLs:

1. **Prompt bloat** (Group C): `capture_file_edit_times` (`src/prompt/mod.rs:346-369`) emits `<path>:<mtime>` for **every** file in `git ls-files` with no cap, no truncation, no relevance filter. In repos with thousands of tracked files (pi-plugins: 4,259 files → 389KB), this is 40-83% of every rendered prompt. Combined with 2× policy duplication (custom `verifierPromptFile` + embedded `VERIFIER_POLICY`) and unbounded `--context`, prompts reach 142-686KB / up to 242K input tokens. The pi backend hits `type:compaction` at turn 1 and the verifier never emits a verdict.

2. **No verdict enforcement** (Group B): the prompt template (`src/prompt/default_template.txt:32`) instructs the model in prose to call `verifier-verdict`, but there is no code-level guardrail. Spawned sessions exit after 1 assistant turn (esp. on pi 0.80.6) without ever calling `verifier-verdict`. `gather()` in `src/spawn/orchestrator.rs` reads whatever is on disk; a missing verdict silently becomes `status:null`.

3. **No compaction self-recovery**: when compaction fires mid-verification, the session JSONL ends with `type:compaction` and no `agent_end`. `extract_final_output` (`src/acp/parser.rs:154`) only harvests from `agent_end`, which never appears. The verdict is lost.

Current state: `src/prompt/mod.rs` caps only `gitDiff` (`truncate_diff`, line 223) and stderr (`STDERR_CAP_BYTES`, orchestrator.rs:58). `fileEditTimes`, `context`, and policy are uncapped. The spawn/gather path has no verdict-presence check and no compaction-event handling.

## Goals / Non-Goals

**Goals:**
- Rendered verifier prompt stays under a configurable byte budget, which **reduces the likelihood of compaction** firing before a verdict on a normally-sized repo. This is a soft goal: a sufficiently large `gitDiff` or `--context` can still push the prompt over the OS/backend input limit, and a hard ceiling (refusing to spawn when over budget) is a **future enhancement** — not implemented here.
- A verifier that completed its investigation ALWAYS reaches a verdict — even if compaction fired mid-analysis. Compaction is a recoverable event, not a verdict-killer.
- `gather()` never silently extracts null from a session that exited without calling `verifier-verdict`; it re-prompts within `maxTurn` first.
- No regression to the fail-closed invariants (NULL verdict never → APPROVE; missing store → no hash).

**Non-Goals:**
- Cross-round consensus carry (Group D feature gap — separate change if wanted).
- `jewilo GATHER` re-eval subcommand (Group D feature gap).
- pi-side skill injection changes (confirmed NOT the cause — pi does not inject skills into `pi -p` prompts).
- Fixing pi 0.80.6 itself (external; we harden around it).
- Changing the n/m consensus rule or the tamper-evident hash contract.

## Decisions

### D1 — `fileEditTimes` scoped to changed files only
**Choice:** Replace `git ls-files` enumeration with `git status --porcelain -z` (changed/untracked files) for the fileEditTimes block. Add a hard byte cap (`FILE_EDIT_TIMES_CAP_BYTES`, default 8KB) as a secondary guard.

**Implementation note (porcelain parsing):** the code uses `git status --porcelain -z` (NUL-delimited) rather than the default newline-delimited porcelain, because the latter C-quotes pathnames containing spaces/Unicode (`"my path"`) and represents renames as `old -> new`, both of which are ambiguous to parse. With `-z` the records are NUL-terminated and unquoted; a rename is emitted as `XY <new_path>\0<old_path>\0` (new path first, old path second — the second record is consumed and discarded so the rename source does not appear as a spurious changed file). This means arbitrary ASCII pathnames — including ones containing a literal ` -> ` substring — are handled correctly.

**Why over alternatives:**
- *Drop entirely*: loses forensic value (proving edit times for tamper detection). Keep the signal but scope it.
- *Hard cap only*: a repo with 10,000 unchanged tracked files still wastes bytes before the cap; scoping is strictly better.
- Changed files are the forensically relevant set for a verification round; unchanged files add noise.

**Alternative considered:** cap by count (N most-recent). Rejected — mtime without change-status is meaningless for tamper proof.

### D2 — Policy deduplication
**Choice:** When `verifierPromptFile` is set in config, the embedded `VERIFIER_POLICY` constant in `DEFAULT_TEMPLATE` SHALL be skipped (the custom file replaces it, not supplements it). When `verifierPromptFile` is null, the embedded policy is used as today.

**Why:** both files are near-identical 31KB policy text starting with `<_unfold.md>`. Currently prepended raw (`prepend_custom`, mod.rs:149) AND embedded via `concat!` (mod.rs:92-97) = 62KB wasted. The custom file is meant to *override* policy, not duplicate it.

**Alternative considered:** merge/diff the two at render time. Rejected — non-deterministic and fragile. Override semantics are clear and testable.

### D3 — Context byte cap
**Choice:** `--context` input capped at `contextMaxChars` (default 20000). Over-cap → truncated with indicator, mirroring `truncate_diff`.

**Why:** unbounded `--context` is a latent bloat source observed in sampled goals. Cap mirrors the existing diff-cap pattern.

### D4 — Rendered-prompt budget warning
**Choice:** After rendering, if total prompt bytes exceed `promptBudgetBytes` (default 50000), emit `eprintln!` warning with the breakdown (policy / fileEditTimes / gitDiff / context / goal). Does NOT block spawn — operator decides.

**Why over hard fail:** a legit large-diff verification should still proceed; the operator gets visibility. Hard fail would block valid use cases.

### D5 — Verdict enforcement in gather (within-round re-prompt)
**Choice:** After `gather()` reaps a child, if no verdict.json exists OR `status == null` AND `turnsUsed < maxTurn`, the orchestrator SHALL re-prompt the same session (sid reuse) with a minimal verdict-nudge prompt: "You have completed your investigation. Register your verdict NOW via: verifier-verdict approve --notes '...' OR verifier-verdict reject --notes '...'". Up to `maxTurn - turnsUsed` nudges per slot per round.

**Scope (universal):** this enforcement applies to BOTH fresh (`spawn_round`) and resume (`spawn_resume`) rounds — the verifier-spawn spec "Verdict is enforced after child exit" carries no round-type carve-out. A resume-round child that exits with no verdict is re-prompted on the same sid, exactly like a fresh round. The transport guard (Stdin only) matches across both paths: GoalFile custom adapters are not designed for multiple nudge resumes and are scoped out consistently.

**Why over fresh re-spawn:** sid reuse preserves the investigation context; fresh spawn throws away completed analysis and re-incurs the bloat/compaction risk. The nudge is cheap (small prompt) and targets the exact failure (model forgot the final step).

**Alternative considered:** hard-fail the round on first null. Rejected — that's the current broken behavior.

### D6 — Compaction as a first-class recoverable event
**Choice:** The ACP parser SHALL detect `{"type":"compaction",...}` events in the stream. When compaction is observed and the session ends without `agent_end`/verdict, the orchestrator SHALL auto-resume the same sid with a compaction-aware recovery nudge prompt (`COMPACTION_RECOVERY_NUDGE_PROMPT`, distinct from the generic `VERDICT_NUDGE_PROMPT` used for D5 enforcement) to harvest the verdict. The recovery nudge tells the verifier that compaction occurred, its prior investigation is preserved in the resumed session, and it must register its verdict immediately. This is "must always be able to compact itself."

**Turn accounting (does recovery consume a turn?):** yes. Recovery runs INSIDE the same nudge loop as D5 and is gated by the `turnsUsed < maxTurn` check at the top of each iteration. The recovery resume increments `turnsUsed` by one (via `reap_nudge_child`'s meta update), exactly like a plain verdict-enforcement nudge. Consequently, if a slot has already exhausted its turn budget, no recovery is attempted and the slot fails closed to null. The hard cap "at most one recovery resume per slot per round" is enforced separately by the `recovery_attempts == 0` predicate.

**Why:** compaction is the confirmed kill mechanism for Groups B+C. The investigation is done; only the verdict emission was lost. Resuming post-compaction with a tiny nudge is the minimal recovery.

**Alternative considered:** pre-emptively shrink prompt to avoid compaction entirely. That's D1-D4 (necessary but not sufficient — a huge diff can still trigger compaction). D6 is the safety net.

### D7 — Strengthened final-step in prompt template
**Choice:** `default_template.txt` and `default_resume_template.txt` SHALL end with an explicit fenced bash block showing the exact `verifier-verdict` invocation pattern (approve/reject with notes), not prose. This is a prompt-level complement to D5/D6, not a substitute.

**Why:** reduces the chance the model skips the verdict step. Belt + suspenders with the code-level enforcement.

## Risks / Trade-offs

- **[D1 loses full fileEditTimes forensic trail] → Mitigation:** changed-files set (`git status --porcelain`) is the forensically relevant set for a round; unchanged files do not strengthen tamper proof. The mtime signal for changed files is preserved.
- **[D5 within-round nudges increase wall-clock per round] → Mitigation:** bounded by `maxTurn - turnsUsed`; nudge prompts are tiny (no compaction risk). Net faster than failing the round and re-running RESUME.
- **[D6 post-compaction resume may itself compact] → Mitigation:** the nudge prompt is minimal; if a second compaction+exit occurs, treat as exhausted and fail-closed null (do not loop infinitely). Hard cap: 1 compaction-recovery resume per slot per round.
- **[D2 override semantics may surprise operators who append custom policy] → Mitigation:** document clearly; the custom file is a *replacement*. Operators who want both must concatenate manually before pointing `verifierPromptFile` at the result.
- **[pi 0.80.6 1-turn-exit regression may defeat D5/D6 nudges too] → Mitigation:** D5/D6 re-prompt via sid resume which is a different code path from initial spawn; if 0.80.6 also breaks resume, fall back to fail-closed null + clear error in verdict.json reason field (not silent null). Track 0.80.6 separately as external dependency.
