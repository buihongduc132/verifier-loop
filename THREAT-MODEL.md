# THREAT-MODEL — verifier-loop / jewilo / jewije

This document is the **honest** security model for the verifier-loop CLI on a single host.
It is the authoritative reference for the `verifier-identity` + `signed-verdict-record` +
`receipt-log` + `completion-proof` hardening introduced by the
`add-verifier-tamper-hardening` change. For the normative behavioural contract, read the
specs at [`openspec/changes/add-verifier-tamper-hardening/specs/`](openspec/changes/add-verifier-tamper-hardening/specs/)
and the rationale in
[`openspec/changes/add-verifier-tamper-hardening/design.md`](openspec/changes/add-verifier-tamper-hardening/design.md)
(decisions D0–D8, risks R1–R6, non-goals).

The point of this file is to **prevent** an operator from confusing the tamper-evident
completion hash with a forgery-**prevention** guarantee. It is not. Read on.

---

## Actors

| Symbol   | Role                                                              | Runs on        |
|----------|------------------------------------------------------------------|----------------|
| **A**    | The arbiter / orchestrator process. Drives `jewilo` (NEW/RESUME). | same host      |
| **V\***  | Each verifier sub-agent. Drives `jewije` (approve/reject).       | same host (today) |
| **Auditor** | Any process that re-reads the goal dir + salt and recomputes the hash. | any host |
| **Mallory** | An in-process adversary. On a same-box deployment this is any process with write access to `~/.verifier-loop/` (the store root, `VERIFIER_LOOP_HOME`). | same host |

The store root is a directory tree under `VERIFIER_LOOP_HOME` (default `~/.verifier-loop/`).
Anyone with read+write access to that tree is Mallory.

---

## (a) What the per-verifier Ed25519 secret buys you

At spawn time `jewilo` mints a **fresh Ed25519 keypair per V\*** slot, writes only the
**public** half to `<slot>/verifier-pubkey.json` (`{pubkey, mintedAt}`), and injects the
**signing** half into the V\* process environment as
`VERIFIER_LOOP_VERIFIER_SECRET` (hex). The signing key is ALSO persisted to
`<slot>/verifier-secret.hex` (mode 0600, first-write-wins) so the verdict-enforcement
nudge loop (D5) and the compaction-recovery resume (D6) — which spawn NEW verifier
processes — can re-inject the SAME secret that signed the pinned pubkey to harvest a
signed verdict on resume. On a single host this is equivalent exposure to the existing
forgeability concession in §(b): a process with read access to the slot dir can forge.
Out-of-process V\* on a separate host remains the only prevention guarantee
(spec: `verifier-identity`).

Every signed `verdict.json` now carries `signature` (128-hex Ed25519 over the canonical
record bytes `{status, notes, registeredAt, goalId, verifierId, round}`) and `pubkeyId`
(first 16 hex of the pinned pubkey). Consensus evaluation verifies each APPROVE verdict's
signature against the slot's **pinned** pubkey **before** counting it toward n/m. A null
placeholder (`{status: null}`) carries no signature and is never counted.

**Deterrent effect on same-box Mallory:** before this change, forging a verdict required
only "set the env var and write the file" — anyone with filesystem write access could
trivially impersonate any V\*. After this change, Mallory must additionally **possess the
slot's pinned signing secret** to produce a verdict that survives consensus signature
verification. (See the persistence mechanism described at the top of §(a) above.)

This raises the bar from "any write access" to "must possess the per-verifier spawn-time
secret." It is a **deterrent + detection layer**, not a prevention guarantee.

---

## (b) The fundamental same-box limitation (read this twice)

> **A process that has write access to `~/.verifier-loop/` AND can read a V\*'s
> environment (i.e. `VERIFIER_LOOP_VERIFIER_SECRET`) can still forge that V\*'s verdict.**

This is unavoidable on a single host:

