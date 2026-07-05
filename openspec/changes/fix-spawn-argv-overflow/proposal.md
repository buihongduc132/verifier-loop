## Why

`jewilo NEW` always fails with `spawn failed: io error: Argument list too long (os error 7)` before any verifier runs (gh issues #1, #4). The full rendered verifier prompt (verifierPromptFile preamble ~49KB + baked-in policy 31KB + template 3KB + gitDiff up to gitDiffMaxChars) is passed as a single argv element via `cmd.arg(prompt)`, exceeding Linux `MAX_ARG_STRLEN` / `ARG_MAX`. A second latent bug — stray literal `"` tokens from `{prompt}`-template splitting — corrupts argv even when size fits. The mandated primary verifier-loop path is unusable until fixed.

## What Changes

- **BREAKING**: Spawn no longer inlines the prompt into argv. The prompt is delivered out-of-band via **stdin pipe** by default (preferred) OR via a **tempfile** referenced by a new `--goal-file <path>` flag, per backend adapter.
- Built-in adapter templates change from `pi -p "{prompt}" --mode json` to `pi --goal-file {goalFile} --mode json` (stdin variant: `pi --mode json` reads prompt from stdin).
- Adapter templates gain a `{promptFile}` / `{goalFile}` placeholder and a `transport` field (`stdin` | `goal-file`) declaring how the orchestrator delivers the prompt.
- Custom adapters in `config.json` MUST declare `transport`; legacy `{prompt}`-inline templates are rejected at config load (fail-closed) with a clear migration message.
- `Stdio::null()` for child stdin (orchestrator.rs:139, 214) becomes `Stdio::piped()` when `transport=stdin`; tempfile is written + path substituted + unlinked after spawn when `transport=goal-file`.
- Stray `"` token bug (template `pi -p "{prompt}"` → argv `[pi, -p, ", <prompt>, ", ...]`) eliminated by removing `{prompt}` inlining entirely.

## Capabilities

### New Capabilities
- `prompt-transport`: Out-of-band prompt delivery contract — stdin-pipe and goal-file transports, adapter declaration, orchestrator wiring, tempfile lifecycle, and fail-closed rejection of inline-`{prompt}` templates.

### Modified Capabilities
- `verifier-spawn`: Spawn no longer takes prompt via argv; adapters declare transport; spawn/resume templates use `{goalFile}` / stdin instead of `{prompt}`. Identity env vars unchanged.

## Impact

- **Code**: `src/spawn/orchestrator.rs` (`build_spawn_command`, `Stdio` config, tempfile write/unlink, stdin write task), `src/acp/adapters.rs` (template format + `transport` field + validation), `src/cli/mod.rs` (no new CLI flag — transport is adapter-config, not user-facing), `src/prompt/mod.rs` (unchanged — still renders the full prompt string).
- **APIs/Config**: `config.json` adapter schema gains `transport` field. Existing custom adapters using `{prompt}` break loudly with migration guidance. Built-in defaults updated.
- **Dependencies**: No new crates (tempfile via `std::fs`/`tempfile` if available, else `std::env::temp_dir`).
- **Tests**: New unit tests for both transports; existing spawn tests updated to assert argv no longer contains the prompt; e2e smoke that a >128KB prompt spawns successfully.
- **Specs**: New `prompt-transport` spec; delta on `verifier-spawn`.
- **Out of scope**: Prompt content/rendering changes, maxTurn/refresh logic, consensus/proof hashing, OT1–OT6 deferred items.
