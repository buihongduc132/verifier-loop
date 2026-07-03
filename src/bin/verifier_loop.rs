//! `verifier-loop` (aliased `jewilo`) — A's interface (NEW / RESUME).
//!
//! Scaffolding entrypoint (tasks.md §1). Real command wiring lands in tasks.md §10.
//! For now it prints identity + version and exits 0 so the binary target builds and runs.

use verifier_loop::VERSION;

fn main() {
    // Minimal scaffold: confirm the binary is on PATH and reports its version.
    // NOTE: argument parsing / NEW / RESUME are implemented in tasks.md §3 and §10.
    println!("verifier-loop (jewilo) v{VERSION} — scaffold");
    println!("subcommands (wired in tasks.md §3/§10): NEW, RESUME");
}
