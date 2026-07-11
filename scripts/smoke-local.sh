#!/bin/bash
# Local smoke test for jewilo/jewije release binaries.
# Exercises the three new capabilities from fix-prompt-bloat-and-compaction-recovery:
#   (1) Prompt bloat: rendered prompt stays under budget on a repo with many tracked files.
#   (2) Verdict enforcement: a stub that "forgets" to register a verdict on the first
#       invocation gets nudged and the nudge fires (orchestration check).
#   (3) Compaction recovery: a stub that compacts+exits gets one recovery resume.
#
# Uses a stub backend (no real pi). The release binaries are at:
#   target/release/verifier-loop   (jewilo)
#   target/release/verifier-verdict (jewije)
#
# NOTE on signing: the initial spawn injects VERIFIER_LOOP_VERIFIER_SECRET so the real
# verifier-verdict can sign. Nudge/recovery resumes CANNOT re-inject the secret (it is
# minted once and never persisted; mint_and_pin_pubkey returns AlreadyPinned on resume).
# So test 1 (happy path) runs the full consensus via the real verifier-verdict, while
# tests 2/3 verify the orchestration fired (invocation count + meta.json fields) without
# requiring the full consensus to pass. This signing gap is tracked separately.
#
# Exit non-zero on ANY failure.

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
VL="$REPO_ROOT/target/release/verifier-loop"
VV="$REPO_ROOT/target/release/verifier-verdict"
SMOKE_DIR=$(mktemp -d)
trap 'rm -rf "$SMOKE_DIR"' EXIT

PASS=0
FAIL=0

ok() { echo "  ✓ $1"; PASS=$((PASS + 1)); }
no() { echo "  ✗ $1"; FAIL=$((FAIL + 1)); }

echo "=== Local smoke test: jewilo/jewije release binaries ==="
echo "  repo: $REPO_ROOT"
echo "  smoke dir: $SMOKE_DIR"
echo ""

# ---------------------------------------------------------------------------
# Shared setup: a git repo with MANY tracked files (to exercise prompt bloat scoping).
# ---------------------------------------------------------------------------
setup_repo() {
    local repo="$1"
    mkdir -p "$repo"
    cd "$repo"
    git init -q
    git config user.email "smoke@test.t"
    git config user.name "smoke"
    # Create 200 tracked files (simulates a real repo — would bloat fileEditTimes under
    # the old `git ls-files` enumeration).
    for i in $(seq 1 200); do
        printf 'content %d\n' "$i" > "file_$(printf '%03d' "$i").txt"
    done
    git add -A
    git commit -q -m "seed: 200 tracked files"
    # Modify 3 files so the changed-files set is small (D1 scoping test).
    printf 'changed\n' > "file_001.txt"
    printf 'changed\n' > "file_050.txt"
    printf 'changed\n' > "file_150.txt"
}

# A snippet that writes a verdict directly into the slot (bypasses verifier-verdict
# signing — used by nudge/recovery tests that only check orchestration, not consensus).
verdict_snippet() {
    cat <<'SNIP'
SLOT="$VERIFIER_LOOP_HOME/goals/$VERIFIER_LOOP_GOAL_ID/rounds/$VERIFIER_LOOP_ROUND/$VERIFIER_LOOP_VERIFIER_ID"
mkdir -p "$SLOT"
printf '%s\n' '{"status":"APPROVE","registeredAt":"2026-07-11T00:00:00Z"}' > "$SLOT/verdict.json"
SNIP
}

# ===========================================================================
# Test 1: Prompt bloat + happy-path consensus — rendered prompt is bounded,
#         and the full NEW→APPROVE→hash flow works with the real verifier-verdict.
# ===========================================================================
echo "--- Test 1: prompt bloat + happy-path consensus (200 tracked, 3 changed) ---"
REPO1="$SMOKE_DIR/repo1"
HOME1="$SMOKE_DIR/home1"
mkdir -p "$HOME1"
setup_repo "$REPO1"
cat > "$HOME1/config.json" <<'CFG'
{ "n": 1, "m": 1, "maxTurn": 3, "backend": "stub", "gitDiffMaxChars": 1000, "verifierTimeoutSec": 15, "promptBudgetBytes": 50000 }
CFG

# Approve stub: uses the REAL verifier-verdict (signs on initial spawn → consensus passes).
APPROVE_STUB="$SMOKE_DIR/approve.sh"
{
    echo '#!/bin/sh'
    echo 'cat <<ACP'
    echo '{"type":"session","id":"smoke-approve-sid"}'
    echo '{"type":"agent_end","messages":[{"role":"assistant","content":[{"type":"text","text":"approved"}]}],"willRetry":false}'
    echo 'ACP'
    echo "\"$VV\" approve --notes \"smoke approve\""
} > "$APPROVE_STUB"
chmod +x "$APPROVE_STUB"

