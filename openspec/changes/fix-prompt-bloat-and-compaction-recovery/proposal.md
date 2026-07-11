## Why

jewilo produces null verdicts on the majority of real verification runs, blocking the mandatory verifier-loop gate for every agent task. Three independent root causes were confirmed across 21 open gh issues (verified by 4 dedicated sub-agents against source + live goal dirs + session JSONLs):

1. **Prompt bloat → compaction** (Group C, #10/#12/#13/#21/#22/#31): `capture_file_edit_times` dumps every tracked file's mtime with no cap — 40-83% of every rendered prompt (up to 686KB / 242K input tokens). The pi backend hits `type:compaction` at turn 1 and the verifier never emits a verdict.
2. **No verdict enforcement** (Group B, #11/#14/#17/#20/#23/#24/#25/#26/#27/#32): spawned verifier sessions exit after 1 assistant turn without ever calling `verifier-verdict`. The prompt only "tells" the model to register a verdict in prose; nothing detects a missing verdict, re-prompts, or re-spawns.
3. **No compaction self-recovery**: when compaction fires mid-verification, the session terminates with no verdict and jewilo extracts null. There is no path to resume the post-compaction session and harvest the verdict.

The result: correct, fully-verified work can never reach 2/2 consensus, no completion hash is ever minted, and every agent falls back to the weaker manual orchestrator. This must be fixed now — the automation is effectively non-functional.

## What Changes

### Prompt bloat (Group C)
- **Cap `fileEditTimes`**: limit to changed files only (`git status --porcelain`) OR hard byte cap. Removes 40-83% of prompt bytes.
- **Dedupe policy**: when a custom `verifierPromptFile` is configured, skip the embedded `VERIFIER_POLICY` in `DEFAULT_TEMPLATE` (currently duplicated 2× = 62KB wasted).
- **Cap `--context` length** (currently unbounded).
- **Warn** when rendered prompt exceeds a budget threshold (~50KB) so operators get early warning.

### Verdict enforcement (Group B)
- **Detect no-verdict exit**: after `gather()` reaps a child, if no verdict.json was written (or `status:null`), treat it as a recoverable failure, not a silent null.
- **Within-round resilience**: re-prompt the same session with a minimal "register your verdict now via verifier-verdict" nudge, up to `maxTurn` retries, before declaring the slot null.
- **Strengthen prompt template**: the final step must be an explicit bash command invoking `verifier-verdict`, not prose instruction.

### Compaction self-recovery (bridges B + C)
- **Compaction is a first-class event**: detect `type:compaction` in the session stream. After compaction, auto-resume the same session (sid reuse) with a minimal verdict-nudge prompt to harvest the verdict rather than extracting null.
- **Invariant**: a verifier that has completed its investigation MUST always reach a verdict, even if compaction fired mid-analysis. "Must always be able to compact itself."

## Capabilities

### New Capabilities

- `compaction-recovery`: detects compaction events in spawned verifier sessions and auto-resumes the post-compaction session to harvest the verdict, so compaction never silently produces a null verdict.

### Modified Capabilities

- `verifier-prompt`: rendered prompt must stay under a budget; `fileEditTimes` capped/filtered, policy deduplicated, `--context` bounded, oversize warning emitted.
- `verifier-spawn`: gather layer must enforce a verdict — detect no-verdict exits, re-prompt within `maxTurn`, and never silently extract null from a session that exited without calling `verifier-verdict`.

## Impact

- **Code**: `src/prompt/mod.rs` (fileEditTimes cap, dedupe, context cap, warn), `src/spawn/orchestrator.rs` (gather verdict enforcement + compaction detection), `src/acp/parser.rs` (compaction event parsing + post-compaction resume), `src/prompt/default_template.txt` (explicit final verdict command).
- **APIs**: no CLI surface change; behavior changes are internal (more verdicts reach consensus).
- **Dependencies**: none new.
- **Specs**: `verifier-prompt`, `verifier-spawn` requirement deltas; new `compaction-recovery` spec.
- **Closed issues**: #10, #11, #12, #13, #14, #17, #20, #21, #22, #23, #24, #25, #26, #27, #31, #32. (#1, #4 already fixed by `fix-spawn-argv-overflow`; #8, #9, #16, #30 are correct fail-closed behavior — not bugs, optional feature gaps.)
