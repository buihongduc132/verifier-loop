//! `verifier-verdict` (jewije) logic (tasks.md §7, verdict-registration spec).
//!
//! `approve` / `reject --notes "…"`; identity resolved from VERIFIER_LOOP_* env (D2);
//! atomic write, first-write-wins (D4); reject-without-notes refused; null never->APPROVE (D9).

// TODO §7: verdict CLI logic (RED then GREEN, separate fresh teammates).
