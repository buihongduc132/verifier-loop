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
| `VERIFIER_LOOP_BACKEND_CMD`  | loop           | stub/custom backend command override, used for both spawn and resume.                     |
| `VERIFIER_LOOP_SPAWN_CMD`    | loop           | spawn-only backend command override (takes precedence over `BACKEND_CMD` for spawn).      |
| `VERIFIER_LOOP_RESUME_CMD`   | loop           | resume-only backend command override (defaults to the spawn command when unset).          |

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
