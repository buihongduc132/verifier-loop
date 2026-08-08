# OpenSpec Task Scout — ragquick vs rolesmart benchmark

**Date:** 2026-07-16
**Goal:** pick ONE concrete verifiable task from `Fission-AI/OpenSpec` (public repo, `master`)
for the verifier-loop to judge under both pi-served models.

## Scout method
- Used `zread` MCP (read-only GitHub API) — NO blind clone of OpenSpec.
- Surveyed `openspec/changes/` (active, non-archived) + `openspec/specs/` + `src/commands/`.
- Filtered for: small surface, concrete behavior, deterministic ground truth,
  and at least one *subtle* discrepancy so the verifier actually has to dig (not just
  approve on read).

## Picked task: `graceful-status-no-changes`

- **Proposal:** `openspec/changes/graceful-status-no-changes/proposal.md` — make
  `openspec status` exit code 0 with friendly message when no changes exist, instead
  of throwing. Fixes issue #714.
- **Spec:** `openspec/changes/graceful-status-no-changes/specs/graceful-status-empty/spec.md`
  - 4 scenarios, exact expected strings + exit codes.
- **Tasks:** `tasks.md` — all 3 boxes ticked `[x]`.
- **Implementation (verified present on master):**
  - `src/commands/workflow/status.ts` — early-return path for `available.length === 0`.
  - `src/commands/workflow/shared.ts` — `getAvailableChanges` exported as public fn.

## Ground-truth signals (discriminators)

The change is genuinely *mostly* implemented, so a careful verifier APPROVES. But there
are *subtle* discrepancies a sharp verifier should catch (or, depending on strictness,
REJECT on):

| # | Spec requires | Impl actual | Type |
|---|---|---|---|
| S1 | text mode msg: `No active changes. Create one with: openspec new change <name>` | matches | OK |
| S2 | json mode msg: `{"changes":[],"message":"No active changes."}` | impl emits `{changes:[], message:'No active changes.', root: rootOutput}` — extra `root` field, also pretty-printed (`null, 2`) not minified | SPEC-DIVERGENCE |
| S3 | "specified change not found" msg: `Change 'non-existent' not found` | impl emits `Change 'non-existent' not found. Available changes:\n  ...` (extra suffix) | SPEC-DIVERGENCE |
| S4 | "missing required option" path: when changes exist but no --change | impl: throws `Missing required option --change. Available changes:\n  ...` | OK |

So ground truth = APPROVE-with-notes *or* REJECT-on-strictness, depending on how literally
the model treats the spec's exact-string contract. **This is exactly the discrimination
signal the benchmark wants**: does rag-quick catch S2/S3 like role-smart does, or does it
rubber-stamp?

## Repo fixture plan (respects "do NOT commit OpenSpec into verifier-loop")
- Clone `Fission-AI/OpenSpec` @ pinned SHA into `/tmp/openspec-bench-<shortsha>/`.
- The verifier's goal text tells it to `cd /tmp/openspec-bench-<shortsha>` before inspecting.
- OpenSpec never enters the verifier-loop repo. Pin SHA recorded here once clone happens.

## Verifier goal text shape (drafted, finalized in t2)
- Title: "Verify `graceful-status-no-changes` OpenSpec change matches its spec".
- Instructs verifier to: `cd` to fixture, run `cat` on the spec + impl files, optionally
  run the actual `openspec status` binary in an empty-changes fixture dir, compare each
  scenario to actual behavior.
- DOD: list every scenario → check string-exact match → APPROVE iff zero mismatches.

## Why this task (vs alternatives)
- `fix-spec-parser-fidelity`, `add-qa-smoke-harness`, etc. — larger, more open-ended,
  harder to define a sharp correctness signal.
- `graceful-status-no-changes` is bounded: 4 scenarios, 2 files, exact strings. The kind
  of task where a verifier CAN reach a deterministic verdict in a few turns.
