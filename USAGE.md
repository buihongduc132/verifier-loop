# USAGE — verifier-loop / verifier-verdict

Invocation reference for both binaries. See [`README.md`](README.md) for install + `config.json`,
and [`AGENTS.md`](AGENTS.md) for design source-of-truth pointers.

## `verifier-loop` (A — aliased `jewilo`)

A's interface. Dispatches on `NEW` / `RESUME` (tasks.md §10):

```
verifier-loop NEW   "<goalText>" [--context "<…>"]
verifier-loop RESUME <goalId>    [--fix "<…>"]
```

### `NEW "<goalText>"`

Creates an immutable, signed goal under the store root, captures the frozen artifact snapshot,
renders the verifier prompt, spawns round 1 (m verifiers), gathers, and evaluates n/m consensus.

- Required positional: `goalText` (the goal to verify).
- Optional: `--context "<…>"` — extra context folded into the prompt.
- Output (stdout): `goalId: <id>`; on n/m APPROVE consensus, the short completion hash
  (`mmddyy-XXXXXXXX`, e.g. `070326-a1b2c3d4`). The full 64-hex digest is stored in
  `completion.json` `fullDigest` (not printed).
- Exit `0` on consensus; non-zero on store/config/spawn failure or when a round does not reach
  `n/m` consensus (rejection summary printed to stderr).

### `RESUME <goalId>`

Loads the goal, increments the round, appends fix notes (from prior round rejections), re-captures
the snapshot, renders the resume prompt, and re-spawns verifiers.

- Required positional: `goalId`.
- Optional: `--fix "<…>"` — additional fix notes to feed the resume prompt.
- Output / exit codes: same as `NEW` (hash on consensus, non-zero on failure).

## `verifier-verdict` (V\* — aliased `jewije`)

Each verifier's interface (tasks.md §7 / verdict-registration spec). Identity is resolved **from
the `VERIFIER_LOOP_*` env (D2)** — env always wins; there is no argument override for identity.

```
verifier-verdict approve
verifier-verdict reject --notes "<non-empty reason>"
```

- `approve` — registers an APPROVE verdict in this verifier's slot.
- `reject --notes "<…>"` — registers a REJECT verdict; `--notes` is a required clap argument and
  must be non-empty (an empty-string value is refused with `NotesRequired`, exit non-zero).
- On success: prints `Verdict registered` to stdout, exit `0`.
- The **first** verdict written to a slot is final; a second attempt fails with `AlreadyFinal`
  (exit non-zero) and the stored verdict is left unchanged.

## Environment variables

