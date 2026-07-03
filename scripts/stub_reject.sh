#!/usr/bin/env sh
# Stub backend: emits ACP JSON stream, then REJECTS (for RESUME transcript proof).
SID="stub-reject-$(date +%s)-$$"
printf '{"type":"session","version":3,"id":"%s","cwd":"%s"}\n' "$SID" "$PWD"
printf '{"type":"agent_start"}\n'
printf '{"type":"turn_start"}\n'
printf '{"type":"message_start","message":{"role":"user","content":[{"type":"text","text":"verify"}]}}\n'
printf '{"type":"message_end","message":{"role":"user","content":[{"type":"text","text":"verify"}]}}\n'
printf '{"type":"message_start","message":{"role":"assistant","content":[{"type":"text","text":"Rejecting: hypothetical issue found."}]}}\n'
printf '{"type":"message_end","message":{"role":"assistant","content":[{"type":"text","text":"Rejecting: hypothetical issue found."}]}}\n'
printf '{"type":"turn_end"}\n'
printf '{"type":"agent_end","messages":[{"role":"user","content":[{"type":"text","text":"verify"}]},{"role":"assistant","content":[{"type":"text","text":"Rejecting: hypothetical issue found."}]}],"willRetry":false}\n'
verifier-verdict reject --notes "Round 1 stub rejection: coverage on bin/verifier_verdict.rs was below gate before remediation. Fixed in commit 4ed3680."
