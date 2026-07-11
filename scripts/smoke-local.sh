#!/bin/bash
# Local smoke test for jewilo/jewije release binaries.
# Exercises the three new capabilities from fix-prompt-bloat-and-compaction-recovery:
#   (1) Prompt bloat: rendered prompt stays under budget on a repo with many tracked files.
#   (2) Verdict enforcement: a stub that "forgets" to register a verdict on the first
#       invocation gets nudged and the nudge fires (orchestration check) AND harvests a
#       SIGNED APPROVE via the real verifier-verdict (secret re-injected from the
#       persisted verifier-secret.hex, exactly as the orchestrator does on resume).
#   (3) Compaction recovery: a stub that compacts+exits gets one recovery resume that
#       harvests a SIGNED APPROVE via the real verifier-verdict.
#
# Uses a stub backend (no real pi). The release binaries are at:
#   target/release/verifier-loop   (jewilo)
#   target/release/verifier-verdict (jewije)
#
# NOTE on signing: the initial spawn injects VERIFIER_LOOP_VERIFIER_SECRET (minted once
# and PERSISTED to verifier-secret.hex at spawn time). Nudge/recovery resumes ARE NEW
# processes and cannot inherit that env, so they re-inject the SAME secret by reading it
# back from verifier-secret.hex — mirroring the orchestrator's own spawn_nudge_child
# path. Every harvested verdict is therefore signed and verifies against the pinned
# pubkey, so the full NEW→APPROVE→hash consensus runs end-to-end in all three tests.
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
    local i
    for i in $(seq 1 200); do
        printf 'content %d\n' "$i" > "file_$(printf '%03d' "$i").txt"
    done
    git add file_000.txt file_050.txt file_150.txt 2>/dev/null || true
    git add . 2>/dev/null || git add -A
    git commit -q -m "seed: 200 tracked files"
    # Modify 3 known files so the changed-files set is small (D1 scoping test).
    printf 'changed\n' > "file_001.txt"
    printf 'changed\n' > "file_050.txt"
    printf 'changed\n' > "file_150.txt"
}

