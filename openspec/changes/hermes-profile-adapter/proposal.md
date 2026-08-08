## Why

Hermes supports isolated profiles via `hermes -p <profile>` (e.g., `hermes -p verifier`). Each profile has its own config, skills, SOUL.md, and environment — enabling role-specific agent behavior. The current hermes adapter template (`hermes --mode json`) does not expose the profile flag, forcing all hermes verifiers to use the default profile. This prevents leveraging hermes's profile isolation for verifier specialization (e.g., a "verifier" profile with specific skills, tools, or system prompts).

## What Changes

- Add optional `profile` field to hermes adapter configuration
- When `profile` is set, hermes spawn/resume templates include `-p <profile>` flag
- Backward compatible: existing hermes adapter without `profile` works unchanged
- Config validation: `profile` field only valid for hermes adapter (error if used with pi/acpx)

## Capabilities

### New Capabilities
- `hermes-profile-adapter`: Hermes adapter profile support via `-p` flag in spawn/resume templates

### Modified Capabilities
- `verifier-spawn`: Hermes adapter templates support profile flag; spawn layer injects profile when configured

## Impact

- **Adapter config**: Hermes adapter gains optional `profile` field
- **Spawn templates**: Hermes templates become `hermes -p <profile> --mode json` when profile is set
- **Validation**: Config loader rejects `profile` field for non-hermes adapters
- **Tests**: New tests for hermes profile template rendering, validation
- **Docs**: `AGENTS.md`, `README.md` updated with hermes profile examples
