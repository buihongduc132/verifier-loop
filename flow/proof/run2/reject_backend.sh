#!/bin/sh
cat <<'ACP'
{"type":"session","id":"selfverify-sid"}
{"type":"agent_end","messages":[{"role":"assistant","content":[{"type":"text","text":"needs work"}]}],"willRetry":false}
ACP
exec verifier-verdict reject --notes "round 1: missing evidence"