| var                          | used by        | meaning                                                                                   |
|------------------------------|----------------|-------------------------------------------------------------------------------------------|
| `VERIFIER_LOOP_HOME`         | both           | store root (default `~/.verifier-loop`).                                                  |
| `VERIFIER_LOOP_GOAL_ID`      | verifier       | goal slot to write into (env wins; no arg override).                                      |
| `VERIFIER_LOOP_VERIFIER_ID`  | verifier       | verifier slot id (`v1`, `v2`, …).                                                         |
| `VERIFIER_LOOP_ROUND`        | verifier       | round number (u32).                                                                       |
| `VERIFIER_LOOP_VERIFIER_SECRET` | verifier    | hex-encoded Ed25519 signing key for this V\* slot (see [Signed-regime secret](#verifier_loop_verifier_secret--signed-regime-secret) below). |
| `VERIFIER_LOOP_BACKEND_CMD`  | loop           | stub/custom backend command override, used for both spawn and resume.                     |
| `VERIFIER_LOOP_SPAWN_CMD`    | loop           | spawn-only backend command override (takes precedence over `BACKEND_CMD` for spawn).      |
| `VERIFIER_LOOP_RESUME_CMD`   | loop           | resume-only backend command override (defaults to the spawn command when unset).          |

## `VERIFIER_LOOP_VERIFIER_SECRET` — signed-regime secret

`VERIFIER_LOOP_VERIFIER_SECRET` is the **hex-encoded Ed25519 signing key** injected by the
spawn layer (`jewilo`) into each V\* process at spawn time. It is the per-verifier credential
introduced by the `add-verifier-tamper-hardening` change (spec: `verifier-identity`,
`signed-verdict-record`, `verdict-registration`).

- **Injected automatically by spawn.** `jewilo` mints a fresh Ed25519 keypair per V\* slot,
  writes only the **public** half to `<slot>/verifier-pubkey.json` (`{pubkey, mintedAt}`),
  and injects the **signing** half into the V\* process env as `VERIFIER_LOOP_VERIFIER_SECRET`.
  The signing key is **never** persisted to disk by `jewilo` or `jewije`.
- **Required for the signed registration path.** `jewije approve` / `jewije reject` derive the
  slot's signing key from `VERIFIER_LOOP_VERIFIER_SECRET` and sign the verdict record over the
  canonical bytes `{status, notes, registeredAt, goalId, verifierId, round}`. The written
  `verdict.json` then carries `signature` (128-hex Ed25519) and `pubkeyId` (first 16 hex of the
  pinned pubkey).
- **Fail-closed when absent or mismatched.** If `VERIFIER_LOOP_VERIFIER_SECRET` is unset/empty,
  or its derived pubkey does not match the slot's pinned `verifier-pubkey.json`, `jewije` exits
  non-zero with `VerdictError::Unauthenticated` and writes **no** verdict and **no** receipt-log
  entry. The slot is left untouched.
- **BREAKING for direct/manual `jewije` invocations.** Pre-`add-verifier-tamper-hardening`
  workflows that called `jewije` directly (e.g. from a hand-rolled spawn script or an ad-hoc
  shell) now fail. To register a verdict manually you must obtain the slot's signing secret
  (the same value `jewilo` would have injected) and export it:
  ```sh
  VERIFIER_LOOP_HOME=/path/to/store \
  VERIFIER_LOOP_GOAL_ID=<id> \
  VERIFIER_LOOP_VERIFIER_ID=v1 \
  VERIFIER_LOOP_ROUND=1 \
  VERIFIER_LOOP_VERIFIER_SECRET=<64-hex-ed25519-signing-key> \
    jewije approve
  ```
  The recommended path is to drive V\* via `jewilo` so the secret is minted and injected
  automatically; manual invocation is only for debugging/recovery.
- **Pinned pubkey derivation.** `verifier-pubkey.json` (per slot) is the public half of this
  secret, written at spawn. Consensus evaluation verifies each APPROVE verdict's signature
  against this pinned pubkey before counting it toward n/m; a verdict signed by any other key
  (including a fresh keypair a forger mints after the fact) is treated as untrusted and named
  in the rejection summary.

For the threat model (what this secret does and does **not** prevent on a single host), see
[`THREAT-MODEL.md`](THREAT-MODEL.md).

## Verifier prompt override (`verifierPromptFile`)

`~/.verifier-loop/config.json` may set `verifierPromptFile` to a path whose **raw** contents are
prepended to the baked-in verifier prompt for every NEW + RESUME round. Relative paths resolve
against `VERIFIER_LOOP_HOME`; absolute paths used as-is. No `{{var}}` expansion — it is a static
preamble. Rendered order:

```text
<custom file contents>
---
<baked-in default template (policy + goalText + frozen snapshot)>
```

If the file is missing/unreadable → fail-closed error (no goal dir written). Absent/null →
baked-in default only (today's behavior).

Worked example:

```bash
mkdir -p ~/.verifier-loop/prompts
cat > ~/.verifier-loop/prompts/verifier_rules.md <<'EOF'
## Rules
- you are only to verify; do not do engineering work.
- 1 wrong thing => REJECT.
EOF
# wire it + enforce a minimum goal length:
cat > ~/.verifier-loop/config.json <<'EOF'
{
  "n": 2, "m": 2, "backend": "pi",
  "verifierPromptFile": "prompts/verifier_rules.md",
  "minGoalChars": 500
}
EOF
```

## Goal length validation (`minGoalChars`)

`config.json` may set `minGoalChars` (u64, default `0` = disabled). The **trimmed** `goalText`
char count must be `>= minGoalChars`. Empty/whitespace-only `goalText` is ALWAYS an error
regardless of `minGoalChars`. Below-threshold → fail-closed error before any goal dir / signature
is written.

## Closed schema + runtime `cwd`

The `config.json` schema is **closed**: any key outside the canonical set
(`n`, `m`, `maxTurn`, `backend`, `gitDiffMaxChars`, `verifierTimeoutSec`, `verifierPromptFile`,
`minGoalChars`) is a hard parse error. Notably there is **no `cwd` key** — the frozen snapshot's
`cwd` is always `std::env::current_dir()` at invocation time. To verify work in a worktree:

```bash
cd /path/to/worktree && jewilo NEW "<goal>"
```

Legacy keys from older releases (`cwd`, `model`, `verifierPromptTemplate`,
`verifierResumePromptTemplate`) must be removed; jewilo exits non-zero with an error naming the
offending field if any are present.

## Stub / custom backend

Built-in adapters are `pi`, `hermes`, `acpx` (resolved from `config.backend`). Any **other**
`config.backend` value (e.g. `"stub"` for hermetic tests, or `"custom"`) is resolved from the
backend-command env vars — this lets deterministic end-to-end runs proceed without a real `pi`.

Resolution order (tasks.md §4.4 / §10):

1. `VERIFIER_LOOP_SPAWN_CMD` / `VERIFIER_LOOP_RESUME_CMD` (per-phase, if set).
2. else `VERIFIER_LOOP_BACKEND_CMD` (applies to both spawn and resume).
3. else: **fail closed** — an unknown backend with no command override is an error, never a silent
   fallback to `pi` (which would produce an unparseable stream and a null verdict indistinguishable
   from a real crash).

Example — drive a deterministic stub for tests:

```sh
export VERIFIER_LOOP_HOME=/tmp/vl-store
cat > "$VERIFIER_LOOP_HOME/config.json" <<'JSON'
{ "n": 1, "m": 1, "backend": "stub" }
JSON

VERIFIER_LOOP_BACKEND_CMD='printf "%s" "{\"type\":\"assistant\",\"message\":{\"content\":\"APPROVE\"}}"' \
  verifier-loop NEW "hermetic goal"
```
