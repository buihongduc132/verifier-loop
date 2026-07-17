#!/usr/bin/env bash
# parse-verdict.sh — parse a jewilo run transcript into a structured verdict.
#
# Usage: parse-verdict.sh <transcript-file>
#
# Emits one JSON object on stdout:
#   {
#     "verdict":          "NONE" | "APPROVE" | "REJECT",
#     "completion_hash":  null | "<8+ hex>",
#     "findings_count":   <number of ^D<n>+ defect lines>
#   }
#
# Matching is case-insensitive on the labels the verifier prompt emits:
#   VERDICT: APPROVE | REJECT
#   Completion hash: <hex>
# A "finding" is any body line beginning with `D<digits>` (D1 BLOCKER, D2 MAJOR, ...).
# Always exits 0 — empty/missing verdicts are a valid NONE state, not errors.
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

# --- verdict line (case-insensitive) ---------------------------------------
# Look for a line whose first non-empty token is `verdict:` followed by
# APPROVE or REJECT. Take the first such line we see.
verdict="NONE"
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

# --- completion hash (only meaningful for APPROVE, but search regardless) ---
completion_hash="null"
hash_line="$(grep -iE 'completion[[:space:]]+hash:[[:space:]]*[0-9a-fA-F]{8,}' "$transcript" 2>/dev/null | head -n1 || true)"
if [ -n "$hash_line" ]; then
  hex="$(printf '%s' "$hash_line" | grep -oiE '[0-9a-fA-F]{8,}' | head -n1 || true)"
  if [ -n "$hex" ]; then
    # lower-case the hex for canonical form
    completion_hash="\"$(printf '%s' "$hex" | tr '[:upper:]' '[:lower:]')\""
  fi
fi

# --- findings: count ^D<digits> defect lines -------------------------------
# Anchor at line start; tolerate leading whitespace.
findings_count="$(grep -cE '^[[:space:]]*D[0-9]+' "$transcript" 2>/dev/null || printf '0')"
# grep -c always yields a number, but guard against the empty-file case where
# some greps exit non-zero.
case "$findings_count" in
  ''|*[!0-9]*) findings_count=0 ;;
esac

# --- emit JSON via jq so output is always valid ----------------------------
jq -nc \
  --arg v "$verdict" \
  --argjson h "$completion_hash" \
  --argjson c "$findings_count" \
  '{verdict:$v, completion_hash:$h, findings_count:$c}'
