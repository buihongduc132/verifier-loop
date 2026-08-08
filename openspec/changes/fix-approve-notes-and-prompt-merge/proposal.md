## Why

Two usability bugs block the primary `jewilo`/`jewije` verifier path:

1. **`verifier-verdict approve` rejects `--notes`** — the subcommand takes no arguments, so a verifier that runs `verifier-verdict approve --notes "evidence..."` (mirroring the `reject` shape, and the documented example in the verifier prompt's own "register your verdict via the verifier-verdict CLI (approve / reject --notes \"...\")" line) aborts with `error: unexpected argument --notes found` and the verdict is never written. The slot stays null → no consensus → no hash, even though the verifier reached an APPROVE decision. Today the only way to attach approval evidence is to omit it entirely.

2. **The rendered verifier prompt is delivered as two perceived messages** — `DEFAULT_TEMPLATE` (`src/prompt/mod.rs:88`) prepends the canonical `verifier_policy.txt` via `concat!`, **and** `default_template.txt` *also* embeds a full "# Verifier Detective Policy" block inline. So the policy text (`<_unfold.md>` investigation rules + detective bullets) appears **twice** in the single stdin write. The spawned backend (pi) renders this as: a large standalone policy block, a `---`, then a second policy block followed by the goal — which the verifier and humans read as two prompts, not one. The first "half" (policy only) gets a stray response before the real review duty arrives.

## What Changes

- **`verifier-verdict approve` gains an optional `--notes`** (and a short `-n` alias). `--notes` is optional on approve (not required, unlike reject). When supplied and non-empty, the notes are stored in `verdict.json` (`notes: Some(...)`); when omitted, behavior is unchanged (`notes: None`). Applies identically to the signed-approve path (`register_signed_approve`).
- **Verifier prompt becomes a single coherent message**: `default_template.txt` and `default_resume_template.txt` are stripped of their inline "# Verifier Detective Policy" blocks. The policy text lives in exactly one place — `verifier_policy.txt` — and is composed into the final prompt via the existing `concat!` in `DEFAULT_TEMPLATE` / `DEFAULT_RESUME_TEMPLATE`. The rendered round-1 prompt is now exactly: identity line → canonical policy → goal → context → frozen snapshot → duty.
- No spec changes to consensus, completion-proof, or receipt-log. First-write-wins, fail-closed, and signature semantics are untouched.

## Capabilities

### New Capabilities
<!-- none -->

### Modified Capabilities
- `verdict-registration`: The `approve` subcommand now accepts an OPTIONAL `--notes` (empty-by-default). Reject keeps `--notes` as required. Notes on approve are stored verbatim (trimmed) on the `APPROVE` verdict record and are part of the canonical signed bytes.
- `verifier-prompt`: The default round-1 and resume templates render the verifier policy exactly once. The template body files no longer carry an inline duplicate policy block; the policy is composed at compile time via `concat!` from `verifier_policy.txt`.

## Impact

- **Code**:
  - `src/bin/verifier_verdict.rs` — `Cmd::Approve` gains `#[arg(long, short = 'n')] notes: Option<String>`; the approve match arms thread `notes.as_deref()` into `register_approve` / `register_signed_approve`.
  - `src/verdict/mod.rs` — `register_approve` and `register_signed_approve` gain an optional `notes: Option<&str>` parameter; trimmed notes are stored on the `APPROVE` `VerdictRecord`. `build_signed_record` already takes `notes: Option<&str>` (no signature change). The canonical bytes for an APPROVE now include notes when present (signature binding already supports this — `notes.as_deref()` is passed through).
  - `src/cli/mod.rs` — no change (CLI subcommand wiring lives in the bin).
  - `src/prompt/mod.rs` — `DEFAULT_TEMPLATE` / `DEFAULT_RESUME_TEMPLATE` `concat!` unchanged; `default_template.txt` / `default_resume_template.txt` edited to drop their duplicate inline policy block (keep identity + goal + context + snapshot + duty only).
- **APIs/Config**: `register_approve` / `register_signed_approve` signatures change (new optional param). No config schema change. No adapter change.
- **Dependencies**: none.
- **Tests**:
  - New: `approve --notes "..."` writes `notes` on the verdict (unsigned + signed paths).
  - New: `approve` with no `--notes` keeps `notes: None` (regression guard).
  - New: signed approve with notes — signature verifies (canonical bytes include notes).
  - Updated: rendered round-1 prompt contains the policy exactly once (assert `verifier_policy.txt` marker substring appears once, not twice).
  - Updated: existing approve tests call the new signature.
- **Specs**: delta on `verdict-registration` (approve optional notes), delta on `verifier-prompt` (single policy rendering).
- **Out of scope**: consensus/proof hashing, receipt-log, maxTurn/refresh, OT1–OT6 deferred items, custom adapter templates.