- The signing secret is injected into V\*'s env at spawn. **A itself is the parent of
  V\*** in the spawn tree. A process with sufficient privilege (a fully-compromised A,
  root, or anything that can read another process's `/proc/<pid>/environ` on Linux) can
  read that secret out of V\*'s env, then write a perfectly-signed `verdict.json` into
  V\*'s slot using the stolen secret. The signature will verify. The receipt log will
  chain. The completion hash will recompute cleanly. Forensics will look clean.
- The pinned-pubkey file is on the same writable filesystem Mallory already controls, so
  even the pinning can be undermined if Mallory can pre-write `verifier-pubkey.json`
  before the legitimate spawn (the code treats the **mtime-earliest** entry as authoritative,
  which raises the bar further but is not a hardware root of trust).

This is the same class of "defendant is also the judge and the forensics lab" problem
called out in `design.md` (risk R1, non-goal: "Protecting against a fully-compromised A
process that can read its own child process environment"). **It is documented, not
solved.** No amount of in-process cryptography can solve it; the secret and the verifier
it is supposed to bind live on the same machine controlled by the adversary.

---

## (c) The only true fix: out-of-process V\* on a separate host

True forgery **prevention** requires that the signing secret be **out of Mallory's
reach**. Concretely:

- Run each V\* on a **separate host** from A, where:
  - A cannot read V\*'s process environment (cannot exfiltrate
    `VERIFIER_LOOP_VERIFIER_SECRET`).
  - A cannot write V\*'s slot directory on that remote host (cannot hand-craft a
    `verdict.json` or overwrite the pinned `verifier-pubkey.json`).
- The pinned-pubkey + signed-verdict + hash-chained receipt-log design is exactly **what
  makes out-of-process V\* verification work**: an auditor (on a third, also-separate
  host) re-reads the goal dir + salt + pinned pubkeys + receipt log and can
  cryptographically confirm that every counted APPROVE was signed by the slot's pinned
  key, with no same-box path for A to have produced the signature.

Deploying V\* out-of-process is an **infra/deploy change**, explicitly listed as a
**non-goal** of the `add-verifier-tamper-hardening` change. The code is structured so
that such a deployment is straightforward (V\* needs only `jewije` + its identity env +
its secret env + read/write access to its own slot), but the deploy itself is out of
scope here.

---

## (d) What IS guarded (detectably or fail-closed)

These invariants hold **even on a same-box deployment** (Mallory with file write access
but **without** the per-verifier secret):

| Attack                                                            | Outcome                                                                                                                | Spec                                  |
|-------------------------------------------------------------------|------------------------------------------------------------------------------------------------------------------------|---------------------------------------|
| **Identity spoofing** — Mallory calls `jewije approve` against another V\*'s slot, faking the env vars. | `jewije` requires `VERIFIER_LOOP_VERIFIER_SECRET` matching the pinned pubkey. Without it → `VerdictError::Unauthenticated`, no verdict written. | `verdict-registration`, `verifier-identity` |
| **Null-slot first-fill** — Mallory pre-fills a `{status:null}` placeholder slot with APPROVE before the legitimate V\* verdicts. | First-fill of a null slot requires a signature from the pinned key. A null placeholder never carries a signature and never counts toward n/m. → `VerdictError::Unauthenticated`. | `verdict-registration`, `signed-verdict-record` |
| **In-flight verdict edit** — Mallory edits `status` (REJECT → APPROVE) / `notes` / `registeredAt` of an already-registered signed verdict, before consensus eval. | The signature no longer verifies over the canonical record bytes. Consensus treats the verdict as untrusted (not matching) and the rejection summary names the slot + signature failure. **Fail-closed in-flight, not merely post-hoc-auditable.** | `signed-verdict-record` |
| **Pinned-pubkey overwrite** — Mallory overwrites `verifier-pubkey.json` with a fresh keypair and re-signs. | The code treats the **mtime-earliest** pinned pubkey as authoritative; a verdict signed by the replacement key fails verification at consensus. (Raises the bar; does not defeat a Mallory who can also rewrite mtimes.) | `verifier-identity` |
| **`goalText` edit post-completion** — Mallory edits `goal.json` `goalText` after `completion.json` was written. | `goalSignature = SHA256(salt + goalText + createdAt)` changes → recomputed completion hash ≠ stored `fullDigest`. **Auditable post-completion.** | `completion-proof` |
| **Receipt-log mid-edit** — Mallory edits / inserts / deletes a line in `receipt-log.jsonl`. | The hash chain breaks (`prevHash`/`entryHash` mismatch) and/or the stored completion hash's `receiptLogHead` no longer matches the recomputed chain head. **Detectable on recompute.** | `receipt-log` |
| **Trailing-line deletion** — Mallory deletes the last receipt line after `completion.json` was written. | Recomputed chain head ≠ head folded into stored `fullDigest`. **Detectable on recompute.** | `receipt-log` |
| **NULL → APPROVE** — a crashed/timed-out V\* (null verdict) is counted as APPROVE. | Never. A null placeholder carries no signature and is non-matching by definition (D9). | `signed-verdict-record`, `verdict-registration` |
| **Missing store / goal dir** — Mallory deletes the goal dir. | No hash is produced. | (existing) |

