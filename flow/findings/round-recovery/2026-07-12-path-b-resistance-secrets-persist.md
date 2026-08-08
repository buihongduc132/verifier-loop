# Task #8 — secrets-persist resistance evaluation (path-b-resistance)

> Scope: quantify friction (lower = less resistance) of the **secrets-persist** recovery
> path vs the **shape-1** (wait-for-verdict, never kill the signer, fall through to
> RESUME N+1) baseline. NOT a correctness argument — only friction.
>
> Definition (restated): write the per-slot Ed25519 signing key to
> `rounds/N/vN/secret` (0600, shred on first verdict write) so a resumed / re-spawned
> child process can re-sign verdicts. Enables shape-2 (kill orphan → resume by SID →
> nudge) AND fresh-spawn inside RECOVER.
>
> Evidence base: `src/verdict/mod.rs`, `src/spawn/orchestrator.rs`,
> `src/bin/verifier_verdict.rs`, `src/store/salt.rs`, `src/crypto/mod.rs`,
> `openspec/changes/add-verifier-tamper-hardening/specs/verifier-identity/spec.md`,
> `THREAT-MODEL.md`, plus the round-recovery explore record (turns 1–4, gotcha-a,
> gotcha-b, open-threads OT1–OT10).

---

## 1. IMPLEMENTATION resistance — score: **4/10**

### Where the code changes land

| File / symbol | Current behavior | secrets-persist delta | LOC |
|---|---|---|---|
| `verdict::mint_and_pin_pubkey` (`src/verdict/mod.rs:176`) | Mints keypair, atomically pins pubkey, **returns SigningKey, never persists** (L172 docstring) | After pin, write `secret` 0600 atomically (tmp + rename, mirror `store::salt_in`); update docstring + remove the "NEVER persisted" comment | ~15 |
| `verdict::write_first_verdict` (`src/verdict/mod.rs`) | Atomic first-verdict write | After successful write, shred (`fs::remove_file`) the slot's `secret` so the vulnerable window closes | ~8 |
| `verdict::read_persisted_secret` (NEW) | — | Read + chmod-verify 0600 + hex-decode the signing key (mirror `read_pinned_pubkey`) | ~15 |
| `spawn::mint_verifier_secret` (`orchestrator.rs:681`) | Always mints fresh; `AlreadyPinned` is fatal on a pinned-but-null slot | On `AlreadyPinned`, fall back to `read_persisted_secret` so RECOVER can re-sign without re-minting | ~10 |
| `bin/verifier_verdict.rs` secret resolution | Reads `VERIFIER_LOOP_VERIFIER_SECRET` from env only | If env missing AND slot has pinned pubkey AND `secret` file exists, load it from disk | ~10 |
| Core subtotal | | | **~58** |

### The hidden, dominating cost: a landed SPEC must change

`openspec/changes/add-verifier-tamper-hardening/specs/verifier-identity/spec.md` "Per-verifier
signing keypair minted at spawn" currently states, normatively:

> The signing key SHALL be injected into V*'s environment as `VERIFIER_LOOP_VERIFIER_SECRET`
> (hex) and **SHALL NOT be persisted to disk by jewilo.**

with an acceptance scenario:

> **AND** no file under `<store>` contains the signing key

Secrets-persist **violates a SHALL NOT in a landed spec** and **breaks an acceptance
scenario**. This is not a code delta — it is a spec change requiring its own openspec
proposal, decisions, and a rewritten scenario. That is the single largest implementation
friction and it lives outside any LOC count.

### Crypto path: plaintext-0600 is correct here — encrypt-at-rest would be theater

A new crypto path (encrypt-at-rest, KDF) is **not** required and would not help. Rationale:

- `store::salt_in` already writes the load-bearing secret (the salt) as plaintext 0600
  with the same single-host threat framing.
- THREAT-MODEL.md (b) explicitly documents that any process with read access to the
  store + env-read of a V* can forge on the same host; a wrapping key would have to
  live on that same host and be readable by the same Mallory, so wrapping only relocates
  the secret, it does not protect it.
- The shred-on-first-verdict-write bounds the vulnerable window to
  `{spawn → first verdict}` — only the slots that RECOVER is trying to salvage.

So: plaintext 0600 + shred is consistent with the existing model. Encrypt-at-rest adds
LOC and a new dependency surface for no security gain in the documented same-host scope.

**Net implementation friction: small core (~58 LOC), but the spec SHALL NOT / scenario
rewrite is the gating cost. Score 4.**

---

## 2. PRODUCT resistance — score: **7/10**

### What secrets-persist unlocks that shape-1 cannot

| Capability | shape-1 | secrets-persist |
|---|---|---|
| Salvage a slot whose orphan is still ALIVE | ✅ (poll for verdict.json) | ✅ |
| Salvage a slot whose orphan **DIED** before writing | ❌ impossible (signer gone) → must RESUME N+1 | ✅ fresh-spawn re-reads persisted secret and re-signs |
| Kill the orphan to stop token burn then still win the slot | ❌ | ✅ (via fresh-spawn / resume-by-SID) |
| Consensus / completion-hash / audit trail | unchanged | unchanged |

