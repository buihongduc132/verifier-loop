#!/usr/bin/env bash
# parse-verdict.sh — parse a jewilo run transcript into a structured verdict.
#
# Usage: parse-verdict.sh <transcript-file>
#
# Emits one JSON object on stdout:
#   {
#     "verdict":          "NONE" | "APPROVE" | "REJECT",
#     "completion_hash":  null | "<8+ hex>",
#     "findings_count":   <number of D<n>+ defect markers>
#   }
#
# Two input shapes are handled:
#  1. jewilo --json output (authoritative when present). Single JSON line:
#       REJECT : {"ok":false,...,"status":"rejected","rejection":{"rejectNotes":[[vid,notes],...],"nullVerifiers":[...]}}
#       APPROVE: {"ok":true,...,"status":"approved","completion":{"hash":"<8+hex>",...}}
#  2. Plain-text verifier transcript (fallback):
#       VERDICT: APPROVE | REJECT
#       Completion hash: <hex>
# A "finding" is any `D<digits>` defect marker (D1 BLOCKER, D2 MAJOR, ...) or any
# bare severity token (BLOCKER / MAJOR / MINOR) emitted by a verifier. Always exits 0.
set -euo pipefail

if [ "$#" -lt 1 ] || [ -z "${1:-}" ]; then
  echo "usage: parse-verdict.sh <transcript-file>" >&2
  exit 2
fi

transcript="$1"

if [ ! -f "$transcript" ]; then
  echo "parse-verdict: transcript not found: $transcript" >&2
  exit 2
fi

verdict="NONE"
completion_hash="null"
findings_count=0

# Count defect markers (D<n>, BLOCKER, MAJOR, MINOR) in a body of text.
# Uses grep -i so case variations match; one match per line (a line with both
# "D1 (BLOCKER)" still counts as 1 because grep -c counts matching LINES not
# matches — which avoids double-counting a single finding that uses both forms).
count_findings() {
  grep -ciE 'D[0-9]+|\b(BLOCKER|MAJOR|MINOR)\b' "$1" 2>/dev/null || printf '0'
}

# --- jewilo --json output (authoritative when present) -------------------
jewilo_line="$(grep -E '"command":"(new|resume)".*"status":"(rejected|approved)"' "$transcript" 2>/dev/null | tail -n1 || true)"
if [ -n "$jewilo_line" ]; then
  status_val="$(printf '%s' "$jewilo_line" | jq -r '.status // empty' 2>/dev/null || true)"
  case "$status_val" in
    approved)
      verdict="APPROVE"
      h="$(printf '%s' "$jewilo_line" | jq -r '.completion.hash // empty' 2>/dev/null || true)"
      if [ -n "$h" ]; then completion_hash="\"$h\""; fi
      findings_count=0
      ;;
    rejected)
      notes_blob="$(printf '%s' "$jewilo_line" | jq -r '.rejection.rejectNotes[].[1] // empty' 2>/dev/null || true)"
      notes_count="$(printf '%s' "$jewilo_line" | jq -r '.rejection.rejectNotes | length' 2>/dev/null || printf '0')"
      case "${notes_count:-0}" in ''|*[!0-9]*) notes_count=0 ;; esac
      if [ "$notes_count" -gt 0 ]; then
        verdict="REJECT"
        fc_tmp="$(printf '%s' "$notes_blob" | count_findings /dev/stdin || printf '0')"
        case "$fc_tmp" in ''|*[!0-9]*) fc_tmp=0 ;; esac
        findings_count="$fc_tmp"
      else
        # rejected with zero rejectNotes = no verdict emitted (nullVerifiers / timeout)
        verdict="NONE"
        findings_count=0
      fi
      ;;
  esac
fi

# --- fallback: plain-text VERDICT: line (only when no jewilo JSON) --------
if [ "$verdict" = "NONE" ]; then
  verdict_line="$(grep -iE '^[[:space:]]*verdict:[[:space:]]*[A-Za-z]+' "$transcript" 2>/dev/null | head -n1 || true)"
  if [ -n "$verdict_line" ]; then
    token="$(printf '%s' "$verdict_line" \
             | sed -E 's/^[[:space:]]*[Vv][Ee][Rr][Dd][Ii][Cc][Tt]:[[:space:]]*//' \
             | tr '[:lower:]' '[:upper:]' \
             | sed -E 's/[^A-Z].*$//')"
    case "$token" in
      APPROVE) verdict="APPROVE" ;;
      REJECT)  verdict="REJECT" ;;
      *)       verdict="NONE" ;;
    esac
  fi

  # completion hash from plain text (only if not already set by jewilo JSON)
  if [ "$completion_hash" = "null" ]; then
    hash_line="$(grep -iE 'completion[[:space:]]+hash:[[:space:]]*[0-9a-fA-F]{8,}' "$transcript" 2>/dev/null | head -n1 || true)"
    if [ -n "$hash_line" ]; then
      hex="$(printf '%s' "$hash_line" | grep -oiE '[0-9a-fA-F]{8,}' | head -n1 || true)"
      if [ -n "$hex" ]; then
        completion_hash="\"$(printf '%s' "$hex" | tr '[:upper:]' '[:lower:]')\""
      fi
    fi
  fi

  # findings: count ^D<digits> defect lines at line start (plain-text mode)
  fc="$(grep -cE '^[[:space:]]*D[0-9]+' "$transcript" 2>/dev/null || printf '0')"
  case "$fc" in ''|*[!0-9]*) fc=0 ;; esac
  findings_count="$fc"
fi

# --- emit JSON via jq so output is always valid --------------------------
jq -nc \
  --arg v "$verdict" \
  --argjson h "$completion_hash" \
  --argjson c "$findings_count" \
  '{verdict:$v, completion_hash:$h, findings_count:$c}'
