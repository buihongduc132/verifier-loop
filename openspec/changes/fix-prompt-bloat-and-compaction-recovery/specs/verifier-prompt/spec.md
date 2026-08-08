## MODIFIED Requirements

### Requirement: Frozen artifact snapshot is captured at spawn
At spawn time the CLI SHALL capture `cwd`, `git status --porcelain`, file edit times **scoped to changed files only** (`git status --porcelain`, not `git ls-files`), and `git diff` truncated to `gitDiffMaxChars` (default 10000). The file edit times block SHALL additionally be capped to `fileEditTimesMaxChars` (default 8000). The `--context` input SHALL be capped to `contextMaxChars` (default 20000). This snapshot is frozen for the round and rendered into every verifier's prompt.

#### Scenario: Diff is truncated
- **WHEN** the git diff exceeds `gitDiffMaxChars`
- **THEN** the rendered `{{gitDiff}}` is truncated to that many characters with an indicator that truncation occurred

#### Scenario: fileEditTimes scoped to changed files
- **WHEN** a repo has 4,000 tracked files but only 12 changed
- **THEN** the rendered fileEditTimes block contains entries for only the changed files
- **AND** the block does not enumerate unchanged tracked files

#### Scenario: fileEditTimes is byte-capped
- **WHEN** the changed-files fileEditTimes block exceeds `fileEditTimesMaxChars`
- **THEN** the rendered block is truncated to that many bytes with an indicator that truncation occurred

#### Scenario: Context is capped
- **WHEN** the `--context` input exceeds `contextMaxChars`
- **THEN** the rendered context is truncated to that many characters with an indicator that truncation occurred

#### Scenario: Snapshot is consistent within a round
- **WHEN** two verifiers are spawned in the same round
- **THEN** both receive byte-identical artifact snapshots

## ADDED Requirements

### Requirement: Policy is not duplicated when custom verifierPromptFile is set
When `verifierPromptFile` is set in config, the rendered prompt SHALL include the custom policy file content exactly once and SHALL NOT also embed the built-in `VERIFIER_POLICY` constant. When `verifierPromptFile` is null, the built-in policy is used. The two policy sources are mutually exclusive (override semantics, not additive).

#### Scenario: Custom file overrides built-in policy
- **WHEN** config has `verifierPromptFile: "/path/to/custom.md"`
- **THEN** the rendered prompt contains the custom file content once
- **AND** the rendered prompt does not contain the built-in `VERIFIER_POLICY` text

#### Scenario: Null file uses built-in policy
- **WHEN** config has `verifierPromptFile: null`
- **THEN** the rendered prompt contains the built-in `VERIFIER_POLICY` exactly once

### Requirement: Rendered prompt budget warning
After rendering the full verifier prompt, if the total byte size exceeds `promptBudgetBytes` (default 50000), the CLI SHALL emit a warning to stderr with a per-section byte breakdown (policy, fileEditTimes, gitDiff, context, goal). The warning SHALL NOT block the spawn.

#### Scenario: Oversize prompt warns but proceeds
- **WHEN** the rendered prompt is 120,000 bytes and `promptBudgetBytes` is 50000
- **THEN** a warning is printed to stderr listing per-section sizes
- **AND** the verifier is still spawned

#### Scenario: On-budget prompt does not warn
- **WHEN** the rendered prompt is 30,000 bytes and `promptBudgetBytes` is 50000
- **THEN** no budget warning is printed