The marginal capability is exactly one thing: **dead-orphan slot salvage**. It is real
and shape-1 genuinely cannot do it.

### Kill-safety work is the dominant product cost — and it is MANDATORY

The moment secrets-persist enables shape-2 (kill orphan) and fresh-spawn-in-RECOVER,
the kill step becomes live, and OT7 / gotcha-a G1+G2+G6+G10 / gotcha-b G4-3+G3-2 all
become REQUIRED rather than MOOT. Quantified:

| Kill-safety item | Why mandatory | LOC |
|---|---|---|
| Spawn each verifier in own session (`setsid` / `process_group(0)` in `pre_exec`) | G2/G3-2: PID you forked is the mise/nvm **wrapper**, not the model grandchild; only pgrp/SID kill reaps the tree | ~30–50 |
| Kill primitive with identity verify (`{pid, starttime, cmdline_hash}`) | G1/G4-3: PID recycling → killing an unrelated process; `kill(pid,0)` + `/proc/<pid>/stat` check | ~40–60 |
| `kill(-pgid)` instead of `kill(pid)` | Consequence of the setsid change above | ~10 |
| SIGTERM → grace → SIGKILL escalation policy | G6: a single-word "kill" primitive is under-specified; SIGKILL alone leaves the session JSONL unflushed | ~30 |
| PID file written `O_CREAT\|O_EXCL\|O_NOFOLLOW`, reject symlinks | G10: classic PID-file CVE class (self-kill, symlink, TOCTOU) | ~15 |
| **Kill-safety subtotal** | | **~125–165** |

Under **shape-1, all of the above is zero** — recovery never kills, so it never needs
to be safe about killing. This is the largest single delta between the two paths.

**Net product friction: real marginal capability (dead-orphan salvage) but it forces the
entire kill-safety suite (~125–165 LOC) that shape-1 entirely avoids. Score 7.**

---

## 3. SECURITY resistance — score: **5/10**

### Exact THREAT-MODEL.md scope statements

From section (a):

> The signing key is **never** persisted to disk by `jewilo` (spec: `verifier-identity`).
> … Mallory must additionally **possess the slot's pinned signing secret** to produce a
> verdict that survives consensus signature verification. That secret lives only in V*'s
> process env, not on disk.

From section (b) — the load-bearing scope statement:

> **A process that has write access to `~/.verifier-loop/` AND can read a V*'s
> environment (i.e. `VERIFIER_LOOP_VERIFIER_SECRET`) can still forge that V*'s verdict.**

From the spec acceptance scenario:

> **AND** no file under `<store>` contains the signing key

### Does secrets-persist widen the forging attack surface?

Yes — from a **two-factor** requirement to a **single-factor** requirement:

| | Today | After secrets-persist |
|---|---|---|
| Mallory needs | write to store root **AND** read a V*'s env (`/proc/<pid>/environ`, ptrace, root) | **read** the store root (specifically `<slot>/secret` while the window is open) |
| Bar | "write + env-read" | "store-read" |

This is a strict weakening. Concrete deltas:

- A non-root process that can write the store but cannot ptrace/read another process's
  env **cannot** forge today; **can** forge after.
- The "Fully-compromised A" attack in section (e) currently requires A to read V*'s
  spawned env. After secrets-persist, A only needs read access to its own store tree,
  which it already has. The deterrent degrades from "A must reach into its child's env"
  to "A must read a file it already governs."

### Mitigations baked into the definition (these bound the regression)

- **0600 + per-slot**: limits the read surface to the file owner and root (same scope as
  the salt).
- **Shred on first verdict write**: the `secret` file exists only for
  `{spawn → first verdict}` for slots that reach a verdict; it persists only for slots
  that never verdict — i.e. exactly the slots RECOVER is trying to salvage.
- **Out-of-process V\* path untouched**: the doc's high-trust recommendation ("run V\*
  on a separate host") does not interact with same-host `secret` files, so the
  regression is bounded to the same-host recovery scenario the threat model already
  treats as non-preventive.

### THREAT-MODEL.md — paragraph addition is NOT enough

Three sections must be edited, not annotated:

1. **Section (a)** — restate the deterrent as "raises the bar from any-write to any-read
   while the secret window is open" (strictly weaker than the current "must possess the
   spawn-time secret that lives only in env").
2. **Section (b) scope statement** — rewrite from "write access AND env-read" to
   "read access to the slot dir (during the secret window) OR write + env-read".
3. **Section (e) NOT-guarded list** — add "any process with read access to a slot's
   `secret` file during the open window" alongside the existing env-read vector.

Plus the **spec scenario rewrite** ("no file under `<store>` contains the signing key" →
amended to permit `secret` under recovery with 0600 + shred + window-bounded).

### Is the regression acceptable?

**Yes, conditionally — not silently.** Given the documented "deterrent + detection, not
prevention" framing and the explicit non-goal of preventing a fully-compromised A:

- The deterrent degrades but does not vanish — the window is bounded by shred, the file
  is 0600, and a same-box reader is already in Mallory's camp per section (b).
