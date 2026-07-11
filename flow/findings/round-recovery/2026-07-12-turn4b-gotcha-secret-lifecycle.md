# Gotcha Coverage Review — Round-Recovery Explore (teammate gotcha-reviewer-b)

> Scope: surface GAPS the original 4 conclusions missed. Not a re-analysis.
> Target conclusions: (1) kill-behavior table, (2) poll-vs-respawn tradeoff,
> (3) orphan-collision "fine for correctness", (4) truncated-session-file accepted.
> Rank scale: 1 = YAGNI / minor → 5 = sophisticated / architecture-breaking.
> **Rank 3+ listed first**, then rank ≤2.

## Evidence basis for the headline gap

`src/spawn/orchestrator.rs` L175 & L286: `mint_verifier_secret(...)` is called
**unconditionally** on both `spawn_round` and `spawn_resume`. It calls
`verdict::mint_and_pin_pubkey` (verdict/mod.rs L176) which:
- generates a fresh Ed25519 keypair,
- atomically pins the **verifying** key to `rounds/N/vN/verifier-pubkey.json` (first-write-wins, immutable, returns `AlreadyPinned` if re-called),
- returns the **signing** key so the orchestrator can inject it into the child env (`VERIFIER_LOOP_VERIFIER_SECRET`).

The signing key is **never persisted to disk** (verdict/mod.rs L172: "it is NEVER
persisted to disk by this function"). `consensus::evaluate` (consensus/mod.rs L135):
"If a pubkey IS pinned, the verdict's signature MUST verify against it" — i.e. an
unsigned APPROVE on a pinned slot is rejected and does NOT count toward `n`.

**Implication:** the only process that can produce a verdict that counts for a given
slot is the one holding the secret in its env — the original child. Kill that child
(or kill the jewilo that holds the secret in memory before passing it on) and the
slot can no longer produce a *countable* verdict via any fresh/resumed process.

---

## Rank 5 — architecture-breaking

### G5-1 · The signing secret dies with jewilo; Shape 2 (LD1) cannot produce a countable verdict for any pinned-but-null slot
- **What:** The explore's core premise — "the verdict file IS the resumption contract" and "consensus::evaluate stays unchanged" — is false once tamper-hardening is in scope. The resumption contract is a **signed** verdict whose signature verifies against the slot's pinned pubkey. The signing key is minted per-spawn, passed to exactly one child via env, and never persisted. On jewilo death the secret is gone.
  - Shape 2 branch "kill orphan → resume by SID → nudge": the resumed `pi --session` process is a NEW process with no `VERIFIER_LOOP_VERIFIER_SECRET`. Its `verifier-verdict` call either refuses to sign or emits an unsigned APPROVE → consensus signature-gate rejects → slot stays null.
  - Shape 2 branch "no orphan → spawn fresh → gather": `mint_verifier_secret` returns `AlreadyPinned` (pubkey was pinned at the original spawn before the kill) → spawn fails closed → RECOVER aborts.
  - Even the headline "salvage the orphan's reasoning" is broken: killing the orphan destroys the **only** valid signer for that slot.
- **Why missed:** The explore read `verdict/mod.rs` for `register_signed_approve`/first-write-wins (references.md lists it) but never traced the secret's lifecycle or noted "NEVER persisted to disk." It reasoned about verdict.json as a plain status field, not as a signed record gated by an in-memory credential. The probe (turn 3) confirmed SID persistence but never probed secret/key persistence.
- **Severity:** CRITICAL. LD1 is unimplementable as written against the landed tamper-hardening; the one lock that claims "consensus::evaluate stays unchanged" is the thing that's actually broken.
- **Mitigation:** Either (a) persist the per-slot secret to disk (`rounds/N/vN/secret`) so a resumed/re-spawned child can re-sign — **but see G4-2, this is a threat-model regression**; (b) restrict RECOVER to Shape 1 (poll-only; never kill the orphan — only the live orphan can sign); (c) allow RECOVER to fall through to RESUME (round N+1, fresh slots, fresh keys) when a pinned-but-null slot has no live signer. Any of these revises LD1; the lock must be reopened.

## Rank 4 — high

### G4-1 · "Resumption contract = verdict.json" understates the contract (signed + timestamped)
- **What:** Consensus counts a verdict as matching only if it has `registered_at` AND a signature that verifies (consensus/mod.rs L49-50, L132, L135). A non-null but unsigned/null-timestamp verdict is rejected. So "read verdict.json, non-null → KEEP" (Shape 2 flowchart) is insufficient: a recovered slot must produce a record meeting *both* fields. This is the framing error behind G5-1.
- **Why missed:** The explore asserted the contract shape ("verdicts + signature + round" in the completion-hash line) yet still drew a flowchart keyed only on `verdict != null`. The signature requirement was acknowledged in prose and ignored in the control flow.
- **Severity:** HIGH (the control-flow shape that was locked does not match the contract it cited).
- **Mitigation:** Flowchart must key on "signed APPROVE present" not "verdict non-null". Re-derive every RECOVER branch against the signed-record schema.

### G4-2 · Persisting the secret to "fix" G5-1 is a threat-model regression
- **What:** The natural fix for G5-1 is to write the signing key to `rounds/N/vN/secret`. But THREAT-MODEL.md currently scopes forging to "a process with write access to `~/.verifier-loop/` AND the ability to read a V*'s env." Putting the secret on disk widens that to "any process with read access to the store" — strictly weaker. The deterrent/detection layer is weakened precisely for the recovery path.
- **Why missed:** The explore never considered the secret exists, so it never weighed this tradeoff. OT2 (PID-file lifecycle) is the closest open thread, but it covers the PID file only.
- **Severity:** HIGH (silent security downgrade if G5-1 is patched naïvely).
- **Mitigation:** If secrets must be persisted, scope them (0600, shred on first verdict write, short-lived), and update THREAT-MODEL.md explicitly. Prefer G5-1 option (b)/(c) to avoid persisting at all.

### G4-3 · PID recycling: RECOVER may kill an unrelated process
- **What:** OT2 raises "what if PID is stale/reused" but leaves it open. Between spawn-time PID-file write and RECOVER-time read, the orphan may have died and its PID recycled by an unrelated process. `kill(pid)` then strikes an innocent process — possibly another jewilo, a model backend, or a system daemon. PID + process-name + start-time validation (`/proc/<pid>/stat`, or `kill(pid,0)` + cmdline check) is unspecified.
- **Why missed:** Treated as a lifecycle-cleanup detail rather than a safety hazard. The explore framed the PID file as "4 bytes, cheap."
- **Severity:** HIGH (wrong-process kill = correctness AND host-safety hazard, not just token waste).
- **Mitigation:** Persist `(pid, start_time)` or `(pid, comm)`; validate before kill; refuse to kill on mismatch (fall through to poll/RESUME). Never `kill -9` a PID whose identity isn't confirmed.

---

## Rank 3 — material

### G3-1 · Kill table conflates "SIGINT" with "TTY Ctrl-C"; `kill -INT <pid>` orphans children
- **What:** Conclusion 1's row "Ctrl-C in a TTY → SIGINT → whole pgrp dies" is correct, but it is a *terminal-driver* behavior, not a *signal* behavior. An outer agent doing `kill -INT <jewilo-pid>` sends SIGINT to ONE process; children do not receive it and are orphaned exactly like the SIGTERM row. The table label "SIGINT → children die" is only true for TTY-generated SIGINT.
- **Why missed:** The table's two columns (Scenario / signal) blur mechanism (pgrp broadcast vs single-pid delivery).
- **Severity:** MED-HIGH (outer agents routinely use `kill -INT`; the predicted behavior would be wrong).
- **Mitigation:** Split the row into "TTY-generated SIGINT (pgrp)" vs "kill()-delivered SIGINT (single pid)". Document that only the former reaps children.

### G3-2 · "Children inherit jewilo's pgrp" verified only by rg-absence; grandchild detach unconsidered
- **What:** Conclusion 1's Assumption [A] admits pgrp-inheritance is "verified by rg absence, not by reading every Command builder." Two risks: (a) a dependency or adapter may add `setsid`/`process_group` later, silently flipping the model; (b) more importantly, the *backend* (pi/hermes/acpx) may itself fork+detach long-lived grandchildren (model server, MCP server, telemetry). Such grandchildren escape the pgrp and survive Ctrl-C as orphans even in the "scenario A" case the explore calls clean.
- **Why missed:** The explore equated "the child process jewilo spawned" with "the leaf process doing the work." ACP backends are multi-process.
- **Severity:** MED-HIGH (scenario A is not actually clean if backends detach grandchildren).
- **Mitigation:** Probe whether each adapter's backend daemonizes. If so, RECOVER must expect live grandchildren even after Ctrl-C. Consider spawning each verifier in its own process group/session deliberately (then kill by PGID/SID, not PID).

### G3-3 · Orphan may die of SIGPIPE when jewilo exits, contradicting "keeps burning tokens"
- **What:** When jewilo dies, the read end of the stdout/stderr pipes closes. The orphan's next write to stdout returns EPIPE / raises SIGPIPE. Unless the backend ignores SIGPIPE (Rust/pi typically do; a raw model-server may not), the orphan is killed by its own broken-pipe write — it does NOT "keep running until verifierTimeoutSec." Conclusion 1's scenario-B row ("orphans keep running, keep burning tokens") is therefore backend-dependent, not universal.
- **Why missed:** The explore focused on whether the orphan is *signaled by jewilo's killer* and ignored the secondary death via the lost pipe reader.
- **Severity:** MED (invalidates the cost model that motivates the whole recovery design: if orphans self-terminate on EPIPE, the "token leak" problem may be largely self-healing and RECOVER's poll window is racing a dying orphan).
- **Mitigation:** Probe each adapter's SIGPIPE handling. If EPIPE kills the orphan, scenario B's risk shrinks dramatically and the poll-vs-respawn tradeoff shifts.

### G3-4 · cgroup / systemd / container supervisors override the raw-signal model
- **What:** Conclusion 1 is pure Unix-signal reasoning. Real "driven by an outer agent" deployments (the common case per the repo) run jewilo under systemd, Nomad, Docker, or a CI supervisor. systemd kills the whole cgroup on `TimeoutStopSec` (reaps all descendants regardless of setsid); `docker stop` SIGTERMs PID 1 only and depends on PID 1's forwarding; Nomad alloc kills are cgroup-scoped. The TTY-vs-non-TTY split is irrelevant under a supervisor; what matters is cgroup scope.
- **Why missed:** The table is keyed to interactive-vs-`kill(1)`, not to supervisor semantics.
- **Severity:** MED-HIGH (the orphan scenario may not occur at all under systemd, or may occur even on Ctrl-C under a non-forwarding PID-1 container).
- **Mitigation:** Add a third axis (supervisor) to the kill table. Document cgroup-kill (reaps all) vs single-PID SIGTERM (orphans) vs no-forwarding PID-1 container (children survive Ctrl-C).

### G3-5 · Truncated-session-file wrinkle accepted WITHOUT a probe; failure mode unknown
- **What:** LD2 accepts the truncated-JSONL wrinkle as non-blocking with "no pre-proposal probe required." But pi's actual session-loader behavior on a truncated tail was never determined. If the loader is strict (aborts on a malformed line), resume-by-SID fails closed — Shape 2 produces no verdict. If lenient (truncate to last good line), behavior depends on *which* line was cut. The riskiest acceptance in the entire explore, because it locks "non-blocking" against an unknown.
- **Why missed:** Explicitly skipped ("no pre-proposal probe required").
- **Severity:** MED-HIGH (could silently break the LD1 resume path; the lock forecloses the investigation that would tell us).
- **Mitigation:** Reopen LD2. Probe pi's loader on a hand-truncated JSONL before drafting the openspec change. At minimum, probe both "missing final line" and "partial final line."

### G3-6 · Concurrent / double-RECOVER has no locking model
- **What:** The explore calls RECOVER "idempotent" but never defines concurrency control. Two RECOVERs (user retry, or an outer agent racing) both read null slots, both try to mint (→ AlreadyPinned), both try to kill the same PID, both spawn duplicate children. No flock on the goal dir, no "RECOVER in progress" marker.
- **Why missed:** Idempotency was framed at the *outcome* level (drive round N to consensus), not at the *execution* level.
- **Severity:** MED-HIGH (double-spawn, double-kill, PID-file races).
- **Mitigation:** Add a goal-level lock (flock on `rounds/N/.recover.lock`, or a marker file with the recovering jewilo's PID). Reject concurrent RECOVER; or serialize.

### G3-7 · "Re-spawned child wins deterministically" is inverted
- **What:** Conclusion 3's example shows the re-spawn writing first (t=45s) and the orphan later (t=120s), concluding the re-spawn's verdict wins. But the orphan has a head start (it began at original-spawn time); the re-spawn begins cold at RECOVER time (cold model, fresh context). In the common case the *orphan* finishes first and its verdict wins — meaning the verdict you keep was authored by the orphan you didn't control, not the re-spawn you did. First-write-wins still yields one valid verdict (correctness holds), but the "deterministic in the re-spawn's favor" claim is wrong, and that determinism was the stated basis for "fine for correctness."
- **Why missed:** The example inverted the realistic timing.
- **Severity:** MED-HIGH (the design rationale, not just the outcome, is unsound; auditability of *which* verifier authored the kept verdict is non-deterministic).
- **Mitigation:** Drop the "re-spawn wins" claim. State that first-write-wins yields *a* valid signed verdict but the winner is nondeterministic; if audit needs determinism, kill the orphan before re-spawn (which loops back to G5-1: you then have no signer).

---

## Rank 2 — worth noting

### G2-1 · No shared clock between dead and recovering jewilo → poll window is arbitrary
- **What:** The poll timeout `T = min(verifierTimeoutSec, 60s)` runs on the RECOVER clock. The orphan's elapsed budget (orig_spawn → kill) is unknown to the recovering jewilo. RECOVER may give up and re-spawn (→ G5-1: useless) one second before the orphan would have finished.
- **Why missed:** Treated as a tuning knob, not as a missing-information problem.
- **Severity:** MED.
- **Mitigation:** Persist the spawn timestamp alongside the PID; size the poll window as `verifierTimeoutSec - elapsed`, clamped.

### G2-2 · Rate-limited backends defeat the poll window
- **What:** Per the repo's AGENTS.md, zhipu/GLM coding-plan keys have cd=30s–600s cooldowns; bailian/byteplus similar. A rate-limited orphan may not emit its verdict for many minutes — far beyond any reasonable poll `T`. Poll cannot distinguish "about to finish" from "rate-limited, will finish in 10 min."
- **Why missed:** Explore was architecture-level; backend rate-limiting not modeled.
- **Severity:** MED.
- **Mitigation:** Make the poll window adaptive to observed backend latency, or accept RECOVER degrades to RESUME under rate-limit storms.

### G2-3 · "Kill THEN resume" cannot waitpid a reparented child
- **What:** Shape 2 kills the orphan then resumes by SID. But the orphan was reparented to init on jewilo's death; the recovering jewilo is not its parent and cannot `waitpid` it. It can only `kill(pid, 0)`-poll for exit. If RECOVER resumes-by-SID before the SIGKILL takes effect (uninterruptible I/O, slow exit), two processes touch the session file simultaneously — corruption beyond the truncation wrinkle.
- **Why missed:** The kill→resume ordering was stated as sequential without specifying the synchronization primitive.
- **Severity:** MED.
- **Mitigation:** After `kill(pid, SIGKILL)`, poll `kill(pid,0)` until ENOENT/SRCH, *then* resume. Add a timeout fallthrough.

### G2-4 · Completion-hash recipe not re-derived against the tamper-hardened schema
- **What:** The explore asserts "hash inputs are identical" under RECOVER (same round, same verdicts). But it never re-derived the hash recipe against the *signed* verdict schema. If the hash incorporates the pinned pubkey fingerprint or any per-spawn nonce, a recovery that mints a new key (the no-secret path) diverges from a clean round's hash. (Consensus would reject the signature first, so this is subsumed by G5-1 — but the "identical inputs" assertion is unverified.)
- **Why missed:** Stated from memory of the pre-hardening design.
- **Severity:** MED.
- **Mitigation:** Re-derive the hash inputs from `consensus::compute` against the current `VerdictRecord` schema before locking "identical."

### G2-5 · VCC / immediate-compaction adapter mid-state on resume
- **What:** AGENTS.md documents VCC adapters (`__pi_vcc__` markers, branched lineage) and immediate-compaction. An orphan killed mid-compaction, or mid-VCC-branch, may leave a session file in a state `pi --session` resume doesn't gracefully continue — distinct from the "truncated tail" wrinkle (this is *semantically* mid-transition, not byte-truncated).
- **Why missed:** Wrinkle scoped to byte truncation only.
- **Severity:** MED.
- **Mitigation:** Probe resume of a session killed during compaction/VCC branch as part of reopening LD2.

---

## Rank 1 — YAGNI / minor

- **G1-1 · SIGHUP / terminal-close / `nohup`.** Closing a terminal sends SIGHUP to the session leader (pgrp cascade) — behaves like the TTY-SIGINT row via a different signal. `nohup`/`disown` invert it. Worth a row in the table for completeness. Severity: LOW.
- **G1-2 · `registeredAt` drift forensics.** A resumed verdict is timestamped at RECOVER-time, possibly far after round start — no marker that it's a recovery verdict. Forensically confusing, not incorrect (not a hash input). Severity: LOW.
- **G1-3 · `meta.json` / `final-output.txt` non-atomic clobber under collision.** verdict.json is tmp+rename (atomic); meta.json is jewilo-only; final-output.txt is a plain `fs::write`. Under a collision both writers may clobber final-output.txt. Not a consensus input, so non-fatal. Severity: LOW.
- **G1-4 · `rename()` atomicity on network FS.** verdict.json's tmp+rename is atomic on local FS; weaker on NFS (some CI home dirs are NFS). Power-loss/`kill -9` with dirty page cache is the only trigger. Severity: LOW.
- **G1-5 · Rushed-verdict quality on immediate nudge.** "Emit your verdict now" to a verifier mid-tool-call may yield a low-quality verdict. Correctness intact (fail-closed), quality at risk. Severity: LOW.
- **G1-6 · Completion hash may differ across recoveries.** If the set of matching verdicts differs (orphan salvaged +1 vs clean re-spawn), the hash differs for the "same" goal/round. Only a problem if cross-recovery hash-determinism is a contract (it isn't stated to be). Severity: LOW-MED.
- **G1-7 · `kill_on_drop` runs on returned-Err `main`, not on `process::exit`.** Conclusion 1's "internal error → killed via kill_on_drop" row is too glib: a `?`-propagated `Err` through `main() -> ExitCode` drops the runtime (kill_on_drop fires); an explicit `std::process::exit` does not. Severity: LOW-MED.

---

## Summary of impact on the locks

- **LD1 (shape 2)** — NOT implementable as written against landed tamper-hardening (G5-1, G4-1). Must be reopened. Safe fallback is Shape 1 (poll, never kill the only signer) or RECOVER→RESUME fallthrough for pinned-but-null slots with no live signer.
- **LD2 (truncated wrinkle accepted, no probe)** — Reopen; the acceptance forecloses the one investigation that would tell us the risk (G3-5, G2-5).
- **Conclusion 1 (kill table)** — Add a signal-source axis (TTY-pgrp vs `kill()`-single-pid) and a supervisor axis (cgroup/systemd/container). Note SIGPIPE and grandchild-detach (G3-1, G3-2, G3-3, G3-4).
- **Conclusion 2 (poll vs respawn)** — "Re-spawn double-pays" understates: re-spawn *cannot produce a countable verdict* for pinned slots (G5-1/G2-1). The tradeoff table is inverted in the signed case.
- **Conclusion 3 (collision "fine")** — Correctness holds via first-write-wins, but "re-spawn wins deterministically" is inverted (G3-7); audit determinism is lost.
- **OT2 (PID-file lifecycle)** — Promote from "lifecycle cleanup" to "safety hazard" (G4-3); PID recycling can kill unrelated processes.

The single highest-leverage action: **trace the signing-secret lifecycle into the recovery design before any openspec change is drafted.** Everything else is secondary to that.
