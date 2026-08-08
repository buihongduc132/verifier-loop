//! Escalation counter lifecycle (dynamic-pipeline D5; LD4, LD21).
//!
//! The `esca` counter tracks consecutive PL-D invocations that ended in
//! (Gate-pass ∧ Confirm-reject). When `esca_count >= esca_threshold` AND `esca_threshold > 0`
//! AND `m >= 2`, the goal flips to PL-E for subsequent invocations.
//!
//! ## Lifecycle (LD4)
//!
//! ```text
//! Gate pass + Confirm reject  →  esca_count++
//! Confirm pass                →  esca_count = 0
//! Mixed pass + Final reject   →  esca_count = 0   (one PL-E cycle consumed)
//! Mixed reject                →  esca_count = 0
//! ```
//!
//! ## Guards (LD21)
//!
//!   * `esca_threshold = 0` → escalation disabled (PL-D forever).
//!   * `m < 2` → esca frozen (PL-E degenerates to Confirm; no point escalating).
//!   * `escalation_depth >= esca_max_retries` → hard-fail "escalation exhaustion".
//!
//! Counters live in `state.json` (`esca_count`, `escalation_depth`) and persist across
//! RESUME so the escalation state survives process restarts.

use crate::store::Config;

/// Outcome of an invocation's terminal phase, used to update the esca counter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvocationOutcome {
    /// PL-D: Gate passed + Confirm passed → APPROVE. Reset esca.
    PlDApprove,
    /// PL-D: Gate rejected. No change to esca (Gate never reached Confirm).
    PlDGateReject,
    /// PL-D: Gate passed + Confirm rejected → increment esca.
    PlDConfirmReject,
    /// PL-E: Mixed rejected. Reset esca.
    PlEMixedReject,
    /// PL-E: Mixed passed + Final rejected. Reset esca.
    PlEFinalReject,
    /// PL-E: Mixed passed + Final passed → APPROVE. Reset esca + leave escalation_depth.
    PlEApprove,
}

/// State derived from `state.json` fields for the esca decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EscaState {
    pub esca_count: u32,
    pub escalation_depth: u32,
}

impl EscaState {
    pub fn new(esca_count: u32, escalation_depth: u32) -> Self {
        Self {
            esca_count,
            escalation_depth,
        }
    }
}

/// Decide whether the NEXT invocation should run PL-E.
///
/// PL-E activates when `esca_threshold > 0` AND `m >= 2` AND `esca_count >= esca_threshold`
/// AND the goal has not exhausted its PL-E retries.
pub fn should_run_escalation(cfg: &Config, state: EscaState) -> bool {
    if cfg.esca_threshold == 0 {
        return false;
    }
    if cfg.m < 2 {
        return false;
    }
    if state.esca_count < cfg.esca_threshold {
        return false;
    }
    if cfg.esca_max_retries > 0 && state.escalation_depth >= cfg.esca_max_retries {
        return false;
    }
    true
}

/// Apply an invocation outcome to the esca state, returning the new state.
///
/// LD4 rules:
///   * Gate-pass + Confirm-reject → esca_count++
///   * Confirm pass → esca_count = 0
///   * Mixed reject → esca_count = 0
///   * Mixed-pass + Final-reject → esca_count = 0 (escalation_depth unchanged; it counts
///     PL-E *cycles*, not reject events, so it's only incremented on a PL-E invocation
///     that DIDN'T approve)
///   * PL-E Approve → esca_count = 0
pub fn apply_outcome(cfg: &Config, state: EscaState, outcome: InvocationOutcome) -> EscaState {
    match outcome {
        InvocationOutcome::PlDConfirmReject => {
            let count = if cfg.m >= 2 && cfg.esca_threshold > 0 {
                state.esca_count.saturating_add(1)
            } else {
                state.esca_count // frozen when m < 2
            };
            EscaState {
                esca_count: count,
                escalation_depth: state.escalation_depth,
            }
        }
        // All non-Confirm-reject outcomes reset the streak.
        InvocationOutcome::PlDApprove
        | InvocationOutcome::PlDGateReject
        | InvocationOutcome::PlEMixedReject
        | InvocationOutcome::PlEFinalReject
        | InvocationOutcome::PlEApprove => {
            // A completed PL-E invocation that did NOT approve bumps escalation_depth
            // (counting PL-E cycles consumed). An approve or a PL-D outcome does not.
            let depth_bump = matches!(
                outcome,
                InvocationOutcome::PlEMixedReject | InvocationOutcome::PlEFinalReject
            );
            EscaState {
                esca_count: 0,
                escalation_depth: if depth_bump {
                    state.escalation_depth.saturating_add(1)
                } else {
                    state.escalation_depth
                },
            }
        }
    }
}

