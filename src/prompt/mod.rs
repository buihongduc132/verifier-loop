//! Verifier prompt rendering (tasks.md §9, verifier-prompt spec).
//!
//! Blind + frozen-artifact: V* sees identity, goalText, context, (resume) fix/prev-notes, and a
//! frozen snapshot (cwd, `git status --porcelain`, file edit times, `git diff` truncated to
//! gitDiffMaxChars). V* does NOT see round number, other verdicts, n/m, or the hash (D10).
//! Variables: {{goalId}} {{verifierId}} {{round}} {{goalText}} {{context}} {{fixNotes}}
//! {{prevNotes}} {{cwd}} {{gitStatus}} {{fileEditTimes}} {{gitDiff}} {{gitDiffMaxChars}}
//! {{process.env.*}}. Null template -> baked-in verifier-policy default.

// TODO §9: template engine + snapshot capture (RED then GREEN, separate fresh teammates).
