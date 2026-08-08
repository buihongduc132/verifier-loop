## Context

`jewilo NEW` fails 100% of the time with `os error 7` (`E2BIG`) before any verifier runs (gh #1, #4). Forensics:

- `src/spawn/orchestrator.rs:395-409` `build_spawn_command` does `template.split_once("{prompt}")` then `cmd.arg(prompt)` — the entire rendered prompt becomes ONE argv element.
- Rendered prompt = `verifierPromptFile` preamble (reported ~49KB) + baked-in `verifier_policy.txt` (31KB) + `default_template.txt` (3KB) + gitDiff (up to `gitDiffMaxChars`, default 6000). Easily exceeds Linux `MAX_ARG_STRLEN` (128KB single arg) and `ARG_MAX` (typically 2MB total argv+env).
- A second latent bug: template `pi -p "{prompt}" --mode json` splits to left=`pi -p "`, right=`" --mode json`; `split_whitespace()` keeps the literal `"` as a token. Even when size fits, argv = `[pi, -p, ", <prompt>, ", --mode, json]` — `pi`'s `-p` consumes `"` as its value, real prompt demoted to a positional arg.
- Doc comment at orchestrator.rs:389-394 claims it delegates to `sh -c` for quoting; code does no such thing. Comment/code drift.
- Child stdin is `Stdio::null()` (orchestrator.rs:139, 214) — currently forbids any stdin-based delivery.

Constraints:
- Tamper-evidence invariants (NULL verdict never → APPROVE; missing store → no hash; goalText edit → hash mismatch) MUST be preserved. Transport change is orthogonal to verdict/hash logic but must not corrupt the prompt bytes that verifiers see.
- TDD discipline (RED by fresh author, GREEN by different fresh author, ≥80% line coverage per new src file) is mandatory.
- No new heavy deps; prefer std + already-available crates.

## Goals / Non-Goals

**Goals:**
- Eliminate `E2BIG` / `Argument list too long` for any realistic prompt size.
- Provide two transports: `stdin` (default, no tempfiles) and `goal-file` (for backends that only accept file paths).
- Fail-closed at config load for legacy `{prompt}`-inline templates.
- Preserve all tamper-evidence invariants; prompt bytes verifiably reach the verifier.
- Fix the stray-`"` argv corruption as a consequence of removing `{prompt}` inlining.

**Non-Goals:**
- Changing prompt content/rendering (`src/prompt/`), gitDiff truncation, or the `{{var}}` template engine.
- maxTurn/refresh logic, consensus, hash/proof — untouched.
- A user-facing `--goal-file` CLI flag on `jewilo` itself. `transport` is adapter-config, declared in `config.json`; `goal-file` paths are orchestrator-internal tempfiles, not user-supplied.
- OT1–OT6 deferred items.
- Per-verifier `--goal-file` flag templating beyond `{goalFile}` substitution (no `{promptFile}`, no env-var path).

## Decisions

### D1 — Two transports, stdin default
**Choice:** Every adapter declares `transport: "stdin" | "goal-file"`. Built-in `pi`/`hermes`/`acpx` default to `stdin`.

**Why over alternatives:**
- *Alternative A: tempfile-only.* Adds disk I/O, cleanup lifecycle, tempfile-leak risk on crash. Worse default.
- *Alternative B: env var (`VERIFIER_LOOP_PROMPT`).* Env is also bounded by `ARG_MAX` (argv+env share the limit); a 1MiB prompt can still blow it. Rejected.
- *Alternative C: keep `{prompt}` but auto-detect size and switch.* Detect-and-switch adds branches and surprises; explicit transport is simpler and testable.

stdin is the canonical Unix pipe-for-large-data path; goal-file covers backends that demand a path (some CLIs only accept `-f <file>`). Covering both is cheap.

### D2 — Adapter schema gains `transport`; `{prompt}` rejected at load
**Choice:** `Adapter { spawn, resume, transport }`. `config.json` custom adapters MUST set `transport` and MUST NOT use `{prompt}`. Built-in defaults: `transport=stdin`, `spawn="pi --mode json"`, `resume="pi --session {sid} --mode json"`.

**Why:** Fail-closed. A silent legacy `{prompt}` template would re-introduce the bug. Load-time rejection with a migration message is the cheapest defense.

**Alternatives considered:** Deprecation warning + auto-migration. Rejected — auto-rewriting user config is fragile; better to fail loud and let the user fix one line.

### D3 — `goal-file` uses tempfile, unlinks post-spawn
**Choice:** Write prompt to `std::env::temp_dir()/verifier-loop-<goal_id>-<verifier_id>-<round>-<rand>.txt`, substitute path into `{goalFile}`, spawn, then `unlink` immediately. On spawn error, unlink before propagating.

**Why post-spawn unlink (not post-child-exit):** The child opens the file (or memory-maps it) before `exec` returns; once spawned, the file descriptor is the source of truth, not the directory entry. Unlinking post-spawn guarantees no tempfile persists even if the orchestrator crashes later, while the child still reads via its fd. This is the standard `O_CREAT|OUnlink` pattern (cf. `mkstemp(3)` + `unlink(2)`).

**Alternatives:** Keep file until child exit → leak risk on orchestrator crash. Use `memfd_create` → not portable, no Rust std equivalent. Rejected.

### D4 — stdin write is a background task; EPIPE handling
**Choice:** After `cmd.spawn()` with `Stdio::piped()`, spawn a `tokio::spawn` task that writes the full prompt to the child's stdin then closes it. If the write returns `EPIPE`, surface it to the gather logic; gather treats `EPIPE` as fatal only if no ACP event was parsed (fail-closed — null verdict).

**Why:** Async write avoids blocking the orchestrator on a slow-reading child. `EPIPE` after a verdict is benign (child finished early); before any ACP output it indicates the verifier never started → fail-closed null.

**Alternatives:** Synchronous blocking write → simpler but stalls parallel spawns against a slow reader. Rejected (parallelism is a spec requirement).

### D5 — `{goalFile}` placeholder, single substitution
**Choice:** `goal-file` templates use exactly one placeholder: `{goalFile}`. Replaces all occurrences (rare; usually one). No `{promptFile}`, no per-segment split.

**Why:** `split_once` on `{prompt}` was the source of the stray-`"` bug. A single path token has no whitespace/quoting issues — paths are well-formed and shell-safe in `cmd.arg()`.

### D6 — Built-in template migration
**Choice:**
| Adapter | transport | spawn | resume |
|---------|-----------|-------|--------|
| pi      | stdin     | `pi --mode json` | `pi --session {sid} --mode json` |
| hermes  | stdin     | `hermes --mode json` | `hermes --session {sid} --mode json` |
| acpx    | stdin     | `acpx --mode json` | `acpx --session {sid} --mode json` |

**Note (CA1):** Whether `pi`/`hermes`/`acpx` actually read the prompt from stdin when no `-p` is passed is UNVERIFIED. This is the highest-risk assumption — see Risks R3. Tasks MUST include a smoke test that confirms the backend reads stdin BEFORE marking the group done.

### D7 — Stdio config per transport
**Choice:** `transport=stdin` → child `stdin = Stdio::piped()`, stdout/stderr unchanged. `transport=goal-file` → child `stdin = Stdio::null()` (current behavior), stdout/stderr unchanged. No change to how stdout/stderr are captured (ACP stream parsing).

## Risks / Trade-offs

- **[R1] Tempfile leak on orchestrator panic between write and unlink** → Mitigation: unlink in a `Drop`-style guard (`TempPromptFile` RAII) so panic/early-return still unlinks. Add a startup sweep that removes stale `verifier-loop-*` tempfiles in `temp_dir()` (best-effort, non-blocking).
- **[R2] `EPIPE` ambiguity** → Mitigation: tie `EPIPE` fatality to ACP-output presence (D4). Tests cover both branches. Fail-closed default: if uncertain, treat as fatal → null verdict.
- **[R3] Backends may not read prompt from stdin** [CA1] → Mitigation: Task §3 (smoke) MUST verify `pi --mode json` reads stdin and produces a session event BEFORE shipping the default. If `pi` does not support stdin prompts, fall back to `transport=goal-file` as the pi default and document. This is the single biggest unknown.
- **[R4] Custom-adapter users break at config load** → Mitigation: clear migration message naming the field and accepted replacements. This is an intentional, loud BREAKING change — better than silent re-introduction of E2BIG.
- **[R5] Stray `"` was masking a separate quoting concern** → Removing `{prompt}` inlining eliminates the stray `"` for free; no separate fix needed. Document in code comment that `{prompt}` is forbidden.
- **[R6] Comment/code drift on `build_spawn_command`** → Mitigation: rewrite the doc comment to match new behavior; the existing comment claiming `sh -c` delegation was already wrong.

## Migration Plan

1. Ship the change. Built-in defaults switch to `transport=stdin`, `spawn="pi --mode json"`.
2. Users with custom `config.json` adapters using `{prompt}` get a loud load-time error with the exact fix.
3. Rollback: revert to prior `build_spawn_command` + `{prompt}` templates. No data migration needed (no on-disk format change; goals/verdicts/hashes untouched).
4. Smoke per release: a >128KB prompt spawns successfully (R3 verification).

## Open Questions

- **OQ1:** Does `pi --mode json` actually read the prompt from stdin when `-p` is omitted? Resolved by Task §3 smoke. If NO, pi's default transport becomes `goal-file` (template `pi --goal-file {goalFile} --mode json`) — needs `pi` to support a `--goal-file` flag, also UNVERIFIED. Fallback if neither: investigate `pi -p -` (read prompt from stdin via `-` sentinel).
- **OQ2:** Should `transport` be per-adapter or global in `config.json`? Decision: per-adapter (inside the adapter object), so a mixed fleet is possible. Revisit if config UX gets noisy.
