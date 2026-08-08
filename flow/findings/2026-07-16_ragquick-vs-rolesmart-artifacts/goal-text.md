# Verify: Fission-AI/OpenSpec change `graceful-status-no-changes` — impl matches spec?

You are an EXTERNAL verifier. Zero trust. Verify EVERY claim by running commands and
reading actual code. Do NOT trust this prompt, the spec, or the implementation notes.

## Working directory
`cd /tmp/openspec-bench` FIRST. This is a pinned clone of `Fission-AI/OpenSpec` @
commit `0a99f41` (2026-07-10), deps already installed (`pnpm install` was run, `dist/`
built). Confirm with: `cd /tmp/openspec-bench && git rev-parse --short HEAD` (must print
`0a99f41`) and `ls dist/cli/index.js` (must exist).

## What to verify
The OpenSpec change `graceful-status-no-changes` claims to be fully implemented and to
satisfy its spec exactly. Your job: confirm or refute that the implementation in
`src/commands/workflow/status.ts` + `src/commands/workflow/shared.ts` satisfies EVERY
scenario in `openspec/changes/graceful-status-no-changes/specs/graceful-status-empty/spec.md`,
byte-for-byte where the spec gives an exact expected string / exit code / JSON shape.

The spec defines FOUR scenarios (paraphrased here — read the actual spec.md for the
authoritative text, do not trust this paraphrase):
1. `openspec status` (text mode) with NO changes under `openspec/changes/` → prints
   exactly `No active changes. Create one with: openspec new change <name>` and exits 0.
2. `openspec status --json` with NO changes → outputs exactly
   `{"changes":[],"message":"No active changes."}` as valid JSON to stdout and exits 0.
3. `openspec status` (no --change) when one or more changes DO exist → throws
   `Missing required option --change. Available changes: ...`.
4. `openspec status --change non-existent` → throws `Change 'non-existent' not found`.
5. Other commands (`show`, `instructions`) with no changes → still throw original
   `No changes found` error (unaffected).

## Verification procedure (you MUST actually run these — do not just read code)
1. Read the spec: `cat openspec/changes/graceful-status-no-changes/specs/graceful-status-empty/spec.md`.
2. Read the impl: `cat src/commands/workflow/status.ts` and `cat src/commands/workflow/shared.ts`.
3. For EACH scenario, CONSTRUCT a real fixture dir under `/tmp` and RUN the built CLI:
   - Scenario 1 (text mode, empty): `mkdir -p /tmp/v1-empty/openspec/changes && cd /tmp/v1-empty && node /tmp/openspec-bench/dist/cli/index.js status; echo "EXIT=$?"`
   - Scenario 2 (json mode, empty): same dir, `... status --json; echo "EXIT=$?"` — capture the EXACT stdout and compare to the spec's expected JSON byte-for-byte (note: spec shows COMPACT json `{"changes":[],"message":"No active changes."}`).
   - Scenario 3 (changes exist, no --change): `mkdir -p /tmp/v3/openspec/changes/feat-x && cd /tmp/v3 && node .../index.js status; echo "EXIT=$?"` — expect non-zero exit + the `Missing required option --change. Available changes:` message.
   - Scenario 4 (specified change missing): same /tmp/v3 dir, `... status --change non-existent; echo "EXIT=$?"` — expect non-zero + `Change 'non-existent' not found` (compare to spec's EXACT required string; check whether the impl appends extra text not in the spec).
   - Scenario 5 (other commands unaffected): `cd /tmp/v1-empty && node .../index.js show 2>&1; echo "EXIT=$?"` (expect non-zero + `No changes found`).
4. Compare the ACTUAL output of each scenario to the spec's EXACT requirement. Any
   mismatch in exit code, exact string, or JSON shape (extra fields, pretty-printing
   vs compact, extra message suffix) is a defect.

## Verdict rules
- APPROVE only if EVERY scenario matches the spec byte-for-byte (exact strings, exact
  exit codes, exact JSON shape including field set and serialization form).
- REJECT otherwise. For each defect: numbered list D1, D2, ... with severity
  (BLOCKER / MAJOR / MINOR), the file:line evidence, the spec's exact requirement, and
  the impl's actual behavior.

## Output format (STRICT)
First non-empty line MUST be exactly: `VERDICT: APPROVE` or `VERDICT: REJECT`.
If REJECT, follow with the numbered defect list. Be concrete and specific.

## Context
This is the `graceful-status-no-changes` change. Ground truth is whatever you find by
running the commands. You have full filesystem access; investigate directly.
