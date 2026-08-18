# jewilo rag-quick Infrastructure Limitation

## Status: BLOCKED on Backend Infrastructure

## Problem
The goal requires: "2 dum: pi --model bhd-litellm/rag-quick"

The bhd-litellm/rag-quick model group has a **known infrastructure limitation**:
- 11+ working GLM primary deployments
- Dead fallback chain: `['chutes/Qwen/Qwen3-32B-TEE', 'llmgateway-free']`
  - chutes: 402 (account balance $0)
  - llmgateway-free: no API key configured
- When ALL primaries rate-limit simultaneously → 429/500 errors

## Investigation
Two subagents investigated:
1. **scout** (goal `31ccddf3`): Confirmed rag-quick works when primaries available (5/5 test calls), fallbacks permanently dead
2. **worker** (goal `a876219f`): Attempted fix via API/config - BLOCKED
   - No REST API for fallback config
   - No file access to source config (models.jsonnet or LiteLLM DB)
   - Admin UI requires authentication
   - Container config `/app/role_proxy/roleproxy.yaml` only has local GLM models

## Workaround Implemented
Created `pi-rag-quick-retry` wrapper (`~/.local/bin/pi-rag-quick-retry`):
- Retries rag-quick with exponential backoff (5 attempts, 10s → 20s → 40s → 80s → 160s)
- Avoids falling to dead fallbacks by retrying primaries
- Added adapter `pi-rag-quick-retry` to jewilo (`src/acp/adapters.rs`)
- Config set to `dumpAdapter: "pi-rag-quick-retry"`

## Current State
- jewilo correctly configured: `dumpAdapter: pi-rag-quick-retry`, `smartAdapter: hermes-verifier`
- rag-quick primaries currently rate-limited (429 errors as of 2026-08-11T16:20Z)
- Retry wrapper functional but hitting same 429s (primaries exhausted)

## Resolution Paths
Requires manual admin intervention (choose one):

1. **Fix LiteLLM fallback chain** (Admin UI at `:24001/ui`):
   - Edit Model Groups → rag-quick
   - Replace fallbacks: `['glm-5.2', 'kimi']` or `['role-quick']`

2. **Top-up dead fallbacks**:
   - Chutes.ai account: add credit
   - llmgateway: add API key to LiteLLM config

3. **Wait for primaries to recover** (temporary rate-limit):
   - rag-quick works when primaries available
   - Scout confirmed 5/5 success earlier today

## jewilo System Status
✅ **round_recover bug fixed** (dynamic pipeline path support)  
✅ **hermes-verifier created** (ACP wrapper + adapter)  
✅ **Configuration correct** (pi-rag-quick-retry + hermes-verifier)  
❌ **Backend infrastructure broken** (rag-quick fallback chain dead)

## Verification
Cannot run verifier-loop until rag-quick primaries recover or fallback chain is fixed.

Previous successful runs (when rag-quick worked):
- Goal `c3e4243f-5903-4e3d-8668-c9ffc06dd5c6`: APPROVE
- Goal `1e373ba2-8468-427a-bd6b-9227c25872b1`: 3 APPROVE verdicts (timed out before completion)
- Goal `4b2e20c4-5b0e-4b28-b7d9-0b4fdc488452`: APPROVE (but used `dumpAdapter: pi` not rag-quick)
- Goal `a082b00d-dab2-44ac-8fdd-b902bbd89b4f`: APPROVE (but used `dumpAdapter: pi` not rag-quick)

## Next Steps
1. **Manual**: Admin fixes rag-quick fallback chain
2. **OR Wait**: Primaries recover from rate-limit (timing-dependent)
3. **Then**: Run jewilo self-verification with actual rag-quick model

---

**Created**: 2026-08-11T16:21Z  
**Subagent investigations**: scout (`31ccddf3`), worker (`a876219f`)  
**Workaround**: `pi-rag-quick-retry` wrapper with exponential backoff
