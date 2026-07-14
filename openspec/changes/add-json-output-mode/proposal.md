## Why

Both `jewilo` (`verifier-loop`) and `jewije` (`verifier-verdict`) print free-text, human-readable output to stdout/stderr (`goalId: …`, a bare `mmddyy-XXXXXXXX` hash, `Verdict registered`, multi-line rejection summaries, `STATUS` is the only JSON today). Any programmatic caller — an outer driving agent, a wrapper script, or a CI consumer — must scrape and regex these lines. This is brittle: any rewording breaks the consumer, the success/failure signal is conflated with exit code + prose, and there is no structured place to surface rich fields (round, hash, goalId, needs, rejection reasons) in one parse. A stable `--json` mode on every command makes the CLIs safe to drive from automation.

## What Changes

- **New global flag `--json` on `jewilo`** (top-level, applies to `NEW` / `RESUME` / `RECOVER` / `STATUS`). When set, `jewilo` emits exactly one JSON object on stdout and suppresses the legacy free-text lines; human diagnostics stay on stderr.
- **New global flag `--json` on `jewije`** (applies to `approve` / `reject`). When set, the success line and the structured verdict-record echo are emitted as one JSON object on stdout.
- **Stable JSON envelope schema** with: `ok` (bool), `command`, `goalId`, `round`, `status` (e.g. `consensus-passed` / `rejected` / `verdict-registered` / `cooldown-fallback`), `hash` (short form when present), `fullDigest` (when present), `needs` (RECOVER/RESUME/Done hint), `rejection` (reject-notes / null-verifiers / signature-failures), and `error` (string) on failure. All field names camelCase to match the on-disk artifact convention.
- **`STATUS` already prints JSON today** — `--json` makes it conform to the same envelope (`ok:true`, `command:"status"`, plus the existing `round` / `state` / `needs` / `verdicts` body) so consumers parse one schema, not two.
- **Exit codes unchanged.** `--json` is additive output shape only; the existing `SUCCESS` / `FAILURE` exit-code contract and all fail-closed invariants are preserved. `--json` MUST NOT cause a NULL verdict to become APPROVE or weaken any signature/hash invariant.
- **Default (no `--json`) behavior is byte-identical to today.** No breaking change to the human-readable path or to the on-disk `completion.json` / `receipt-log.jsonl` / `trace.jsonl` artifacts — those are unaffected; `--json` shapes stdout only.

## Capabilities

### New Capabilities
- `json-output`: Machine-readable JSON output mode for `jewilo` and `jewije`. Defines the stable envelope schema, the `--json` flag semantics, the per-command status values, the stderr/stdout separation, and the invariant that `--json` does not alter hash/verdict/exit-code behavior.

### Modified Capabilities
<!-- No existing spec's REQUIREMENTS change. The on-disk artifacts, hash inputs, verdict
     semantics, and exit codes are unchanged. json-output is purely an output-format
     capability layered on top of the existing CLI commands. -->

## Impact

- **Code**: `src/cli/mod.rs` (add `--json` flag to `VerifierLoopCli`); `src/bin/verifier_loop.rs` (route every `println!`/`eprintln!` success+diagnostic site through an output formatter chosen by the flag); `src/bin/verifier_verdict.rs` (add `--json` to its `Cli`, route the success line + error path). A small new helper module (e.g. `src/cli/json_output.rs` or fold into `cli/mod.rs`) for the envelope type + serializers.
- **APIs**: Public CLI surface gains `--json` / `-j` on both binaries. No library API breakage.
- **Dependencies**: Likely none — `serde_json` is already a dependency. May add a typed envelope struct behind `serde`.
- **Tests**: New RED tests per command path (success, reject, cooldown, recover-still-null, status) asserting stdout is exactly one valid JSON object matching the envelope; default-path tests asserting byte-identical legacy output.
- **Docs**: `README.md` documents the JSON envelope and the `--json` flag. `AGENTS.md` notes the machine-readable contract.
- **Out of scope**: Structured logging beyond the existing tracing layer; a streaming/NDJSON mode (one object total, not one per phase); changing `trace.jsonl` / `receipt-log.jsonl` / `completion.json` on-disk formats.
