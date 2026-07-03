# verifier-loop

Out-of-process **verifier-loop** CLI that an agent (A) cannot bypass, bias, or forge. Produces a
tamper-evident completion hash (`vl:<40 hex>`) only on genuine **n/m** consensus among independent
verifier sessions (V\*) spawned as real ACP-JSON CLI-agent processes.

Two binaries, strict capability separation (design D1):

| binary           | alias    | role | interface                                   |
|------------------|----------|------|---------------------------------------------|
| `verifier-loop`  | `jewilo` | A    | `NEW`, `RESUME`, spawn, gather, consensus, hash |
| `verifier-verdict` | `jewije` | V\* | `approve`, `reject --notes "…"` |

> **Status:** scaffold (tasks.md §1). Behaviour lands group-by-group under strict TDD per
> [`openspec/changes/add-verifier-loop-cli/tasks.md`](openspec/changes/add-verifier-loop-cli/tasks.md).

## Design source

- Proposal / decisions: [`openspec/changes/add-verifier-loop-cli/`](openspec/changes/add-verifier-loop-cli/) (D0–D10, locked decisions LD1–LD27)
- Explore rationale: [`flow/explore/`](flow/explore/), [`flow/findings/`](flow/findings/)
- Behavioural specs: [`openspec/changes/add-verifier-loop-cli/specs/`](openspec/changes/add-verifier-loop-cli/specs/) (6 specs)

## Build

```bash
cargo build --release
# binaries land in target/release/{verifier-loop,verifier-verdict}
```

## Install + aliases

```bash
cargo install --path .
# then create the short aliases (tasks.md §10.4):
ln -sf "$(which verifier-loop)"    "$(dirname "$(which verifier-loop)")/jewilo"
ln -sf "$(which verifier-verdict)" "$(dirname "$(which verifier-verdict)")/jewije"
```

## Coverage gate (>=80% lines)

```bash
cargo llvm-cov --fail-under-lines 80 --html    # report at target/llvm-cov/html/index.html
# alternative:
cargo tarpaulin --skip-clean --out Html --fail-under 80
```

## Fail-closed guarantees (D9)

- A NULL verdict (crash / timeout / forgot-to-call-verdict) **never** becomes APPROVE.
- A missing `~/.verifier-loop/` or goal directory yields **no hash**.
- Editing `goal.json` goalText after creation breaks `signature.json` and every downstream hash.
- Editing a stored APPROVE verdict invalidates the completion hash on recompute.

See `USAGE.md` (tasks.md §11) and `AGENTS.md` for full reference.