/// Whether the goal has exhausted its PL-E retries and should hard-fail.
pub fn escalation_exhausted(cfg: &Config, state: EscaState) -> bool {
    cfg.esca_max_retries > 0 && state.escalation_depth >= cfg.esca_max_retries
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(threshold: u32, max_retries: u32, m: u32) -> Config {
        Config {
            n: 2,
            m,
            confirm_count: 1,
            esca_threshold: threshold,
            esca_max_retries: max_retries,
            ..Config::default()
        }
    }

    #[test]
    fn confirm_reject_increments_esca() {
        let c = cfg(2, 3, 2);
        let s = EscaState::new(0, 0);
        let s2 = apply_outcome(&c, s, InvocationOutcome::PlDConfirmReject);
        assert_eq!(s2.esca_count, 1);
        let s3 = apply_outcome(&c, s2, InvocationOutcome::PlDConfirmReject);
        assert_eq!(s3.esca_count, 2, "two consecutive → threshold reached");
    }

    #[test]
    fn confirm_pass_resets_esca() {
        let c = cfg(2, 3, 2);
        let s = apply_outcome(&c, EscaState::new(1, 0), InvocationOutcome::PlDConfirmReject);
        let s = apply_outcome(&c, s, InvocationOutcome::PlDApprove);
        assert_eq!(s.esca_count, 0, "approve resets esca");
    }

    #[test]
    fn mixed_reject_resets_esca_and_bumps_depth() {
        let c = cfg(2, 3, 2);
        let s = apply_outcome(&c, EscaState::new(2, 0), InvocationOutcome::PlEMixedReject);
        assert_eq!(s.esca_count, 0);
        assert_eq!(s.escalation_depth, 1);
    }

    #[test]
    fn final_reject_resets_esca_and_bumps_depth() {
        let c = cfg(2, 3, 2);
        let s = apply_outcome(&c, EscaState::new(2, 0), InvocationOutcome::PlEFinalReject);
        assert_eq!(s.esca_count, 0);
        assert_eq!(s.escalation_depth, 1);
    }

    #[test]
    fn should_escalate_requires_threshold_m_and_count() {
        let c = cfg(2, 3, 2);
        assert!(!should_run_escalation(&c, EscaState::new(0, 0)));
        assert!(!should_run_escalation(&c, EscaState::new(1, 0)));
        assert!(should_run_escalation(&c, EscaState::new(2, 0)));
    }

    #[test]
    fn esca_threshold_zero_disables() {
        let c = cfg(0, 3, 2);
        assert!(!should_run_escalation(&c, EscaState::new(99, 0)));
    }

    #[test]
    fn m_lt_2_disables() {
        let c = cfg(2, 3, 1);
        assert!(!should_run_escalation(&c, EscaState::new(5, 0)));
    }

    #[test]
    fn exhaustion_hard_fails() {
        let c = cfg(2, 3, 2);
        let s = EscaState::new(5, 3); // depth == max
        assert!(escalation_exhausted(&c, s));
        assert!(!should_run_escalation(&c, s), "exhausted → no more PL-E");
    }

    #[test]
    fn esca_frozen_when_m_lt_2() {
        // LD21: when m < 2, esca is frozen (does not increment).
        let c = cfg(2, 3, 1);
        let s = apply_outcome(&c, EscaState::new(0, 0), InvocationOutcome::PlDConfirmReject);
        assert_eq!(s.esca_count, 0, "m=1 → esca frozen");
    }
}
