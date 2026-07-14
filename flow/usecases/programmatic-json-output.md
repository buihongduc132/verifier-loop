# Usecase — programmatic driving of jewilo / jewije

## Problem

An outer driving agent, a CI consumer, or a wrapper script that drives the `verifier-loop`
CLIs must today scrape and regex the free-text stdout lines (`goalId: …`, the bare
`mmddyy-XXXXXXXX` hash, `Verdict registered`, multi-line rejection summaries). This is
brittle: any rewording breaks the consumer, success/failure is conflated with exit code +
prose, and rich fields (round, hash, `needs`, rejection breakdown) live on separate lines
with no single parse point. `STATUS` is the only structured output today, and it uses a bare
body rather than a uniform envelope.

## Use case

Drive `jewilo` (`NEW` / `RESUME` / `RECOVER` / `STATUS`) and `jewije` (`approve` / `reject`)
from automation **without scraping free text**. The caller passes the global `--json`
(short `-j`) flag on either binary — placed before or after the subcommand — and receives
**exactly one** JSON object on stdout per invocation, conforming to a single stable
camelCase envelope schema.

## Envelope (the contract)

Field names and semantics are defined in `src/cli/json_output.rs` (`JsonEnvelope`,
`RejectionBreakdown`) and documented in `README.md` → `## JSON output mode (--json)`.
Always present: `ok` (boolean), `command` (`new`|`resume`|`recover`|`status`|`approve`|`reject`).
Additive fields (`goalId`, `round`, `verifierId`, `status`, `hash`, `fullDigest`, `needs`,
`rejection`, `verdicts`, `state`, `error`) are `Option<…>` and omitted when absent.
`status` is one of: `consensus-passed`, `rejected`, `cooldown-fallback`,
`recover-null-after-timeout`, `verdict-registered`.

## Guarantees the consumer relies on

- **One root object** on stdout per process (never an array, never NDJSON), regardless of how
  many internal phases ran.
- **stdout / stderr separation**: stdout is the single structured parse point; all
  human-readable diagnostics stay on stderr. On the error path stdout still gets one
  envelope `{"ok":false,"error":"…"}`; the human text mirrors to stderr.
- **Determinism**: `rejection.rejectNotes` / `nullVerifiers` / `signatureFailures` are sorted
  by `verifierId` ascending; identical logical inputs yield identical parsed content.
- **Byte-identical invariants**: exit codes, the completion-hash inputs, verdict semantics,
  signature verification, and the on-disk artifacts (`completion.json`, `receipt-log.jsonl`,
  `trace.jsonl`) are **unchanged** by `--json`. Default (no `--json`) output is unchanged.

## Sources

- Spec: [`openspec/changes/add-json-output-mode/specs/json-output/spec.md`](../../openspec/changes/add-json-output-mode/specs/json-output/spec.md)
- Design (D0–D8): [`openspec/changes/add-json-output-mode/design.md`](../../openspec/changes/add-json-output-mode/design.md)
- Implementation: [`src/cli/json_output.rs`](../../src/cli/json_output.rs)
- README section: [`README.md` → JSON output mode](../../README.md#json-output-mode---json)
