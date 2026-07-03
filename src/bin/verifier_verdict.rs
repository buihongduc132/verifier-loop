//! `verifier-verdict` (aliased `jewije`) — V*'s interface (approve / reject).
//!
//! Scaffolding entrypoint (tasks.md §1). Real command wiring lands in tasks.md §7 and §10.
//! For now it prints identity + version and exits 0 so the binary target builds and runs.

use verifier_loop::VERSION;

fn main() {
    // Minimal scaffold: confirm the binary is on PATH and reports its version.
    // NOTE: approve / reject are implemented in tasks.md §7 and §10.
    println!("verifier-verdict (jewije) v{VERSION} — scaffold");
    println!("subcommands (wired in tasks.md §7/§10): approve, reject --notes");
}
