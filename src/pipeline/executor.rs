//! Pipeline executor (dynamic-pipeline D6; LD5, LD13, LD24).
//!
//! Runs a `Vec<Phase>` sequentially, short-circuiting on reject. Each phase:
//!   1. Resolves the adapter (dump for Gate, smart for Confirm)
//!   2. Spawns verifiers via the existing spawn primitives
//!   3. Gathers verdicts
//!   4. Evaluates consensus with the phase's threshold
//!   5. Short-circuits on reject (LD13: sub-phase REJECT = hard reject for the invocation)
//!
//! The executor is phase-shape-agnostic: it runs whatever `Vec<Phase>` it receives
//! (PL-D or PL-E). Adding a future phase type changes a pipeline constructor, not the
//! executor (LD11 rot-proof).
//!
//! ## Integration gaps (documented per goal custom prompt)
//!
//! **LD20 (per-slot adapters for Mixed):** The existing `spawn_round` takes a single
//! adapter for all m slots. The Mixed phase needs both dump AND smart adapters within
//! one phase. The pragmatic first cut: for Mixed, spawn in two batches (dump batch then
//! smart batch) and merge. This violates LD20's "one spawn = one gather" contract but
//! is a documented gap that will be resolved when the spawn orchestrator is extended to
//! carry `Vec<Adapter>` per slot.
//!
//! **Verifier-id scheme:** The existing `spawn_round` uses legacy `v{i+1}` ids. The
//! dynamic pipeline uses d/s scheme (LD16, LD26). The executor currently uses the legacy
//! scheme for compatibility with the existing spawn primitives. The d/s scheme is
//! implemented in `spawn::ids` but not yet wired into the spawn orchestrator. This is
//! a documented integration gap.

use crate::pipeline::{Phase, PhaseRole};
use crate::store::Config;

/// Result of running a single phase.
#[derive(Debug, Clone)]
pub struct PhaseResult {
    pub phase_id: String,
    pub role: PhaseRole,
    pub passed: bool,
    pub approve_count: u32,
    pub threshold: u32,
    /// Verifier ids that APPROVE'd (for hash input).
    pub matching_verifier_ids: Vec<String>,
    /// Rejection notes (if !passed).
    pub rejection_notes: Vec<(String, String)>,
}

/// Result of running the full pipeline.
#[derive(Debug, Clone)]
pub struct PipelineResult {
    /// "PL-D" or "PL-E".
    pub pipeline_tag: String,
    /// Per-phase results (in execution order).
    pub phase_results: Vec<PhaseResult>,
    /// Whether the pipeline passed (all phases passed).
    pub passed: bool,
    /// Union of all matching verifier ids across all phases (for hash input).
    pub all_matching_verifier_ids: Vec<String>,
    /// Output format: `<phase1approves>+<phase2approves>[+...]/<m>`.
    pub output_format: String,
}

/// Run the pipeline.
///
/// Returns `Ok(PipelineResult)` on completion (pass or fail). The caller is responsible
/// for writing the completion hash (if passed) or surfacing the rejection (if failed).
///
/// This is a library-level orchestrator that composes the existing spawn/gather/evaluate
/// primitives. The actual spawning is delegated to the caller (the bin/verifier_loop.rs
/// run_round function) because the spawn primitives are tightly coupled to the binary's
/// async runtime + prompt rendering.
pub fn run_pipeline(
    _config: &Config,
    phases: &[Phase],
    _phase_executor: impl Fn(&Phase) -> PhaseResult,
) -> PipelineResult {
    // The actual executor is in bin/verifier_loop.rs because it needs access to the
    // async runtime + prompt rendering + spawn primitives. This function documents the
    // contract and provides the output-format builder.
    //
    // The bin/verifier_loop.rs `run_pipeline_round` function implements the actual
    // execution loop using this contract.
    unimplemented!(
        "pipeline executor is implemented in bin/verifier_loop.rs::run_pipeline_round; \
         this function documents the contract"
    )
}

