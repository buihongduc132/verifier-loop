# Tasks — add-json-output-mode

Implementation roadmap. Follows the repo's standing TDD discipline (`AGENTS.md`):
**RED test by one fresh teammate → GREEN impl by a different fresh teammate → coverage gate `>=80%` lines per new src file before the group is done.**
Every group below is one RED+GREEN pair. `--json` is purely an output-format layer (design D6): no group may alter hash inputs, verdict semantics, signature verification, on-disk artifact bytes, or exit codes.

Reference: proposal.md (why), design.md (decisions D0–D8), specs/json-output/spec.md (WHAT — each scenario is a test case).

## 1. Envelope type + formatter scaffold (json-output spec; design D1, D6, D8)

- [ ] 1.1 Create `src/cli/json_output.rs` (and `pub mod json_output;` from `src/cli/mod.rs`). Define `#[derive(Debug, Serialize)] pub struct JsonEnvelope` with camelCase serde renames and `#[serde(skip_serializing_if = "Option::is_none")]` on every `Option` field. Fields: `ok: bool`, `command: String`, `goalId: Option<String>`, `round: Option<u32>`, `verifierId: Option<String>`, `status: Option<String>`, `hash: Option<String>`, `fullDigest: Option<String>`, `needs: Option<String>`, `rejection: Option<RejectionBreakdown>`, `verdicts: Option<serde_json::Value>`, `state: Option<String>`, `error: Option<String>`. Define `RejectionBreakdown { rejectNotes: Vec<(String,String)>, nullVerifiers: Vec<String>, signatureFailures: Vec<(String,String)> }` (sorted-by-verifierId at construction).
- [ ] 1.2 RED: `envelope_serializes_camelcase_and_skips_none` — build an envelope with only `ok:true` + `command:"new"` set; assert the serialized JSON has exactly those two keys and no `hash`/`error`/etc. Then set `hash` + `round` and assert they appear; assert a snake_case key (`goal_id`) NEVER appears in any serialization.
- [ ] 1.3 RED: `envelope_rejection_arrays_sorted_by_verifier_id` — construct a `RejectionBreakdown` with notes from `v3` then `v1`; assert serialized `rejectNotes` is ordered `v1`, `v3`.
- [ ] 1.4 GREEN: implement `JsonEnvelope` + `RejectionBreakdown` (sort in a constructor `RejectionBreakdown::from_unsorted(...)`). Satisfies 1.2–1.3. Different author than the RED tests.
- [ ] 1.5 Add `pub enum Output { Human, Json }` + helpers `fn print_success(&self, env: &JsonEnvelope, human_line: &str)` and `fn print_error(&self, env: &JsonEnvelope, human_err: &str)` — under `Json`, print exactly one `serde_json::to_string` line to stdout; under `Human`, print the legacy line(s). Errors also print the human text to stderr under both modes (stderr is the debugging channel).
- [ ] 1.6 RED: `print_success_json_emits_exactly_one_stdout_line` — under `Output::Json`, calling `print_success` writes exactly one line to stdout that parses as JSON matching the envelope; under `Output::Human`, writes the `human_line` verbatim and no JSON.
- [ ] 1.7 GREEN: implement the formatter. Coverage gate: `cargo llvm-cov --fail-under-lines 80` for `src/cli/json_output.rs`.

## 2. Top-level `--json` flag on both binaries (json-output spec; design D2)

- [ ] 2.1 RED: `jewilo_flag_parses_before_subcommand` — `jewilo --json NEW "<goal>"` parses with `json == true`; `jewilo NEW "<goal>"` parses with `json == false`. Also `jewilo NEW "<goal>" --json` (post-subcommand placement) parses with `json == true` if clap permits; if not, document the single supported placement and update the spec scenario.
- [ ] 2.2 RED: `jewije_flag_parses_on_approve_and_reject` — `jewije --json approve` and `jewije reject --notes "x" --json` both parse with `json == true`.
- [ ] 2.3 GREEN: add `#[arg(long, short = 'j', global = true)] json: bool` to the top-level `Cli` of `jewilo` (`VerifierLoopCli` in `src/cli/mod.rs`) and `jewije` (`Cli` in `src/bin/verifier_verdict.rs`). Thread the bool into `run(...)`. Different author than the RED tests.
- [ ] 2.4 Coverage gate for `src/cli/mod.rs`.

