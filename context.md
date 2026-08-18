# Code Context — rag-quick 403 "An active plan is required" Investigation

## Verdict

**rag-quick is NOT permanently broken.** Primary deployments work right now (5/5 test calls succeeded via zai glm-4.7-flash). The problem is the **fallback chain is permanently broken**: when ALL 11 primary deployments simultaneously fail (rate limits, timeouts, local down), LiteLLM falls to 2 dead fallbacks. It's intermittent/fragile, not dead.

## Root Cause

The `rag-quick` fallback chain (stored in LiteLLM PostgreSQL DB, **not** the YAML) is:

```
rag-quick → chutes/Qwen/Qwen3-32B-TEE → llmgateway-free
```

Both fallbacks are **permanently dead**:

| Fallback | Error | Root Cause |
|----------|-------|------------|
| `chutes/Qwen/Qwen3-32B-TEE` | 402 "Quota exceeded and account balance is $0.0, please pay with fiat or send tao to ..." | Chutes.ai account has **$0 balance**. "An active plan is required" = Chutes.ai message when no active subscription. |
| `llmgateway-free` | "api_key client option must be set" | **No API key configured** — `litellm_credential_name: null`, `api_key: null` in DB. |

When all 11 primary deployments are down simultaneously → fallback to chutes (dead) → fallback to llmgateway-free (dead) → 403.

## Files Retrieved

1. `~/Documents/Projects/bhd/llm-configuration/config/litellm/24001/config.yaml` (lines 1-80) — LiteLLM primary config. Key: `store_model_in_db: true` → models live in PostgreSQL, NOT in YAML. `master_key: sk-litellm-asdwasdq`. Admin UI at `http://100.114.135.99:24001/ui`.
2. `~/Documents/Projects/bhd/llm-configuration/config/models.jsonnet` (lines 270-300, 485) — Source of truth for model definitions + fallback chains. Line 485: `{ "rag-quick": ["chutes/Qwen/Qwen3-32B-TEE", "llmgateway-free"] }`.
3. `~/Documents/Projects/bhd/verifier-loop/src/acp/adapters.rs` (lines 167-175) — `pi-rag-quick` adapter spawns `pi --model bhd-litellm/rag-quick --mode json`.
4. `~/.pi/agent/auth.json` — `bhd-litellm` provider key: `sk-litellm-asdwasdq` (this IS the master key).
5. `~/.verifier-loop/config.json` — Live config: `dumpAdapter: "pi-rag-quick"`, `smartAdapter: "pi"`.
6. `~/.verifier-loop/health.jsonl` — 13 unhealthy events (Aug 9-10), zero on Aug 11.

## Key Code

### Adapter wiring (adapters.rs:167)
```rust
"pi-rag-quick" => Ok(Adapter {
    spawn: "pi --model bhd-litellm/rag-quick --mode json".to_string(),
    resume: "pi --session {sid} --model bhd-litellm/rag-quick --mode json".to_string(),
    ..Default::default()
}),
```

### rag-quick live deployments (11 total, from `/v1/model/info`)
| # | Model | API Base | Status |
|---|-------|----------|--------|
| 1-6,9 | `glm-4.7-flash` | `api.z.ai/api/paas/v4` | ✅ Working (zai-free, zhipu-key1/key2) |
| 7-8 | `glm-4.5-flash` | `api.z.ai/api/coding/paas/v4` | ✅ Working |
| 3 | `openai/claude-haiku-4-5-free` | `api.llmgateway.io/v1` | ⚠️ Has key under rag-quick group, but standalone `llmgateway-free` group has NONE |
| 10 | `cyankiwi/Qwen3.5-4B-AWQ-4bit` | `http://100.114.135.99:8032/v1` | ✅ Local TabbyAPI UP |
| 11 | `Qwen/Qwen3.5-9B` | `api.featherless.ai/v1` | ⚠️ Requires auth (key in LiteLLM creds) |

### Fallback chain (live in DB, differs from config.yaml)
```
chutes/Qwen/Qwen3-32B-TEE → llmgateway-free
```
- **config.yaml fallbacks** (lines 52-66) do NOT include rag-quick — the DB has a different config than the YAML.
- The running fallback list (from error output) includes: `role-smart`, `role-smart-rev`, `role-smart-kimi`, `rag-quick`, `rag-long`.

## Architecture (data flow)

```
jewilo (dumpAdapter=pi-rag-quick)
  → spawns: pi --model bhd-litellm/rag-quick --mode json
    → pi reads auth.json: bhd-litellm.apiKey = sk-litellm-asdwasdq
    → pi sends to LiteLLM proxy at 100.114.135.99:24001
      → LiteLLM routes "rag-quick" model group
        → tries 11 primary deployments (round-robin)
        → if ALL fail → fallback chain: chutes (dead: $0) → llmgateway-free (dead: no key)
        → if fallbacks also fail → 403/402 error bubbles up to pi → jewilo marks V* as unhealthy
```

## Config Fix Options

### Option A: Fix the broken fallbacks (chutes + llmgateway)
- **chutes**: Add funds to Chutes.ai account (send tao/pay fiat) — external billing action.
- **llmgateway-free**: Add API key via Admin UI (`http://100.114.135.99:24001/ui`) → edit model group `llmgateway-free` → set credential.

### Option B (recommended): Replace dead fallbacks with working model groups
Via LiteLLM Admin UI or API (`POST /model/update`), change `rag-quick` fallback from:
```
["chutes/Qwen/Qwen3-32B-TEE", "llmgateway-free"]
```
to something that actually works, e.g.:
```
["role-quick", "glm-4-flash/fallback"]
```
Then sync the change back to `config/models.jsonnet:485` (source of truth).

### Option C: Remove rag-quick fallback entirely
If the 11 primary deployments are sufficient, remove the fallback chain. Risk: when all primaries rate-limit simultaneously, no safety net.

## Start Here

**`~/Documents/Projects/bhd/llm-configuration/config/models.jsonnet` line 485** — the fallback chain source of truth. The live config is in the LiteLLM PostgreSQL DB (manage via Admin UI at `http://100.114.135.99:24001/ui` or `/v1/model/info` API). Fix must be applied in BOTH places.

## Key Evidence

- `curl -H "Authorization: Bearer sk-litellm-asdwasdq" rag-quick` → **works** (5/5, returned valid completions via glm-4.7-flash)
- `curl chutes/Qwen/Qwen3-32B-TEE` → 402 "Quota exceeded and account balance is $0.0"
- `curl llmgateway-free` → 500 "api_key client option must be set" (no credential)
- Local TabbyAPI `:8032` → UP (serving `cyankiwi/Qwen3.5-4B-AWQ-4bit`)
- `store_model_in_db: true` → YAML model_list is IGNORED; models in PostgreSQL
- Goal `c3e4243f` (completed successfully) used `dumpAdapter: "pi-rag"` (not pi-rag-quick) — different adapter
- Latest goal `d4cc0a75` has `dumpAdapter: null` — using default backend, not rag-quick

## Open Questions

1. **chutes.ai billing**: Who owns the Chutes account? Is it intended as a free tier or paid? Balance needs top-up or removal.
2. **llmgateway-free key**: Was the API key ever configured? The `openai/claude-haiku-4-5-free` model exists under rag-quick WITH a key, but the standalone `llmgateway-free` group has no credential — they may share the same upstream but different DB entries.
3. **jsonnet → DB sync**: How does `models.jsonnet` get loaded into the PostgreSQL DB? (Is there a register script? The jsonnet comments mention `register_all_models.py`.) Any DB-side fix needs to be reflected in the jsonnet source to survive re-registration.
