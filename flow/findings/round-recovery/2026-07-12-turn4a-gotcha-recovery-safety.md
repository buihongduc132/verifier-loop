# Gotcha Coverage Review — Round Recovery (reviewer-a)

> Date: 2026-07-12
> Reviewer: gotcha-reviewer-a (task #5)
> Scope: surface GAPS the original explore (turns 1-4, LD1/LD2, OT1-OT4) MISSED.
> Rule: do NOT re-analyze; only edge cases / invalid assumptions / failure modes / unconsidered scenarios.
> Format per gotcha: **what / why-missed / severity / mitigation**.
> Rank: 1=YAGNI (drop or defer) → 5=sophisticated (needs real engineering). Severity is independent of rank.

---

## RANK 3+ (grouped at top — these must be addressed before `add-round-recovery` becomes a proposal)

### G1 — PID-reuse: the locked `kill orphan via PID file` primitive can murder an innocent process
**What:** LD1 freezes "kill orphan (via persisted PID file)" as step 1 of recovery. A PID file is a stale integer after the child exits, after a reboot, or after the kernel recycles the PID. `kill(<recycled-pid>)` then targets an **unrelated process** — could be the user's editor, a database, or jewilo itself.
**Why missed:** Turn 2 called the PID file "a 4-byte file" and OT2 lists "stale/reused" as one sub-bullet among lifecycle questions, but LD1 was **locked before OT2 was resolved**. The safety of the KILL primitive was never evaluated separately from the lifecycle question. The explore reasoned about correctness of the verdict layer (well-analyzed) and hand-waved the OS-process layer.
**Severity:** HIGH — arbitrary foreign-process kill. Fail direction is dangerous (kill), not safe.
**Mitigation (rank 4):** Do NOT kill on PID alone. Verify identity before kill: read `/proc/<pid>/stat` start-time + `/proc/<pid>/cmdline` and compare to values captured at spawn (store `{pid, starttime, cmdline_hash}` in the pid file, not just pid). Or use `pidfd_open()` + `pidfd_send_signal()` (Linux 5.3+) which is reuse-safe by construction. Simpler still: drop the PID-kill, treat scenario-B orphans as "wait for verdict.json or time out, then re-spawn" (the explore's poll option) — accept token waste as the price of not killing strangers.
**Applies to:** LD1 (locked), OT2.

### G2 — The PID in the file is the WRAPPER, not the model-calling grandchild
**What:** Adapters (`src/acp/adapters.rs`) spawn `pi` / `hermes` / `acpx`, frequently through wrappers (`mise`, `nvm`, shell shims). The child jewilo forks is the wrapper; the actual token-burning model client is a **grandchild** with a different PID. `kill(wrapper-pid)` terminates the wrapper but reparents the grandchild to init — the EXACT orphan-leak RECOVER was built to fix.
**Why missed:** Turns 1-3 model the child as a single process. The adapter layer (`build_spawn_command`) was read for resume-by-SID capability, never for the process-tree shape it produces. `kill_on_drop` analysis (turn 1) likewise treats "child" as atomic.
**Severity:** HIGH — defeats the purpose of the recovery primitive in the common adapter case (pi-via-mise is the default pi install path on this very machine).
**Mitigation (rank 4):** Kill the whole **process group / session** the child leads, not the child PID. Requires spawning each verifier with its own pgrp/session (`setsid` / `process_group(0)` in `pre_exec`) — which ALSO fixes the turn-1 orphan-leak root cause (currently children inherit jewilo's pgrp). Then `kill(-pgid, SIGTERM)` reaps the tree. As a bonus this makes scenario-A vs scenario-B converge. If pgrp kill is infeasible, walk `/proc/<pid>/task/*/children` recursively before killing.
**Applies to:** LD1, OT2, and (root-cause) the turn-1 orphan-leak finding.

### G3 — Concurrent RECOVER / RESUME / dual-RECOVER is unguarded
**What:** Nothing prevents two recovery invocations against the same goal simultaneously (outer agent retries; human + agent; two agents on the same goal). Both read the same pid file, both try to kill the same orphan, both `pi --session <sid>` resume → **two processes writing the same session file + same verdict slot**. Turn 3 *proved* concurrent-same-SID is a silent corrupting race at the process level; that hazard was never lifted to the RECOVER-invocation level.
**Why missed:** Turn 3's probe isolated the same-SID race as a property of pi, then dropped it. The explore assumed single-threaded recovery throughout. No `flock`/lockfile on the goal dir was considered.
**Severity:** HIGH — corrupt session file, dual verdict writers, undefined consensus input. The first-write-wins verdict layer survives, but the resumed SIDs are poisoned.
**Mitigation (rank 4):** `flock` an exclusive lock on `store_dir/<goalId>/.lock` for the duration of any RECOVER/RESUME/NEW that mutates state.json or spawns children. Second invocation exits with a clear "goal busy" code. This is independent of command-vs-flag (G11) — both need it.
**Applies to:** OT1, OT2, LD1.

### G4 — RECOVER's fresh-spawn slots MUST re-render the prompt → OT3 is internally incoherent
**What:** LD1 says "Null slots with no live orphan spawn fresh." A fresh spawn requires a **full prompt render from the frozen snapshot** — there is no resumed SID to nudge. But OT3 leans toward "do NOT re-render / do NOT re-capture." So a single RECOVER pass produces a round with **mixed prompt provenance**: resumed-SID slots get a <2KB nudge (no render), fresh-spawn slots get a full render from the snapshot. The explore treats "re-render the prompt" as monolithic and never splits these two cases.
**Why missed:** The verdict-nudge path (resumed SID) and the fresh-spawn path were discussed in different turns (3 vs 2) and never reconciled at the prompt-layer level. The frozen-snapshot-per-round invariant was inherited from the original design without checking that RECOVER preserves it.
**Severity:** HIGH (audit-trail incoherence) — a single round's verdicts are backed by two different prompt shapes; the completion hash is tamper-evident but the semantic meaning of "round N approved" is muddied.
**Mitigation (rank 3):** Make per-slot prompt provenance explicit in `meta.json` (`{slot, source: "original"|"resumed-sid"|"fresh-spawn", prompt_hash}`). Decide deliberately: either (a) fresh-spawn slots in RECOVER use the frozen snapshot (preserve invariant, accept stale context — see G5), or (b) RECOVER refuses to fresh-spawn and instead tells the user "round N has slots that never started; use RESUME N+1" (push the mixed case out of RECOVER entirely).
**Applies to:** LD1, OT3.

### G5 — Stale-snapshot verdicts for fresh-spawn slots when RECOVER runs long after round start
**What:** Time between round-N start and RECOVER can be hours/days (outer agent killed overnight, human away). A fresh-spawn verifier renders from the frozen snapshot — code that no longer matches HEAD. Its verdict is "correct for stale state, wrong for current state." The hash is still tamper-evident; the **verdict semantics are misleading**. The explore frames staleness as a "tension," not a correctness defect.
**Why missed:** The frozen-snapshot invariant was designed for the normal flow (round runs to completion in seconds/minutes). RECOVER stretches the snapshot lifetime indefinitely. No TTL was considered.
**Severity:** MED-HIGH — semantically wrong APPROVE possible, and it's undetectable from the hash alone.
**Mitigation (rank 3):** (a) Snapshot TTL: if `now - snapshot_captured > X`, RECOVER refuses fresh-spawn and instructs RESUME N+1. (b) Store the git ref / tree-hash the snapshot was captured against in meta; RECOVER re-resolves and refuses if the ref no longer exists (rebase/rewrite). (c) For uncommitted-tree snapshots, store a content hash and refuse fresh-spawn if the working tree no longer matches — forcing the user to acknowledge drift.
**Applies to:** OT3, G4.

### G6 — Kill signal & escalation policy unspecified; child signal behavior unprobed
**What:** LD1 says "kill orphan" without specifying signal. If SIGTERM and the child has no handler (turn 1 showed jewilo has none; **child signal behavior was never probed**), the signal may be ignored or lost in a blocking syscall → orphan persists → resumed child collides with it. If SIGKILL with no grace, the child can't flush its session file cleanly (relevant to G8). No SIGTERM→grace→SIGKILL escalation defined.
**Why missed:** Turn 1 carefully characterized jewilo's signal handling and then assumed the same applies to children without probing. The kill primitive was frozen in LD1 as a single word ("kill").
**Severity:** MED-HIGH — a non-landing kill silently re-introduces the orphan-collision hazard the explore worried about in turn 2.
**Mitigation (rank 3):** Define an explicit escalation: `SIGTERM` → wait `kill_grace_sec` (default ~10s) → `SIGKILL`. Probe `pi`/`hermes`/`acpx` SIGTERM behavior before locking. Document that SIGKILL may leave the session JSONL with an unflushed tail (link to G8).
**Applies to:** LD1.

### G7 — Resumed-SID nudge after a truncated ASSISTANT message (LD2 under-specified)
**What:** LD2 "accepted the wrinkle" but the wrinkle as stated was about a truncated JSONL **file tail**. The deeper issue: if the orphan was killed **mid-generation**, the session file contains a partial ASSISTANT turn (content cut off, no closing JSON, tool_use without tool_result). Resuming sends the verdict-nudge after a malformed prior turn → the model may treat the partial turn as complete, hallucinate the missing tail, or refuse. This is distinct from "truncated file tail" (which is a parse concern); this is "truncated message semantics" (a conversation-state concern).
**Why missed:** LD2 collapsed two truncation modes into one. The probe in turn 3 killed mid-turn and resumed "cleanly" — but did not inspect whether the resumed model's behavior was sane, only that pi didn't crash.
**Mitigation (rank 3):** Before nudging, validate the last assistant message in the session JSONL is structurally complete (has `stop_reason`, balanced tool_use/tool_result). If partial, roll the conversation back to the last complete USER or complete ASSISTANT turn before sending the nudge. Emit a meta flag `recovered_from_partial=true` for auditability.
**Applies to:** LD2.

### G8 — Runtime interaction round-recovery × compaction-recovery INSIDE a resumed SID
**What:** OT4 frames the compaction-recovery collision as a **vocabulary** problem. The real hazard is runtime: a resumed SID may have consumed most of its context budget before being killed. The <2KB verdict-nudge lands in a near-full context window → pi triggers **compaction-recovery** (the other change) mid-nudge. Two recovery mechanisms now compose inside one process with no defined interaction. Worst case: compaction drops the very context the verdict depended on → garbage verdict, or verdict fails → null → fail-closed (safe but silent).
**Why missed:** OT4 was raised as a naming concern. The explore never traced what happens when the resumed SID's own recovery kicks in during the nudge.
**Severity:** MED-HIGH — silent degraded verdict quality, or spurious nulls.
**Mitigation (rank 3):** (a) Before resuming by SID, read `meta.turnsUsed` and the SID's session size; if near the compaction threshold, **do not resume** — fresh-spawn instead. (b) Explicitly define composition: "round-recovery respects compaction-recovery; if compaction fires during the nudge, accept the resulting verdict." Document the audit implication. Either choice is fine; leaving it undefined is not.
**Applies to:** LD1, OT4 (runtime half).

### G9 — Outer-agent / machine contract absent from the command-vs-flag choice
**What:** Turn 1 establishes the common jewilo driver is an **outer agent**, not a human. OT1 (command vs flag) is debated purely around *human reflex*. But an outer agent has no reflexes — it needs an **explicit machine-readable contract**: exit codes / structured output distinguishing "round N needs RECOVER" vs "round N complete-and-failed, needs RESUME N+1" vs "round N already at consensus." Neither a separate command nor a flag addresses this.
**Why missed:** The explore anchored on human UX. The idempotent framing in turn 2 gestures at auto-detection but OT1 reduced the design space to two human-facing options.
**Severity:** MED — the primary call path gets no help.
**Mitigation (rank 3):** Add a `STATUS <goalId>` (or `--json` on existing commands) returning `{round, state, needs: "recover"|"resume"|"done", slots: [...]}`. Make RECOVER and RESUME themselves idempotent and emit structured terminal state. Decouple the human-UX choice (OT1) from the machine contract.
**Applies to:** OT1.

### G10 — Classic PID-file CVEs (symlink, pre-create, EPERM)
**What:** The pid file lives at a predictable path (`rounds/N/vN/pid`). PID files are a well-trodden CVE class: (a) **symlink attack** — attacker pre-creates `pid → /etc/important_file`, jewilo overwrites it; (b) **self-kill** — attacker pre-creates `pid` containing jewilo's own PID, RECOVER kills itself; (c) **EPERM** — recovery running as a different user can't kill the orphan, silently fails; (d) **TOCTOU** — check-then-kill window between identity verification and `kill()`.
**Why missed:** The explore treated the pid file as a benign 4-byte artifact. Security thinking was concentrated on the verdict/tamper layer (separate change), not the recovery OS layer.
**Severity:** MED-HIGH in multi-user contexts; LOW single-user single-host (but this repo already documents a threat model acknowledging single-host limitations — see THREAT-MODEL.md reference in AGENTS.md).
**Mitigation (rank 3):** Write pid files with `O_CREAT|O_EXCL|O_NOFOLLOW`, reject if the file already exists, refuse to follow symlinks. Verify the directory is owned by the current user and not world-writable. On kill, re-verify identity immediately before the syscall (narrow TOCTOU). Cross-user recovery: emit an explicit error, do not silently no-op.
**Applies to:** OT2, LD1.

### G11 — Crashed-CHILD (child dies, jewilo alive) is not enumerated as a scenario
**What:** Scenarios A and B are both "*jewilo* dies." The inverse — child verifier crashes/exits non-zero while jewilo is alive — is never enumerated. In that case: pid file lingers pointing at a dead/recycled PID; verdict may be null forever; `gather()` waits for a process that's gone. Is this RECOVER's job, or gather()'s, or neither?
**Why missed:** The two-scenario framing (A/B) in turn 2 was clean and persuasive, and the rest of the explore built on it. The third axis (which process died) was never added.
**Severity:** MED — undefined behavior in a realistic case (OOM kill of a child, child panic, adapter crash).
**Mitigation (rank 3):** Add scenario C explicitly: child died, jewilo alive. Define: gather() detects child exit with null verdict → treat slot as recoverable; pid file cleaned up by gather() on child exit (this also answers part of OT2's "who removes"). RECOVER then sees no pid file + null verdict → fresh-spawn path. Document that a non-zero child exit is logged but does NOT auto-trigger RECOVER (that's the user/agent's call via G9's STATUS).
**Applies to:** OT2, LD1, overall scenario model.

---

## RANK 2 (worth noting; defer or cheap-fix)

### G12 — Auto-detect third option for OT1 collapses the binary
**What:** OT1 is framed as "separate command vs flag." A third option — RESUME detects round state and does the right thing (null verdicts + live orphan → recover; complete-and-failed → advance) — removes the user-reflex hazard entirely. The idempotent framing in turn 2 already implies this.
**Why missed:** Binary framing in OT1.
**Severity:** LOW (ergonomic).
**Mitigation (rank 2):** At minimum, make `RESUME <id>` print "round N has null verdicts; did you mean RECOVER?" instead of silently advancing. Full auto-detect is YAGNI for now.
**Applies to:** OT1.

### G13 — User reflex is symmetric; a separate command doesn't fully solve it
**What:** "Separate command is safer" (OT1) only addresses typing RESUME-when-you-meant-RECOVER. The inverse — typing RECOVER-when-you-meant-RESUME — is equally easy and equally silent (RECOVER idempotently does nothing useful on an already-failed round). Two wrong reflexes exist, not one.
**Why missed:** OT1 argued one direction.
**Severity:** LOW.
**Mitigation (rank 2):** Both commands should sanity-check precondition and warn ("RECOVER: round N is already complete; use RESUME to advance" / "RESUME: round N has null verdicts; use RECOVER first"). Cheap, high value.
**Applies to:** OT1.

### G14 — Naming collision is also CODE identifiers, not just docs
**What:** OT4 is about vocabulary in prose. The same collision happens in Rust: two changes (`fix-prompt-bloat-and-compaction-recovery`, `add-round-recovery`) both likely introduce `recover`/`recovery`-prefixed symbols in the same crate. No collision at compile time (modules), but grep/maintainability/IDE-go-to-definition all suffer.
**Why missed:** OT4 stayed at the doc level.
**Severity:** LOW-MED.
**Mitigation (rank 2):** Pick a prefix convention now: `round_recover::*` (this change) vs `compaction_recover::*` (other change) vs `process_recover::*` (if ever). Apply in module paths, not just prose.
**Applies to:** OT4.

### G15 — Recovery depth unbounded
**What:** If the resumed-by-SID child is *also* killed (second SIGTERM during RECOVER), the next RECOVER repeats the cycle. No max-recover-attempts, no cycle detection. An outer agent in a retry loop could spin this indefinitely, burning tokens.
**Why missed:** Single-shot recovery framing.
**Severity:** LOW-MED.
**Mitigation (rank 2):** Record `meta.recover_count` per slot; cap at N (default 2-3); require explicit `--force` to exceed. Fail-closed to "use RESUME N+1."
**Applies to:** LD1.

### G16 — Snapshot may be GONE at recover time
**What:** OT3 debates re-capture vs no-re-capture. Neither branch handles "the snapshot file itself is missing" (tmp purge on reboot, outer agent `rm`, git GC of the referenced ref, /tmp noexec cleanup). RECOVER reads a missing snapshot → either errors opaquely or silently re-captures (violating OT3's lean).
**Why missed:** Assumed the snapshot is durable because round dirs are durable. Snapshot may live elsewhere (tmp, git object db).
**Severity:** MED.
**Mitigation (rank 2):** Store snapshot content hash + git ref in meta at round start. On RECOVER, if the snapshot is unreadable, fail loudly with "snapshot for round N missing; cannot recover — use RESUME N+1." Do not silently re-capture.
**Applies to:** OT3.

### G17 — pid-file write atomicity / fork-vs-write window / fsync
**What:** "Written at spawn time, before gather" leaves a window between child fork and pid write where jewilo death → orphan with no pid file (undetectable). Partial write (kill mid-write) → parse error. No fsync → power-loss loss.
**Why missed:** Treated as a trivial write.
**Severity:** LOW (small windows) but compounds with G1/G10.
**Mitigation (rank 2):** Write pid file BEFORE fork where possible (impossible — pid unknown pre-fork), or write to temp + `rename()` atomically, fsync the dir. Accept the pre-write-window as a known undetectable orphan case (document it).
**Applies to:** OT2.

---

## RANK 1 (YAGNI / clarity only)

### G18 — Four overlapping user-facing terms hurt discoverability
**What:** NEW / RESUME / RECOVER / compaction-recovery. A user who doesn't know RECOVER exists will never use it. Help-text discoverability and "did you mean" are unconsidered in OT1/OT4.
**Why missed:** Vocab treated as internal-clarity problem, not UX-discoverability.
**Severity:** LOW.
**Mitigation (rank 1):** `--help` cross-references; error messages that suggest the right command. YAGNI: no wizard/introspection UI needed yet.
**Applies to:** OT1, OT4.

### G19 — "process-recovery" introduced in OT4 but undefined
**What:** OT4 lists three terms including "process recovery" without defining it. Is it distinct from round-recovery? Same thing? The ambiguity is itself a gap.
**Why missed:** Term dropped in without a referent.
**Severity:** LOW (clarity).
**Mitigation (rank 1):** Either define it (e.g., "jewilo itself crashed and was restarted by the outer agent — round-recovery is the mechanism that handles this") or drop the term.
**Applies to:** OT4.

### G20 — Tamper-detection: killed+resumed verdict is indistinguishable from a normal one in the receipt log
**What:** The tamper-hardening change adds a hash-chained receipt log. A verdict produced by a killed-then-resumed SID looks identical to a normal verdict. Desired? Probably (recovery is legitimate), but the audit story is unstated.
**Why missed:** Recovery and tamper-hardening were analyzed in separate changes.
**Severity:** LOW.
**Mitigation (rank 1):** YAGNI for now. If audit ever needs it, add an optional `meta.recovered=true` flag (already proposed in G7). Do not block on this.
**Applies to:** LD1, cross-change interaction.

---

## Summary table

| # | Gotcha | Dec. | Severity | Rank | Blocks proposal? |
|---|--------|------|----------|------|------------------|
| G1 | PID-reuse kills innocent process | LD1/OT2 | HIGH | 4 | **YES** |
| G2 | PID is wrapper, not grandchild | LD1/OT2 | HIGH | 4 | **YES** |
| G3 | Concurrent RECOVER/RESUME unguarded | OT1/OT2 | HIGH | 4 | **YES** |
| G4 | Fresh-spawn slots must re-render → OT3 incoherent | LD1/OT3 | HIGH | 3 | **YES** |
| G5 | Stale-snapshot verdicts for fresh spawns | OT3 | MED-HIGH | 3 | **YES** |
| G6 | Kill signal & escalation unspecified | LD1 | MED-HIGH | 3 | yes |
| G7 | Truncated assistant message on resume (LD2 under-spec) | LD2 | MED | 3 | yes |
| G8 | round-recovery × compaction-recovery runtime clash | LD1/OT4 | MED-HIGH | 3 | yes |
| G9 | Outer-agent machine contract missing | OT1 | MED | 3 | yes |
| G10 | PID-file CVEs (symlink/self-kill/EPERM) | OT2 | MED-HIGH | 3 | yes |
| G11 | Crashed-child scenario not enumerated | OT2/LD1 | MED | 3 | yes |
| G12 | Auto-detect third option for OT1 | OT1 | LOW | 2 | no |
| G13 | User reflex is symmetric | OT1 | LOW | 2 | no |
| G14 | Code-identifier collision (not just docs) | OT4 | LOW-MED | 2 | no |
| G15 | Recovery depth unbounded | LD1 | LOW-MED | 2 | no |
| G16 | Snapshot may be gone at recover time | OT3 | MED | 2 | no |
| G17 | pid write atomicity / fsync | OT2 | LOW | 2 | no |
| G18 | Four overlapping terms / discoverability | OT1/OT4 | LOW | 1 | no |
| G19 | "process-recovery" undefined | OT4 | LOW | 1 | no |
| G20 | Tamper-detection gap for resumed verdicts | LD1 | LOW | 1 | no |

## Top-line callouts

1. **LD1 was locked with an unsafe kill primitive.** The shape is right; the kill step is not. G1+G2+G6+G10 together mean "kill orphan via PID file" is under-specified in four independent ways. Resolve before drafting `add-round-recovery`.
2. **OT3 is internally incoherent given LD1.** Fresh-spawn slots force prompt re-render; the "do not re-render" lean can't hold as stated. G4+G5 must be reconciled.
3. **The explore proved same-SID concurrency unsafe (turn 3) then never protected against it at the RECOVER level (G3).** A goal-dir lock is mandatory and cheap.
4. **The whole design optimizes for human reflex (OT1) while the primary caller is an outer agent (G9).** Add a machine contract.
5. **Scenario model is missing a third axis** — child-dies-while-jewilo-lives (G11). The A/B framing is incomplete.

Eleven gotchas (G1-G11) should be closed before `add-round-recovery` becomes a proposal. The rest are deferrable.