## 3. `jewilo NEW` / `RESUME` consensus-passed + cooldown paths under `--json` (json-output spec; design D3, D4, D7)

- [ ] 3.1 RED: `jewilo_new_json_consensus_passed_envelope` — stub backend, `jewilo --json NEW "<goal>"` reaches consensus; stdout is exactly one JSON object with `ok:true`, `command:"new"`, `goalId`, `round:1`, `status:"consensus-passed"`, `hash`, `fullDigest`; stdout does NOT contain `goalId:` or a bare hash; exit 0.
- [ ] 3.2 RED: `jewilo_new_default_is_byte_identical_to_legacy` — same round without `--json` → stdout first line is `goalId: <id>` and last line is the bare short hash; no JSON object.
- [ ] 3.3 RED: `jewilo_resume_json_consensus_passed` — NEW then `jewilo --json RESUME <goalId>` → one envelope object `command:"resume"`, `round:2`, `status:"consensus-passed"`.
- [ ] 3.4 RED: `jewilo_new_json_cooldown_fallback_envelope` — force >3 unhealthy records in the last hour, then `jewilo --json NEW "<goal>"` → envelope `ok:true`, `status:"cooldown-fallback"`, `hash` == `<mmddyy>-ffffff`; human cooldown notice still on stderr only.
- [ ] 3.5 RED: `completion_json_byte_identical_with_and_without_json` — drive the same goal+round once with `--json` and once without; assert `completion.json` is byte-identical and both `hash`+`fullDigest` match.
- [ ] 3.6 GREEN: route the `run_new` / `run_resume` / `run_round` success + cooldown sites in `src/bin/verifier_loop.rs` through the formatter (D6). Populate the envelope from the in-memory `result` / `hash` / fallback values. Different author than the RED tests.
- [ ] 3.7 Coverage gate for the touched sections of `src/bin/verifier_loop.rs`.

## 4. `jewilo` rejection + error paths under `--json` (json-output spec; design D4, D7)

- [ ] 4.1 RED: `jewilo_new_json_rejection_envelope_sorted` — stub backend returns REJECT from `v3` and `v1`; `jewilo --json NEW "<goal>"` → one envelope `ok:false`, `status:"rejected"`, `rejection.rejectNotes` sorted `v1`,`v3`, `nullVerifiers`, `signatureFailures` arrays present; exit non-zero; stderr still carries the human rejection lines.
- [ ] 4.2 RED: `jewilo_new_json_validation_error_envelope` — `jewilo --json NEW ""` (empty goal) → one envelope `ok:false`, `error` describing the empty-goal failure; exit non-zero; no partial JSON on stdout.
- [ ] 4.3 RED: `jewilo_new_json_missing_store_error_envelope` — unset `VERIFIER_LOOP_HOME` to an unwritable path → one envelope `ok:false` with an `error` string; exit non-zero.
- [ ] 4.4 GREEN: route the rejection branch + the top-level `run()` error return through the formatter. The error envelope is built from the `Err(msg)` value. Different author than the RED tests.
- [ ] 4.5 Coverage gate for the touched sections.

## 5. `jewilo RECOVER` + `STATUS` under `--json` (json-output spec; design D3)

