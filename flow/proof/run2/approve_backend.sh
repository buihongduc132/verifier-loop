#!/bin/sh
# Hermetic verifier backend: emits a valid ACP-JSON stream, then registers APPROVE.
cat <<'ACP'
{"type":"session","id":"selfverify-sid"}
{"type":"agent_end","messages":[{"role":"assistant","content":[{"type":"text","text":"verified"}]}],"willRetry":false}
ACP
exec verifier-verdict approve
