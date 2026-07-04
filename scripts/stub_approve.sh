#!/usr/bin/env sh
# Stub backend: emits a minimal valid ACP JSON stream, then runs verifier-verdict approve.
# Simulates a verifier that inspects the work and approves.
SID="stub-$(date +%s)-$$"
printf '{"type":"session","version":3,"id":"%s","cwd":"%s"}\n' "$SID" "$PWD"
printf '{"type":"agent_start"}\n'
printf '{"type":"turn_start"}\n'
printf '{"type":"message_start","message":{"role":"user","content":[{"type":"text","text":"verify"}]}}\n'
printf '{"type":"message_end","message":{"role":"user","content":[{"type":"text","text":"verify"}]}}\n'
printf '{"type":"message_start","message":{"role":"assistant","content":[{"type":"text","text":"Work verified: cargo build --release clean, cargo test 151 passed, coverage 93pct. Approving."}]}}\n'
printf '{"type":"message_end","message":{"role":"assistant","content":[{"type":"text","text":"Work verified: cargo build --release clean, cargo test 151 passed, coverage 93pct. Approving."}]}}\n'
printf '{"type":"turn_end"}\n'
printf '{"type":"agent_end","messages":[{"role":"user","content":[{"type":"text","text":"verify"}]},{"role":"assistant","content":[{"type":"text","text":"Work verified: cargo build --release clean, cargo test 151 passed, coverage 93pct. Approving."}]}],"willRetry":false}\n'
# Register the verdict — forward the per-verifier signing secret (D3) so the
# signed-verdict path is taken. VERIFIER_LOOP_HOME/GOAL_ID/VERIFIER_ID/ROUND are
# already in our env (injected by spawn); the child inherits them automatically.
# Prefer the orchestrator-resolved sibling binary so a stale/global
# `verifier-verdict` on PATH cannot produce an unsigned verdict.
export VERIFIER_LOOP_VERIFIER_SECRET
VVERDICT="${VERIFIER_LOOP_VERDICT_BIN:-verifier-verdict}"
"$VVERDICT" approve