---

## (e) What is NOT guarded on a same-box deployment

These attacks **succeed** if Mallory has both (1) write access to `~/.verifier-loop/` and
(2) the ability to read a V\*'s env (`VERIFIER_LOOP_VERIFIER_SECRET`):

- **Fully-compromised A** — A's own (compromised) code reads V\*'s spawned env, obtains
  the per-verifier signing secret, and writes a `verdict.json` directly into V\*'s slot
  with the stolen secret. The signature will verify against the pinned pubkey. Forensics
  will look clean. This is the fundamental limit documented as risk R1 / non-goal.
- **Any process that can read another process's env on the same host** (root, ptrace,
  `/proc/<pid>/environ` on Linux, container breakout, etc.) — same attack, same outcome.
- **Pre-spawn pinned-pubkey seeding combined with env read** — a Mallory that can both
  pre-write `verifier-pubkey.json` (and rewrite its mtime) and read the spawned secret
  can fully impersonate a V\* slot.

The mitigation for **all** of the above is the same: **run V\* out-of-process on a
separate host** (section (c)). The code-side signing/pinning/receipt-log design is what
makes that out-of-process verification meaningful; it does not by itself make same-box
forgery cryptographically impossible.

---

## Operational guidance

1. **Treat same-box `jewilo` as coordination + audit, not prevention.** The completion
   hash proves "the goal dir is internally consistent and reached n/m signed APPROVEs";
   on a single host it does **not** prove "no same-box process forged a verdict."
2. **For high-trust goals, deploy V\* out-of-process.** Each V\* on a separate host, with
   its own slot directory writable only by that host. A is restricted to read access to
   the goal dir for gathering/consensus.
3. **Restrict write access to `~/.verifier-loop/`.** The fewer processes that can write
   the store root, the smaller the Mallory surface. (The signed-regime already removes
   the trivial "any writer can forge any V\*" attack.)
4. **Gate releases with `cargo audit`.** `ed25519-dalek`, `rand`, and the transitive
   surface are scanned for advisories on every release (see README coverage gate).
5. **Audit by recompute.** Any auditor with the salt can recompute the goal signature,
   re-verify every verdict signature against its pinned pubkey, re-walk the receipt log
   hash chain, and recompute the completion hash. Mismatch on any of those is a
   tamper signal.

---

## Observability artifacts are NOT evidence (add-otel-observability)

The `add-otel-observability` change adds two new per-goal artifacts — `trace-id` and
`trace.jsonl` — plus a `traceId` field on `receipt-log.jsonl` entries and
`completion.json`. **These are observability metadata, not tamper-evident evidence.**

- `traceId` is **excluded** from the completion-hash inputs and from the receipt-log
  `entryHash` canonical tuple (pinned by tests). Two entries identical except `traceId`
  produce identical hashes. The hash formula is unchanged by this change.
- `trace.jsonl` is a best-effort append-only debug log; an adversary with write access
  to the store root can edit or delete it without affecting any verdict, the receipt
  chain, or the completion hash.
- Tracing is fail-**open**: a tracing error is swallowed and never blocks a verdict or
  poisons consensus. This is the opposite polarity from the evidence layer (which is
  fail-**closed**).

The receipt log (`receipt-log.jsonl`) remains the sole tamper-evident evidence ledger.
Use `trace.jsonl` + `traceId` only to pivot from a completion hash or receipt entry to
the span trail for debugging.

---

## Authoritative references

- Specs: [`openspec/changes/add-verifier-tamper-hardening/specs/`](openspec/changes/add-verifier-tamper-hardening/specs/)
  — `verifier-identity`, `signed-verdict-record`, `receipt-log`, `completion-proof`,
  `verdict-registration`, `verifier-spawn`.
- Design + risks + non-goals: [`openspec/changes/add-verifier-tamper-hardening/design.md`](openspec/changes/add-verifier-tamper-hardening/design.md).
- Failure-mode invariants: `AGENTS.md` "Fail-closed invariants".
- Hash formula + audit recompute: `README.md` "Completion-hash formula".
