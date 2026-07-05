# Finding — Out-of-band prompt transport for `pi` (resolves OQ1, R3)

**Date:** 2026-07-05
**Branch:** `fix/spawn-argv-overflow`
**Scout task:** §3 of `openspec/changes/fix-spawn-argv-overflow/tasks.md`
**Resolves:** Open Question OQ1 in `design.md`; informs decision D6.

## TL;DR

- **`pi` reads the prompt from stdin when no `-p` flag is passed.** This is the
  clean, signature-preserving, ARG_MAX-safe transport. **Recommended DEFAULT.**
- Working command template: **`pi --mode json`** with the rendered prompt written
  to the child's stdin, then stdin closed.
- `hermes` and `acpx` do **not** expose an equivalent non-interactive JSON-line
  stdin transport on this host. Their built-in adapters should keep argv (`-z`)
  until a follow-up probe on a host where they are properly configured, OR be
  marked transport=`GoalFile` only if a file flag is later confirmed.

---

## Probe matrix

| Backend | `printf ... \| <bin> --mode json` (stdin) | `--goal-file` / `--prompt-file` flag? | `@file` inclusion | `-p -` sentinel | Verdict |
|---------|-------------------------------------------|----------------------------------------|-------------------|-----------------|---------|
| `pi`    | ✅ **WORKS** — emits `{"type":"session"...}` then echoes the exact user message | ❌ no such flag (`pi --help` has none) | ⚠️ works but **mutates** text (wraps in `<file name="...">...</file>`) | not tested (stdin already proven) | **stdin** |
| `hermes`| ❌ bare stdin enters interactive TUI; on EOF prints `Goodbye!` with **no model run, no JSON** | ❌ no `--mode json`; uses `-z PROMPT` (argv) | n/a | n/a | **none (argv only)** |
| `acpx`  | n/a — **not on PATH** (`command -v acpx` → not found) | n/a | n/a | n/a | **unprobed** |

---

## Evidence

### Probe 1 — `pi` reads stdin (DEFAULT path) ✅

Command:
```bash
printf 'Reply with exactly the word OK and nothing else.' | timeout 90 pi --mode json
```

First event lines (proving the backend processed the prompt):
```json
{"type":"session","version":3,"id":"019f2e67-31bf-782e-a716-a68072e47f29","timestamp":"2026-07-04T18:32:22.719Z","cwd":"/tmp"}
{"type":"agent_start"}
{"type":"turn_start"}
{"type":"message_start","message":{"role":"user","content":[{"type":"text","text":"Reply with exactly the word OK and nothing else."}],"timestamp":1783189947746}}
```

Assistant reply event (proving the prompt was actually executed, not just echoed):
```json
{"type":"message_end","message":{"role":"assistant","content":[{"type":"text","text":"OK"}],...,"stopReason":"stop",...}}
```

Exit code: `0`. The user-message `text` field is **byte-identical** to the stdin
bytes — no wrapping, no mutation. This is critical for the goalText signature /
completion-hash invariant.

### Probe 3-variant — `pi @file` works but MUTATES the prompt ⚠️ (rejected)

Command:
```bash
printf 'Reply with exactly the word FOO.' > /tmp/test-prompt.txt
pi --mode json @/tmp/test-prompt.txt
```

Observed user message:
```json
{"type":"message_start","message":{"role":"user","content":[{"type":"text","text":"<file name=\"/tmp/test-prompt.txt\">\nReply with exactly the word FOO.\n</file>\n"}]}}}
```

The literal text delivered to the model is **not** the file contents — it is
wrapped in `<file name="...">...</file>`. If the orchestrator hashed the raw
`goalText` but delivered `@file`, the goalText-edit → signature-mismatch →
hash-mismatch fail-closed invariant would fire on **every** run. Therefore
`@file` is **not** a valid goal-file transport for verifier-loop.

### Probe 2 — `pi -p -` (stdin sentinel)

