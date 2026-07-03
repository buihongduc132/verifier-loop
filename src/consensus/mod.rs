//! Consensus + completion hash (tasks.md §8, consensus-check + completion-proof specs).
//!
//! n/m APPROVE counter after gather; on pass compute
//! `"vl:" + first40hex(SHA256(salt + goalId + goalSignature + round + JSON(matchingVerdicts
//! sorted by verifierId) + matchedAtISO))` (D6); write completion.json; on fail surface notes.
//! Each input guards a distinct tamper vector (goalText edit / verdict edit -> hash mismatch).

// TODO §8: counter + hash + completion.json (RED then GREEN, separate fresh teammates).