export VERIFIER_LOOP_HOME="$HOME1"
export VERIFIER_LOOP_BACKEND_CMD="$APPROVE_STUB"
cd "$REPO1"

OUT=$("$VL" NEW "smoke test: prompt bloat" 2>"$SMOKE_DIR/stderr1.txt") || {
    echo "    jewilo NEW failed:"; cat "$SMOKE_DIR/stderr1.txt"; no "jewilo NEW exit code"
}
if echo "$OUT" | grep -qE '[0-9]{6}-[0-9a-f]{8}'; then
    ok "jewilo NEW produced a completion hash (full consensus passed)"
else
    no "jewilo NEW did not produce a completion hash"
fi
GOAL_ID=$(ls "$HOME1/goals" 2>/dev/null | head -1 || echo "")
if [ -n "$GOAL_ID" ]; then
    PROMPT_FILE="$HOME1/goals/$GOAL_ID/rounds/1/v1/initial-prompt.txt"
    if [ -f "$PROMPT_FILE" ]; then
        PROMPT_SIZE=$(wc -c < "$PROMPT_FILE")
        if [ "$PROMPT_SIZE" -lt 50000 ]; then
            ok "rendered prompt $PROMPT_SIZE bytes < 50000 budget"
        else
            no "rendered prompt $PROMPT_SIZE bytes >= 50000 budget (bloat not fixed)"
        fi
        # The fileEditTimes block must contain ONLY the 3 changed files, not all 200.
        FET_LINES=$(awk '/^File edit times:/{f=1;next} /^```/{if(f){f=0}} f{print}' "$PROMPT_FILE" | grep -c ':' || true)
        if [ "$FET_LINES" -le 3 ]; then
            ok "fileEditTimes scoped to $FET_LINES changed files (<=3 expected)"
        else
            no "fileEditTimes has $FET_LINES entries (expected <=3; scoping not applied)"
        fi
        # The default template must end with the explicit verdict command (D7).
        if grep -q '```bash' "$PROMPT_FILE" && grep -q 'verifier-verdict approve' "$PROMPT_FILE" && grep -q 'verifier-verdict reject' "$PROMPT_FILE"; then
            ok "default template ends with explicit fenced verdict command (D7)"
        else
            no "default template missing explicit fenced verdict command (D7)"
        fi
    else
        no "initial-prompt.txt missing at $PROMPT_FILE"
    fi
else
    no "no goal dir created"
fi
unset VERIFIER_LOOP_HOME VERIFIER_LOOP_BACKEND_CMD

# ===========================================================================
# Test 2: Verdict enforcement — nudge fires after a no-verdict exit (orchestration check).
# ===========================================================================
echo "--- Test 2: verdict enforcement (nudge after no-verdict exit) ---"
REPO2="$SMOKE_DIR/repo2"
HOME2="$SMOKE_DIR/home2"
mkdir -p "$HOME2" "$SMOKE_DIR/cap2"
setup_repo "$REPO2"
cat > "$HOME2/config.json" <<'CFG'
{ "n": 1, "m": 1, "maxTurn": 3, "backend": "stub", "gitDiffMaxChars": 1000, "verifierTimeoutSec": 15 }
CFG

# Nudge stub: forgets verdict on invocation 1, writes (unsigned) on invocation 2+.
# We only check orchestration here (invocation count + meta.nudgeAttempts), not consensus,
# because the nudge resume cannot re-inject the signing secret.
NUDGE_STUB="$SMOKE_DIR/nudge.sh"
cat > "$NUDGE_STUB" <<SCRIPT
#!/bin/sh
COUNT_FILE="$SMOKE_DIR/cap2/v1.count"
COUNT=\$(cat "\$COUNT_FILE" 2>/dev/null || echo 0)
COUNT=\$((COUNT + 1))
echo "\$COUNT" > "\$COUNT_FILE"

cat <<'ACP'
{"type":"session","id":"smoke-nudge-sid"}
{"type":"agent_end","messages":[{"role":"assistant","content":[{"type":"text","text":"done"}]}],"willRetry":false}
ACP

if [ "\$COUNT" -ge 2 ]; then
$(verdict_snippet)
fi
SCRIPT
chmod +x "$NUDGE_STUB"

export VERIFIER_LOOP_HOME="$HOME2"
export VERIFIER_LOOP_BACKEND_CMD="$NUDGE_STUB"
cd "$REPO2"

"$VL" NEW "smoke test: verdict enforcement" 2>"$SMOKE_DIR/stderr2.txt" || true
GOAL_ID=$(ls "$HOME2/goals" 2>/dev/null | head -1 || echo "")
if [ -z "$GOAL_ID" ]; then
    no "no goal dir created for verdict enforcement test"
