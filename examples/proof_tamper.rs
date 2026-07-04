//! Proof-of-tamper harness (verification-contract item 4).
//! Demonstrates the three fail-closed tamper vectors on a captured goal-dir:
//!   1. untampered goal -> verify_signature() == Ok
//!   2. goalText edited -> verify_signature() == Err(mismatch)
//!   3. verdict edited  -> recomputed completion hash != stored hash
//! Usage: cargo run --example proof_tamper <goal_dir>  (goal_dir = .../goals/<uuid>)
use serde::Deserialize;
use std::path::PathBuf;
use verifier_loop::consensus::{self, CompletionRecord};
use verifier_loop::goal;
use verifier_loop::verdict;
use verifier_loop::store::{load_config_in};

fn main() {
    let dir = PathBuf::from(std::env::args().nth(1).expect("goal_dir arg"));
    let root = dir.parent().unwrap().parent().unwrap();
    let gid = dir.file_name().unwrap().to_str().unwrap().to_string();

    println!("goal_dir : {}", dir.display());
    println!("goalId   : {gid}");

    let ok = goal::verify_signature(root, &gid).is_ok();
    println!("[1] untampered verify_signature == Ok : {ok}");

    let goal_path = dir.join("goal.json");
    let raw = std::fs::read_to_string(&goal_path).unwrap();
    let mut j: serde_json::Value = serde_json::from_str(&raw).unwrap();
    let orig = j["goalText"].as_str().unwrap().to_string();
    j["goalText"] = serde_json::Value::String(format!("{orig} TAMPERED"));
    std::fs::write(&goal_path, serde_json::to_string_pretty(&j).unwrap()).unwrap();
    let mismatch = goal::verify_signature(root, &gid).is_err();
    println!("[2] goalText edited -> signature mismatch : {mismatch}");
    std::fs::write(&goal_path, &raw).unwrap();
    debug_assert!(goal::verify_signature(root, &gid).is_ok());

    let completion: CompletionRecord =
        serde_json::from_str(&std::fs::read_to_string(dir.join("completion.json")).unwrap())
            .unwrap();
    let round = completion.round_number;

    let cfg = load_config_in(root).unwrap();
    let v1_path = dir.join(format!("rounds/{round}/v1/verdict.json"));
    let vraw = std::fs::read_to_string(&v1_path).unwrap();
    let mut vj: serde_json::Value = serde_json::from_str(&vraw).unwrap();
    vj["status"] = serde_json::Value::String("REJECT".into());
    std::fs::write(&v1_path, serde_json::to_string_pretty(&vj).unwrap()).unwrap();

    // Re-read verdicts from disk + re-evaluate, exactly as the CLI does. A flipped
    // verdict drops out of the matching set -> different (or no) hash.
    let mut verdicts = Vec::new();
    for vid in ["v1", "v2"] {
        if let Ok(rec) = verdict::read_verdict(root, &gid, vid, round) {
            verdicts.push((vid.to_string(), rec));
        }
    }
    let result = consensus::evaluate(root, &gid, round, &verdicts, cfg.n, cfg.m);
    let salt = std::fs::read_to_string(root.join(".salt")).unwrap();
    let goal_sig = {
        #[derive(Deserialize)]
        struct S {
            signature: String,
        }
        serde_json::from_str::<S>(&std::fs::read_to_string(dir.join("signature.json")).unwrap())
            .unwrap()
            .signature
    };
    let tampered_passed = result.passed;
    let tampered_hash = if result.passed {
        Some(consensus::compute_hash(
            salt.trim(),
            &gid,
            &goal_sig,
            round,
            &result.matching_verdicts,
            &completion.matched_at,
            "",
        ))
    } else {
        None
    };
    std::fs::write(&v1_path, &vraw).unwrap();
    let stored = &completion.hash;
    let differs = tampered_hash.as_ref().map(|h| h.short_hash()) != Some(stored.as_str());
    println!("[3] verdict edited -> consensus re-eval passed : {tampered_passed}");
    println!("    recomputed hash (None=round no longer passes) : {tampered_hash:?}");
    println!("    verdict edit -> hash differs (fail-closed)    : {differs}");
}
