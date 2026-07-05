## MODIFIED Requirements

### Requirement: Default templates bake in the verifier policy exactly once

The canonical verifier detective policy text (sourced from `verifier_policy.txt`, LD10) SHALL appear exactly once in every rendered default prompt (round-1 NEW and resume). The policy SHALL be composed at compile time via `concat!` into `DEFAULT_TEMPLATE` / `DEFAULT_RESUME_TEMPLATE`; the template body files (`default_template.txt`, `default_resume_template.txt`) MUST NOT contain a duplicate inline copy of the policy block. The composed prompt SHALL be a single coherent message: identity line → canonical policy → goal → context → (resume only) fix notes / prior notes → frozen artifact snapshot → duty.

#### Scenario: Round-1 prompt contains the policy exactly once
- **WHEN** the round-1 default template is rendered for a verifier
- **THEN** the canonical policy marker text (e.g. the `<_unfold.md>` investigation block heading from `verifier_policy.txt`) appears exactly once in the rendered string
- **AND** the `# Verifier Detective Policy` heading appears exactly once

#### Scenario: Resume prompt contains the policy exactly once
- **WHEN** the resume default template is rendered for a verifier
- **THEN** the canonical policy marker text appears exactly once in the rendered string

#### Scenario: Single stdin write delivers the whole prompt
- **WHEN** a verifier process is spawned with the default template via the stdin transport
- **THEN** the entire rendered prompt is written to the child's stdin as a single contiguous byte stream
- **AND** no portion of the policy is delivered as a separate leading message