# Resolve the persisted per-verifier signing secret from the slot dir and re-inject it as
# VERIFIER_LOOP_VERIFIER_SECRET, exactly as the orchestrator's spawn_nudge_child does on a
# resume. This lets the real verifier-verdict sign a verdict that verifies against the
# pinned pubkey. Args: <home_dir> <goal_id> <round> <verifier_id>.
reinject_secret() {
    local home="$1" goal="$2" round="$3" vid="$4"
    local secret_file="$home/goals/$goal/rounds/$round/$vid/verifier-secret.hex"
    if [ -f "$secret_file" ]; then
        export VERIFIER_LOOP_VERIFIER_SECRET="$(cat "$secret_file")"
    fi
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
        # (c) The fileEditTimes block must contain EXACTLY the 3 changed files
        # (file_001.txt, file_050.txt, file_150.txt) and nothing else. -le 3 would pass
        # even if scoping dropped to 0/1/2 entries; -eq 3 + path match pins it.
        # The template renders the block as:
        #   File edit times:
        #   ```
        #   <content>
        #   ```
        # so we capture lines between the heading's opening fence and its closing fence.
        FET_BLOCK=$(awk '
            /^File edit times:/ {in_section=1; next}
            in_section && /^```/ {fence++}
            in_section && /^```/ && fence==1 {capture=1; next}
            in_section && /^```/ && fence==2 {capture=0; in_section=0; next}
            capture {print}
        ' "$PROMPT_FILE")
        FET_LINES=$(printf '%s\n' "$FET_BLOCK" | grep -c ':' || true)
        if [ "$FET_LINES" -eq 3 ]; then
            ok "fileEditTimes scoped to exactly 3 changed files"
        else
            no "fileEditTimes has $FET_LINES entries (expected exactly 3; scoping broken)"
        fi
        # Verify the 3 specific paths match the modified files.
        PATHS_OK=1
        for expect in file_001.txt file_050.txt file_150.txt; do
            if ! printf '%s\n' "$FET_BLOCK" | grep -q "^$expect:"; then
                PATHS_OK=0
                no "fileEditTimes missing expected changed file '$expect'"
            fi
        done
        [ "$PATHS_OK" -eq 1 ] && ok "fileEditTimes paths match the 3 modified files"
        # (d) D7: extract the LAST fenced ```bash block in the prompt and assert BOTH
        # `verifier-verdict approve` and `verifier-verdict reject` appear within it (not
        # just somewhere in the prompt). The last fenced bash block is the verdict
        # command — appearing earlier in prose is not sufficient.
        LAST_BASH_BLOCK=$(awk '
            /^```bash/{capture=1; buf=""; next}
            /^```/{if(capture){last=buf; capture=0}; next}
            capture{buf=buf $0 "\n"}
            END{print last}
        ' "$PROMPT_FILE")
        if [ -n "$LAST_BASH_BLOCK" ] \
            && printf '%s' "$LAST_BASH_BLOCK" | grep -q 'verifier-verdict approve' \
            && printf '%s' "$LAST_BASH_BLOCK" | grep -q 'verifier-verdict reject'; then
            ok "last fenced bash block contains both verifier-verdict approve + reject (D7)"
        else
            no "last fenced bash block missing verifier-verdict approve/reject (D7); block was:
$LAST_BASH_BLOCK"
        fi
    else
        no "initial-prompt.txt missing at $PROMPT_FILE"
    fi
else
    no "no goal dir created"
fi
unset VERIFIER_LOOP_HOME VERIFIER_LOOP_BACKEND_CMD

# ===========================================================================
# Test 2: Verdict enforcement — nudge fires after a no-verdict exit AND harvests a
#         SIGNED APPROVE via the real verifier-verdict (secret re-injected from
#         verifier-secret.hex, exactly as the orchestrator does on resume).
# ===========================================================================
echo "--- Test 2: verdict enforcement (nudge after no-verdict exit, signed harvest) ---"
REPO2="$SMOKE_DIR/repo2"
HOME2="$SMOKE_DIR/home2"
mkdir -p "$HOME2" "$SMOKE_DIR/cap2"
setup_repo "$REPO2"
cat > "$HOME2/config.json" <<'CFG'
{ "n": 1, "m": 1, "maxTurn": 3, "backend": "stub", "gitDiffMaxChars": 1000, "verifierTimeoutSec": 15 }
CFG

# Nudge stub: forgets verdict on invocation 1, registers a SIGNED verdict via the REAL
# verifier-verdict on invocation 2+. The orchestrator re-injects the secret into the
# resume child env (from verifier-secret.hex), so $VERIFIER_LOOP_VERIFIER_SECRET is set
# here automatically.
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
  "$VV" approve --notes "nudge-harvested signed verdict" 2>"$SMOKE_DIR/cap2/v1.verdict-stderr.log" || echo "verdict-rc=\$?" > "$SMOKE_DIR/cap2/v1.verdict-rc"
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
    # The harvested verdict MUST be a signed APPROVE (full consensus path, not an
    # unsigned placeholder). Surface verdict-CLI stderr if it failed.
    if [ -f "$SMOKE_DIR/cap2/v1.verdict-rc" ]; then
        cat "$SMOKE_DIR/cap2/v1.verdict-stderr.log" 2>/dev/null
        no "verifier-verdict failed during nudge resume (see stderr above)"
    else
        VJ="$HOME2/goals/$GOAL_ID/rounds/1/v1/verdict.json"
        VSTATUS=$(python3 -c "import json;print(json.load(open('$VJ')).get('status'))" 2>/dev/null || echo "?")
        VSIG=$(python3 -c "import json;print(json.load(open('$VJ')).get('signature') or '')" 2>/dev/null || echo "")
        if [ "$VSTATUS" = "APPROVE" ] && [ -n "$VSIG" ]; then
            ok "nudge harvested a SIGNED APPROVE (signature present)"
        else
            no "nudge-harvested verdict not a signed APPROVE (status=$VSTATUS, sig_len=${#VSIG})"
        fi
    fi
fi
unset VERIFIER_LOOP_HOME VERIFIER_LOOP_BACKEND_CMD VERIFIER_LOOP_VERIFIER_SECRET

# ===========================================================================
# Test 3: Compaction recovery — compaction+exit triggers one recovery resume that
#         harvests a SIGNED APPROVE via the real verifier-verdict.
# ===========================================================================
echo "--- Test 3: compaction recovery (compact+exit → recovery resume, signed harvest) ---"
REPO3="$SMOKE_DIR/repo3"
HOME3="$SMOKE_DIR/home3"
mkdir -p "$HOME3" "$SMOKE_DIR/cap3"
setup_repo "$REPO3"
cat > "$HOME3/config.json" <<'CFG'
{ "n": 1, "m": 1, "maxTurn": 3, "backend": "stub", "gitDiffMaxChars": 1000, "verifierTimeoutSec": 15 }
CFG

# Compact stub: emits compaction then exits (no agent_end) on invocation 1;
# registers a SIGNED verdict via the REAL verifier-verdict on invocation 2 (the recovery
# resume). The orchestrator re-injects the secret from verifier-secret.hex.
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
"$VV" approve --notes "recovery-harvested signed verdict" 2>"$SMOKE_DIR/cap3/v1.verdict-stderr.log" || echo "verdict-rc=\$?" > "$SMOKE_DIR/cap3/v1.verdict-rc"
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
    # The recovered verdict MUST be a signed APPROVE.
    if [ -f "$SMOKE_DIR/cap3/v1.verdict-rc" ]; then
        cat "$SMOKE_DIR/cap3/v1.verdict-stderr.log" 2>/dev/null
        no "verifier-verdict failed during recovery resume (see stderr above)"
    else
        VJ="$HOME3/goals/$GOAL_ID/rounds/1/v1/verdict.json"
        VSTATUS=$(python3 -c "import json;print(json.load(open('$VJ')).get('status'))" 2>/dev/null || echo "?")
        VSIG=$(python3 -c "import json;print(json.load(open('$VJ')).get('signature') or '')" 2>/dev/null || echo "")
        if [ "$VSTATUS" = "APPROVE" ] && [ -n "$VSIG" ]; then
            ok "recovery harvested a SIGNED APPROVE (signature present)"
        else
            no "recovery-harvested verdict not a signed APPROVE (status=$VSTATUS, sig_len=${#VSIG})"
        fi
    fi
fi
unset VERIFIER_LOOP_HOME VERIFIER_LOOP_BACKEND_CMD VERIFIER_LOOP_VERIFIER_SECRET

# ===========================================================================
# Summary
# ===========================================================================
echo ""
echo "=== Smoke summary: $PASS passed, $FAIL failed ==="
if [ "$FAIL" -gt 0 ]; then
    exit 1
fi
exit 0