- The out-of-process-V\* prevention path is unaffected.
- The regression is a **documentation + spec change obligation**, not a code blocker.

It is acceptable **iff** the spec and THREAT-MODEL.md are rewritten to match. As a
silent change it is unacceptable (it would let an operator read the existing
two-factor scope statement and trust it when it no longer holds).

**Net security friction: bounded regression + mandatory 3-section doc rewrite + spec
scenario rewrite. Score 5.**

---

## 4. ADDITIONAL SCOPE unlocked by secrets-persist that MUST be handled

These are NOT optional — secrets-persist makes each of these live. Under shape-1, all
are MOOT (see OT2/OT3/OT7/OT9 in `2026-07-12-open-threads.yaml`).

| # | Item | Trigger | LOC |
|---|---|---|---|
| 4.1 | Snapshot TTL / staleness (gotcha-a G4/G5/G16) | Fresh-spawn reads a stale snapshot → semantically wrong APPROVE (undetectable from the hash). Must store `{git_ref, tree_hash, captured_at}` in meta and refuse fresh-spawn on drift | ~30–50 |
| 4.2 | Prompt re-render in RECOVER (G4, OT3) | Fresh-spawn requires a full prompt render from the frozen snapshot. Reuse the RESUME render path, but tag per-slot provenance (`source: "original"\|"resumed-sid"\|"fresh-spawn"`) in meta for audit coherence | ~15–25 |
| 4.3 | Kill-safety (G1/G2/G6/G10, OT7) | setsid + pgrp-kill + identity verify + escalation | ~125–165 (see §2) |
| 4.4 | PID identity file (G1/G10, OT2) | `{pid, starttime, cmdline_hash}` + `O_CREAT\|O_EXCL\|O_NOFOLLOW` | ~30–50 |
| 4.5 | Concurrent-RECOVER flock (G3, OT6/LD5) | Exclusive flock on `goals/<goalId>/.lock` for NEW/RESUME/RECOVER | ~15–25 |
| 4.6 | RECOVER command + orchestrator branch + cli wiring | The recovery command itself (drives round N to consensus; spawns only null slots) | ~40–60 |
| 4.7 | Spec change `verifier-identity` SHALL NOT → MAY | Permit `secret` under recovery; rewrite acceptance scenario | spec-only (gating) |
| 4.8 | THREAT-MODEL.md 3-section edit | See §3 | doc-only |

### Combined LOC delta (secrets-persist path, end-to-end)

```
Core secret persist/load/shred   ~50–70
Kill-safety suite                ~125–165
Snapshot TTL + provenance        ~30–50
PID identity file                ~30–50
Concurrent flock                 ~15–25
RECOVER command + wiring         ~40–60
─────────────────────────────────────────
Total                            ~290–420 LOC
+ 1 landed-spec change (gating)
+ 1 THREAT-MODEL.md 3-section edit
```

For comparison, **shape-1** end-to-end is a thin collect-only pass (poll verdict.json
for live orphans, else fall through to RESUME N+1): **~40–80 LOC, zero kill code, zero
secret persistence, zero spec changes, zero threat-model edits.**

---

## Scores (1–10, lower = less resistance)

| Dimension | Score | Dominant cost |
|---|---|---|
| IMPLEMENTATION | **4** | Small core (~58 LOC); gating cost is the landed `verifier-identity` SHALL NOT → MAY spec change + acceptance-scenario rewrite |
| PRODUCT | **7** | Unlocks dead-orphan salvage (shape-1 cannot), but drags in the entire kill-safety suite (~125–165 LOC) that shape-1 entirely avoids |
| SECURITY | **5** | Regression is window-bounded and same-host only, and the "deterrent not prevention" framing absorbs it — but it collapses write+env-read into store-read and forces a 3-section THREAT-MODEL.md rewrite + spec scenario rewrite |

---

## One-paragraph verdict

Secrets-persist is a **higher-friction** path than shape-1 across all three dimensions,
but the friction is **unevenly distributed**. The implementation cost of the secret
mechanism itself is small (~58 LOC, no new crypto path needed — encrypt-at-rest would be
theater given the documented single-host threat model), and the security regression,
while real (it widens forging from "write + env-read" to "store-read during the open
window"), is bounded by 0600 + per-slot + shred-on-first-verdict and is absorbed by the
existing "deterrent + detection, not prevention" framing once the spec and
THREAT-MODEL.md are updated. The dominant cost is **product-side**: secrets-persist
unlocks shape-2 and fresh-spawn-in-RECOVER, which makes the kill step live, which forces
the **entire kill-safety suite** (setsid sessions, pgrp-kill, PID-identity verify,
signal escalation, PID-file CVE hardening) plus snapshot-TTL, provenance tagging, and
concurrent-RECOVER locking — roughly 290–420 LOC end-to-end versus shape-1's ~40–80.
The marginal capability over shape-1 is narrow but real: salvaging a slot whose orphan
already died. The decision therefore reduces to whether that single capability (dead-orphan
salvage) is worth ~4–8× the code, a landed-spec change, and a threat-model rewrite —
quantified above so the trade is explicit rather than hand-waved.
