#!/usr/bin/env bash
# scripts/smoke-pi.sh — bare-minimum pi boot probe.
#
# Runs pi with EVERYTHING optional disabled (extensions, skills, prompt templates,
# context files, tools) + offline + no-session, then asserts pi responds.
# Purpose: prove the pi binary itself boots and answers with the absolute minimum
# config surface. Catches: broken core, bad model routing, missing provider keys,
# corrupted config that even -ne/-np/-ns/-nc cannot survive.
#
# Usage:
#   scripts/smoke-pi.sh                     # uses SMOKE_PI_MODEL or default
#   SMOKE_PI_MODEL=zai/glm-4.7 scripts/smoke-pi.sh
#   SMOKE_PI_TIMEOUT=120 scripts/smoke-pi.sh
#
# Exit codes: 0 = pi responded "HI"; non-zero = pi broken or timed out.
#
# See: AGENTS.md "Post-deploy smoke" probe pattern.

set -euo pipefail

# ── tunables ─────────────────────────────────────────────────────────────────
# Fast/cheap model. Override per-machine via env. Default = a known-fast provider
# on the bhd machine; swap for other hosts.
SMOKE_PI_MODEL="${SMOKE_PI_MODEL:-bhd-litellm/rag-quick}"
# Total wall-clock budget. pi startup + provider handshake can take 30-55s even
# offline (the LLM call still fires); give generous headroom.
SMOKE_PI_TIMEOUT="${SMOKE_PI_TIMEOUT:-90}"
SMOKE_PI_PROBE_NAME="${SMOKE_PI_PROBE_NAME:-probe_precommit}"

# ── preflight ────────────────────────────────────────────────────────────────
if ! command -v pi >/dev/null 2>&1; then
  echo "smoke-pi: FAIL — 'pi' not on PATH" >&2
  exit 2
fi

# ── run ──────────────────────────────────────────────────────────────────────
# Bare-minimum flags:
#   -n probe_precommit   name session for find/cleanup
#   -ne                  no extensions (plugins off)
#   -np                  no prompt templates
#   -ns                  no skills
#   -nc                  no context files (AGENTS.md/CLAUDE.md)
#   -nt                  no tools (built-in + extension)
#   --offline            skip startup network ops
#   --no-session         ephemeral — no ~/.pi/agent/sessions pollution
#   --provider <model>   fast/cheap model
#   -p "<prompt>"        non-interactive: process + exit
#
# We do NOT use --provider; instead --model accepts "provider/id" directly so a
# single env var pins both. --model falls back through pi's default if unset.
cmd=(
  pi
  -n "${SMOKE_PI_PROBE_NAME}"
  -ne -np -ns -nc -nt
  --offline --no-session
  -p "just say only HI to me"
)
if [[ -n "${SMOKE_PI_MODEL}" ]]; then
  cmd+=(--model "${SMOKE_PI_MODEL}")
fi

echo "smoke-pi: running bare-minimum probe (timeout=${SMOKE_PI_TIMEOUT}s, model=${SMOKE_PI_MODEL:-<default>})"

output_file="$(mktemp -t smoke-pi.XXXXXX)"
trap 'rm -f "${output_file}"' EXIT

if ! timeout "${SMOKE_PI_TIMEOUT}s" "${cmd[@]}" >"${output_file}" 2>&1; then
  rc=$?
  echo "smoke-pi: FAIL — pi exited ${rc} or timed out after ${SMOKE_PI_TIMEOUT}s" >&2
  echo "----- pi output -----" >&2
  cat "${output_file}" >&2
  echo "---------------------" >&2
  exit "${rc}"
fi

# ── assert ───────────────────────────────────────────────────────────────────
# pi must have answered. We accept any case-insensitive "hi" token (models sometimes
# add punctuation/whitespace). A blank or wildly-off response = broken.
if ! rg -iq 'hi' "${output_file}"; then
  echo "smoke-pi: FAIL — pi did not answer 'HI'" >&2
  echo "----- pi output -----" >&2
  cat "${output_file}" >&2
  echo "---------------------" >&2
  exit 1
fi

echo "smoke-pi: OK — pi responded"
exit 0
