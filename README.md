# verifier-loop

Out-of-process **verifier-loop** CLI that an agent (A) cannot bypass, bias, or forge. Produces a
tamper-evident completion hash (`mmddyy-XXXXXXXX`) only on genuine **n/m** consensus among independent
verifier sessions (V\*) spawned as real ACP-JSON CLI-agent processes.

Two binaries, strict capability separation (design D1):

| binary             | alias    | role | interface                                      |
|--------------------|----------|------|------------------------------------------------|
| `verifier-loop`    | `jewilo` | A    | `NEW`, `RESUME`, spawn, gather, consensus, hash |
| `verifier-verdict` | `jewije` | V\*  | `approve`, `reject --notes "…"`                |

See [`USAGE.md`](USAGE.md) for full invocation reference and [`AGENTS.md`](AGENTS.md) for the
agent-facing source-of-truth pointers.

## Design source

- Proposal / decisions: [`openspec/changes/add-verifier-loop-cli/`](openspec/changes/add-verifier-loop-cli/) (D0–D10, locked decisions LD1–LD27)
- Explore rationale: [`flow/explore/`](flow/explore/), [`flow/findings/`](flow/findings/)
- Behavioural specs: [`openspec/changes/add-verifier-loop-cli/specs/`](openspec/changes/add-verifier-loop-cli/specs/) (6 specs)
- Implementation roadmap: [`openspec/changes/add-verifier-loop-cli/tasks.md`](openspec/changes/add-verifier-loop-cli/tasks.md)

## Build

```bash
cargo build --release
# binaries land in target/release/{verifier-loop,verifier-verdict}
```

## Install + aliases

**Prerequisites**: a recent Rust toolchain (`cargo` 1.70+). Install via [rustup](https://rustup.rs/)
if needed. Then:

```bash
# Option A — install just the two binaries into ~/.cargo/bin:
cargo install --path .

# Option B (recommended) — install binaries AND the short jewilo / jewije aliases
# into <root>/bin (default ~/.local/bin) via the canonical script:
./scripts/install.sh                 # default root: ~/.local
./scripts/install.sh /opt/verifier   # custom --root
```

`scripts/install.sh` runs `cargo install --path . --force --root <root>` then symlinks
`jewilo -> verifier-loop` and `jewije -> verifier-verdict` under `<root>/bin` (falling back to a
full copy on filesystems without symlink support). Cargo cannot express multiple names per
`[[bin]]` target natively, so the aliases are created post-install.

**Ensure the install dir is on your PATH**, then smoke-test:

```bash
# both names must resolve:
command -v jewilo jewije
# must print the mmddyy-XXXXXXXX short-hash form on consensus:
VERIFIER_LOOP_BACKEND_CMD="$(pwd)/scripts/stub_approve.sh" \
  jewilo NEW "smoke test"   # e.g.  070326-00a50e40
```

## `config.json` reference

`~/.verifier-loop/config.json` carries the tunables that gate spawning, consensus, and the frozen
diff fed to verifiers (tasks.md §2.2). On-disk keys are camelCase; all fields are optional.

| key                  | type    | default     | meaning                                                                     |
|----------------------|---------|-------------|-----------------------------------------------------------------------------|
| `n`                  | u32     | `2`         | consensus threshold — minimum APPROVE verdicts required to pass (n of m).   |
| `m`                  | u32     | `2`         | number of verifiers spawned per round.                                      |
| `maxTurn`            | u32     | `3`         | per-verifier turn budget; once exhausted the session is spawned fresh (D8). |
| `backend`            | string  | `"pi"`      | ACP backend key: `pi` \| `hermes` \| `acpx` \| a custom/stub key.           |
| `gitDiffMaxChars`    | u64     | `10000`     | cap on the frozen `git diff` snapshot handed to each verifier (chars).      |
| `verifierTimeoutSec` | u64     | `1800`      | per-verifier wall-clock timeout in seconds (D9); a timeout leaves a null verdict. |

Semantics (fail-closed):

- **Missing** `config.json` → fully defaulted [`Config`].
- **Partial** `config.json` → present fields honoured, missing fields defaulted.
- **Malformed** `config.json` → hard error; never silently defaulted.

## Usage examples

```bash
# A — start a fresh goal (round 1); prints `goalId: <id>` then, on consensus, the mmddyy-XXXXXXXX hash:
verifier-loop NEW "implement the foo-bar endpoint with tests"

# A — drive the next round, appending fix notes from the prior round's rejections:
verifier-loop RESUME <goalId> --fix "addressed the missing error path"

# V* — register a verdict (identity comes from VERIFIER_LOOP_* env, NOT arguments):
verifier-verdict approve
verifier-verdict reject --notes "issue 1: missing test for the error path"
```

On n/m APPROVE consensus the short completion hash (`mmddyy-XXXXXXXX`) is printed to stdout and
`completion.json` is written under the goal directory (carrying both the short `hash` and the
full 64-hex `fullDigest` for exact audit recompute). On failure the rejection summary is printed
to stderr and the exit code is non-zero.

## Completion-hash formula

```
short       = mmddyy + "-" + first8hex(SHA256(inputs))   # displayed, printed
fullDigest  = SHA256(inputs)                              # 64 hex, stored in completion.json

inputs      = salt
            + goalId
            + goalSignature
            + String(round)
            + canonicalJSON(matchingVerdicts sorted by verifierId)
            + matchedAtISO

where  goalSignature = SHA256(salt + goalText + createdAt)
      mmddyy         = UTC date of matchedAt (MMDDYY, e.g. 070326 for 2026-07-03)
```

- `salt` — per-store random secret; never printed.
- `matchingVerdicts` — the matching APPROVE verdicts, serialized as **canonical JSON**: objects
  sorted by `verifierId` ascending, object keys alphabetical, no whitespace.
- The **short hash** (`mmddyy-XXXXXXXX`) is the human/agent-facing ID — memorable, trivially
  invokable by sub-agents. Example: `070326-a1b2c3d4`.
- The **full digest** (`fullDigest`, 64 hex) is stored in `completion.json` and is the exact
  (deterministic) tamper guard. 8 hex alone (32 bits) is too weak as a sole guard, so audit
  compares `fullDigest`; the short hash is a scannable label.
- Any edit to `goalText` (breaks `goalSignature`) or to a stored verdict changes BOTH the short
  hash (w.h.p.) and the full digest (deterministically), so recompute will not match stored.

## Fail-closed guarantees (D9)

- A **NULL** verdict (crash / timeout / forgot-to-call-verdict) **never** becomes APPROVE.
- A missing `~/.verifier-loop/` or goal directory yields **no hash**.
- Editing `goal.json` `goalText` after creation breaks `signature.json` and every downstream hash.
- Editing a stored APPROVE verdict invalidates the completion hash on recompute.

## Coverage gate (>=80% lines)

```bash
cargo llvm-cov --fail-under-lines 80 --html    # report at target/llvm-cov/html/index.html
```