else
    COUNT=$(cat "$SMOKE_DIR/cap2/v1.count" 2>/dev/null || echo 0)
    if [ "$COUNT" -ge 2 ]; then
        ok "stub invoked $COUNT times (>=2: initial + nudge resume fired)"
    else
        no "stub invoked only $COUNT time (nudge did not fire)"
    fi
    META="$HOME2/goals/$GOAL_ID/rounds/1/v1/meta.json"
    NUDGE_ATTEMPTS=$(python3 -c "import json;print(json.load(open('$META')).get('nudgeAttempts',0))" 2>/dev/null || echo "?")
    if [ "$NUDGE_ATTEMPTS" -ge 1 ]; then
        ok "meta.json nudgeAttempts=$NUDGE_ATTEMPTS (>=1)"
    else
        no "meta.json nudgeAttempts=$NUDGE_ATTEMPTS (expected >=1)"
    fi
fi
unset VERIFIER_LOOP_HOME VERIFIER_LOOP_BACKEND_CMD

# ===========================================================================
# Test 3: Compaction recovery — compaction+exit triggers one recovery resume.
# ===========================================================================
echo "--- Test 3: compaction recovery (compact+exit → recovery resume) ---"
REPO3="$SMOKE_DIR/repo3"
HOME3="$SMOKE_DIR/home3"
mkdir -p "$HOME3" "$SMOKE_DIR/cap3"
setup_repo "$REPO3"
cat > "$HOME3/config.json" <<'CFG'
{ "n": 1, "m": 1, "maxTurn": 3, "backend": "stub", "gitDiffMaxChars": 1000, "verifierTimeoutSec": 15 }
CFG

# Compact stub: emits compaction then exits (no agent_end) on invocation 1;
# recovers on invocation 2 (the recovery resume).
COMPACT_STUB="$SMOKE_DIR/compact.sh"
cat > "$COMPACT_STUB" <<SCRIPT
#!/bin/sh
COUNT_FILE="$SMOKE_DIR/cap3/v1.count"
COUNT=\$(cat "\$COUNT_FILE" 2>/dev/null || echo 0)
COUNT=\$((COUNT + 1))
echo "\$COUNT" > "\$COUNT_FILE"

if [ "\$COUNT" -eq 1 ]; then
  cat <<'ACP'
{"type":"session","id":"smoke-compact-sid"}
{"type":"compaction","tokensBefore":255106}
ACP
  exit 0
fi

cat <<'ACP'
{"type":"session","id":"smoke-compact-sid"}
{"type":"agent_end","messages":[{"role":"assistant","content":[{"type":"text","text":"recovered"}]}],"willRetry":false}
ACP
$(verdict_snippet)
SCRIPT
chmod +x "$COMPACT_STUB"

export VERIFIER_LOOP_HOME="$HOME3"
export VERIFIER_LOOP_BACKEND_CMD="$COMPACT_STUB"
cd "$REPO3"

"$VL" NEW "smoke test: compaction recovery" 2>"$SMOKE_DIR/stderr3.txt" || true
GOAL_ID=$(ls "$HOME3/goals" 2>/dev/null | head -1 || echo "")
if [ -z "$GOAL_ID" ]; then
    no "no goal dir created for compaction recovery test"
else
    COUNT=$(cat "$SMOKE_DIR/cap3/v1.count" 2>/dev/null || echo 0)
    if [ "$COUNT" -eq 2 ]; then
        ok "stub invoked exactly $COUNT times (initial + 1 recovery)"
    else
        no "stub invoked $COUNT times (expected exactly 2)"
    fi
    META="$HOME3/goals/$GOAL_ID/rounds/1/v1/meta.json"
    COMPACT=$(python3 -c "import json;print(json.load(open('$META')).get('compactionObserved',False))" 2>/dev/null || echo "?")
    RECOVERY=$(python3 -c "import json;print(json.load(open('$META')).get('recoveryAttempts',0))" 2>/dev/null || echo "?")
    if [ "$COMPACT" = "True" ]; then
        ok "meta.json compactionObserved=true"
    else
        no "meta.json compactionObserved=$COMPACT (expected True)"
    fi
    if [ "$RECOVERY" -eq 1 ]; then
        ok "meta.json recoveryAttempts=1 (exactly one recovery)"
    else
        no "meta.json recoveryAttempts=$RECOVERY (expected 1)"
    fi
fi
unset VERIFIER_LOOP_HOME VERIFIER_LOOP_BACKEND_CMD

# ===========================================================================
# Summary
# ===========================================================================
echo ""
echo "=== Smoke summary: $PASS passed, $FAIL failed ==="
if [ "$FAIL" -gt 0 ]; then
    exit 1
fi
exit 0