- [ ] 5.1 RED: `jewilo_recover_json_consensus_passed` — pre-seed in-flight verdicts, `jewilo --json RECOVER <goalId>` → envelope `ok:true`, `command:"recover"`, `status:"consensus-passed"`, `hash`, `round`.
- [ ] 5.2 RED: `jewilo_recover_json_null_after_timeout` — recover times out with null slots → envelope `ok:false`, `status:"recover-null-after-timeout"`, structured null-slots field; exit non-zero; guidance on stderr only.
- [ ] 5.3 RED: `jewilo_status_json_wraps_body_in_envelope` — `jewilo --json STATUS <goalId>` → one envelope `ok:true`, `command:"status"`, `goalId`, `round`, `state`, `needs`, `verdicts` preserved.
- [ ] 5.4 RED: `jewilo_status_default_byte_identical_to_legacy` — `jewilo STATUS <goalId>` (no `--json`) → bare JSON body (round, state, needs, verdicts), no `ok`/`command` wrapper, byte-identical to before this change.
- [ ] 5.5 GREEN: route `run_recover` + `run_status` through the formatter; for STATUS, lift `round`/`state`/`needs` into the envelope and pass the existing body as `verdicts`/extra fields only under `--json`. Different author than the RED tests.
- [ ] 5.6 Coverage gate for the touched sections.

## 6. `jewije` approve / reject under `--json` (json-output spec; design D1, D4)

- [ ] 6.1 RED: `jewije_approve_json_success_envelope` — `jewije --json approve` inside a V* (stub) with valid env → one envelope `ok:true`, `command:"approve"`, `goalId`, `verifierId`, `round`, `status:"verdict-registered"`; stdout has NO `Verdict registered`; exit 0.
- [ ] 6.2 RED: `jewije_reject_json_success_envelope` — `jewije reject --notes "broken" --json` → envelope `command:"reject"`, `status:"verdict-registered"`, `ok:true`.
- [ ] 6.3 RED: `jewije_default_success_is_byte_identical` — `jewije approve` without `--json` → stdout is exactly `Verdict registered`; no JSON object.
- [ ] 6.4 RED: `jewije_json_notes_required_error_envelope` — `jewije reject --notes "" --json` (or missing notes) → one envelope `ok:false`, `error` describing notes-required; exit non-zero; no human error text on stdout.
- [ ] 6.5 RED: `jewije_json_unauthenticated_error_envelope` — pinned slot + missing secret under `--json` → envelope `ok:false`, `error` describing the missing secret; exit non-zero.
- [ ] 6.6 RED: `receipt_log_byte_identical_with_and_without_json` — drive `jewije approve` once with `--json` and once without on equivalent slots; assert the appended receipt-log entries are byte-identical.
- [ ] 6.7 GREEN: route the success `println!("Verdict registered")` and the `run()` error return in `src/bin/verifier_verdict.rs` through the formatter. Build the envelope from resolved identity (`goalId`/`verifierId`/`round`) + the `approve`/`reject` command. Different author than the RED tests.
- [ ] 6.8 Coverage gate for the touched sections of `src/bin/verifier_verdict.rs`.

## 7. Determinism + single-object invariants (json-output spec; design D0, D5)

- [ ] 7.1 RED: `jewilo_resume_m5_json_emits_exactly_one_object` — `m=5` RESUME reaching consensus under `--json` → stdout contains exactly one JSON root object (count of `{` ... `}` top-level == 1), not one per verifier.
- [ ] 7.2 RED: `envelope_deterministic_content_for_identical_outcomes` — run the same logical NEW twice under `--json` with a frozen stub backend → parse both envelopes; assert the field sets and values are identical (field order may differ).
- [ ] 7.3 GREEN (if 7.1/7.2 fail): ensure no internal phase prints a stray JSON object; ensure array sorting is applied at construction. Likely no new code if groups 3–6 are correct — this group exists to lock the invariant.
- [ ] 7.4 Coverage gate for any newly-touched file.

## 8. Docs + AGENTS.md

- [ ] 8.1 Add a "JSON output mode" section to `README.md` documenting the `--json` flag, the envelope schema (camelCase fields), the per-command `status` values, and the stdout/stderr separation guarantee.
- [ ] 8.2 Add a one-line pointer in `AGENTS.md` noting the machine-readable `--json` contract on both binaries (so the outer driving agent can adopt it instead of scraping).
- [ ] 8.3 Add a `flow/usecases/` entry capturing the programmatic-usage usecase this change serves (per the repo's usecase discipline), citing the envelope schema.