Not run. Probe 1 already proved bare-`pi` reads stdin, which is strictly
simpler than `-p -` and avoids consuming a flag. No need to pursue.

### Probe 4 — `pi --help` flag scan

`pi --help` (full output captured) contains **no** `--goal-file`,
`--prompt-file`, `--from-stdin`, or `-p -` sentinel flag. Usage line is:

```
pi [options] [@files...] [messages...]
```

The only file-based path is `@file` (rejected above). The only argv prompt path
is positional `messages...` or `-p` (current broken template). **Stdin is the
sole clean out-of-band transport.**

### Probe 5 — `hermes`

- `hermes --help` shows a Python CLI with `-z PROMPT` (argv) and an `acp`
  subcommand for editor integration. There is **no** `--mode json` and **no**
  `--goal-file` / `--prompt-file`.
- `printf '...' | hermes` (bare) → enters interactive TUI, prints
  `Welcome to Hermes Agent!`, reads the stdin line into the input box, then on
  EOF prints `Goodbye! ⚕` **without invoking the model and without emitting any
  JSON**. So bare-stdin is NOT a non-interactive transport.
- `hermes -z "Reply with exactly OK."` → exits 0 but produces **no stdout**
  on this host (likely needs model/provider setup; out of scope for this
  scout). Hermes' only confirmed non-interactive path is `-z PROMPT`, which is
  argv-bound and therefore still subject to ARG_MAX.

Conclusion: **hermes has no stdin/goal-file transport available here.** Its
built-in adapter should remain on the argv template (`hermes -z {prompt}`)
until a dedicated probe confirms otherwise, and should be flagged as
transport-incompatible with >128KB prompts in the interim.

### Probe 5 — `acpx`

`command -v acpx` → not found. Not probed.

---

## Recommendation (informs design.md D6)

1. **DEFAULT transport for the built-in `pi` adapter: `Stdin`.**
   - Spawn template: **`pi --mode json`** (no `{prompt}` token, no `-p`).
   - Resume template: **`pi --session {sid} --mode json`** (resume also reads
     stdin; verify in §4 RED test by spawning a real `pi --session ...` with
     stdin piped — out of scope for this scout).
   - Orchestrator: set `stdin = Stdio::piped()`, write the full rendered prompt
     to stdin in a `tokio::spawn` background task, then close. Argv contains
     **zero** prompt-derived bytes → no `E2BIG` regardless of prompt size.

2. **`hermes` adapter:** keep argv (`hermes -z {prompt}`) for now. Mark
   transport=`GoalFile`-incompatible until a follow-up probe on a host where
   `hermes -z` actually returns model output. Do **not** block the §3 scout
   resolution on hermes.

3. **`acpx` adapter:** unprobed (binary absent). Leave template as-is; revisit
   when `acpx` is on PATH.

4. **`pi @file` is REJECTED** as a goal-file transport because it mutates the
   prompt text and would break the goalText signature invariant. If a
   `GoalFile` transport is ever needed for `pi` (e.g. a backend that closes
   stdin early), a different mechanism must be found — but stdin is sufficient
   for `pi` today, so `GoalFile` is only relevant for hypothetical custom
   adapters.

5. **OQ1 closure:** *"How does `pi` accept a prompt out-of-band?"* → **stdin,
   via `pi --mode json` with the prompt piped to fd 0 and no `-p` flag.**
   Confirmed by the `{"type":"session"...}` + echoed user-message evidence
   above.

## Next steps (hand-off to §4–§8)

- §4 RED tests: assert `transport=Stdin` → `stdin=Stdio::piped()`, argv has no
  prompt bytes, 1 MiB prompt spawns without `E2BIG`, and `EPIPE` semantics
  (fatal before first ACP event, non-fatal after a verdict).
- §8 built-in migration: change `pi` spawn template to `pi --mode json`
  (transport `Stdin`); leave `hermes`/`acpx` templates untouched with a TODO.
- §3.3: update `design.md` D6 with the stdin decision and close OQ1 (separate
  commit; this scout does not edit `design.md`).