/// Build the output format string: `<phase1approves>+<phase2approves>[+...]/<m>`.
///
/// LD6: the `+` boundaries encode the pipeline path. LD27: denominator = m (not n).
pub fn build_output_format(phase_results: &[PhaseResult], m: u32) -> String {
    let segments: Vec<String> = phase_results
        .iter()
        .map(|r| r.approve_count.to_string())
        .collect();
    format!("{}{}", segments.join("+"), format!("/{m}"))
}

/// Determine which pipeline to run based on esca state (LD4, LD21).
///
/// Returns `true` for PL-E, `false` for PL-D.
pub fn should_run_escalation(config: &Config, esca_count: u32, escalation_depth: u32) -> bool {
    if config.esca_threshold == 0 {
        return false;
    }
    if config.m < 2 {
        return false;
    }
    if esca_count < config.esca_threshold {
        return false;
    }
    if config.esca_max_retries > 0 && escalation_depth >= config.esca_max_retries {
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::{default_pipeline, escalation_pipeline, PhaseRole};
    use crate::store::Config;

    fn cfg(n: u32, m: u32, confirm_count: u32, esca_threshold: u32) -> Config {
        Config {
            n,
            m,
            confirm_count,
            esca_threshold,
            ..Config::default()
        }
    }

    #[test]
    fn output_format_pl_d_pass() {
        let results = vec![
            PhaseResult {
                phase_id: "1a".into(),
                role: PhaseRole::Gate,
                passed: true,
                approve_count: 2,
                threshold: 2,
                matching_verifier_ids: vec!["v1".into(), "v2".into()],
                rejection_notes: vec![],
            },
            PhaseResult {
                phase_id: "1b".into(),
                role: PhaseRole::Confirm,
                passed: true,
                approve_count: 1,
                threshold: 1,
                matching_verifier_ids: vec!["v3".into()],
                rejection_notes: vec![],
            },
        ];
        assert_eq!(build_output_format(&results, 2), "2+1/2");
    }

    #[test]
    fn output_format_pl_e_pass() {
        let results = vec![
            PhaseResult {
                phase_id: "1a".into(),
                role: PhaseRole::Mixed,
                passed: true,
                approve_count: 1,
                threshold: 4,
                matching_verifier_ids: vec!["v1".into()],
                rejection_notes: vec![],
            },
            PhaseResult {
                phase_id: "1b".into(),
                role: PhaseRole::Final,
                passed: true,
                approve_count: 2,
                threshold: 1,
                matching_verifier_ids: vec!["v2".into(), "v3".into()],
                rejection_notes: vec![],
            },
        ];
        assert_eq!(build_output_format(&results, 4), "1+2/4");
    }

    #[test]
    fn output_format_gate_reject() {
        let results = vec![PhaseResult {
            phase_id: "1a".into(),
            role: PhaseRole::Gate,
            passed: false,
            approve_count: 1,
            threshold: 2,
            matching_verifier_ids: vec!["v1".into()],
            rejection_notes: vec![("v2".into(), "bad".into())],
        }];
        assert_eq!(build_output_format(&results, 2), "1/2");
    }

    #[test]
    fn should_run_escalation_requires_threshold() {
        let c = cfg(2, 2, 1, 0);
        assert!(!should_run_escalation(&c, 5, 0), "threshold=0 disables");
    }

    #[test]
    fn should_run_escalation_requires_m_ge_2() {
        let c = cfg(1, 1, 1, 2);
        assert!(!should_run_escalation(&c, 5, 0), "m<2 disables");
    }

    #[test]
    fn should_run_escalation_respects_max_retries() {
        let c = cfg(2, 2, 1, 2);
        assert!(!should_run_escalation(&c, 5, 3), "depth >= max_retries disables");
        assert!(should_run_escalation(&c, 5, 2), "depth < max_retries enables");
    }
}
