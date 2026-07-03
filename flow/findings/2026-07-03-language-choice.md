# Language Choice — Rust vs Go vs TypeScript

> Date: 2026-07-03
> Status: locked → **Rust**
> Source: explore-mode conversation in pi-plugins `flow/findings/verifier-loop-cli/` (turn 6, post-proposal)
> Affects: design.md D0, tasks.md §1

## Context

verifier-loop is a short-lived orchestrator CLI: spawn → wait → hash → exit. It is not a daemon. The real cost is the spawned V* LLM sessions, but at "many loops running concurrently" the coordinator process memory still adds up. Two CLIs (`verifier-loop`/jewilo, `verifier-verdict`/jewije) are spawned per verification round and once per resume.

Hard requirements:
- **Strict types** — central safety property is fail-closed; every error path must be explicit.
- **Low resource usage** — many concurrent loops expected.
- **Single static binary** — `jewilo`/`jewije` must drop in without runtime deps.

## Decision

**Rust.**

The fail-closed design depends on every error path being explicit and every ACP event being handled. Rust enforces both at compile time; Go and TS do not. The ~1 day toolchain-setup overhead is accepted as the cost of compiler-enforced safety.

## Boilerplate comparison (concrete, same operations)

### Op 1 — Strict-typed struct + JSON round-trip

**Rust (17 lines):**
```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Verdict {
    pub verifier_id: String,
    pub status: VerdictStatus,
    pub notes: Vec<String>,
    pub registered_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum VerdictStatus { Null, Approve, Reject }

// read
let v: Verdict = serde_json::from_str(&contents)?;
// write
serde_json::to_string_pretty(&v)?;
```

**Go (17 lines):**
```go
type Verdict struct {
    VerifierID   string        `json:"verifierId"`
    Status       VerdictStatus `json:"status"`
    Notes        []string      `json:"notes"`
    RegisteredAt string        `json:"registeredAt"`
}

type VerdictStatus string

const (
    VerdictNull    VerdictStatus = "NULL"
    VerdictApprove VerdictStatus = "APPROVE"
    VerdictReject  VerdictStatus = "REJECT"
)

// read
var v Verdict
json.Unmarshal([]byte(contents), &v)
// write
b, _ := json.MarshalIndent(v, "", "  ")
```

**Lines: tie (17/17).**

### Op 2 — Spawn child, parse streaming JSON lines, apply timeout

**Rust (~20 lines, ~30 with tokio timeout):**
```rust
use std::process::Command;
use std::io::BufRead;

let child = Command::new("pi")
    .args(["-p", &prompt, "--mode", "json"])
    .env("VERIFIER_LOOP_GOAL_ID", &goal_id)
    .stdout(Stdio::piped())
    .spawn()?;

let stdout = child.stdout.take().unwrap();
let reader = BufReader::new(stdout);
for line in reader.lines() {
    let line = line?;
    let evt: AcpEvent = serde_json::from_str(&line)?;
    match evt {
        AcpEvent::Session { id } => sid = Some(id),
        AcpEvent::AgentEnd { messages, .. } => {
            final_output = messages.last().cloned();
        }
        _ => {}
    }
}
```
(+ need `tokio` for real timeout-with-kill, ~15 more lines with `select!`)

**Go (18 lines):**
```go
cmd := exec.CommandContext(ctx, "pi",
    "-p", prompt, "--mode", "json")
cmd.Env = append(os.Environ(),
    "VERIFIER_LOOP_GOAL_ID="+goalID)

stdout, _ := cmd.StdoutPipe()
cmd.Start()

scanner := bufio.NewScanner(stdout)
scanner.Buffer(make([]byte, 1024*1024), 1024*1024)
for scanner.Scan() {
    var evt AcpEvent
    json.Unmarshal(scanner.Bytes(), &evt)
    switch evt.Type {
    case "session":
        sid = evt.ID
    case "agent_end":
        finalOutput = lastMsg(evt.Messages)
    }
}
cmd.Wait()
```

**Lines: Go slightly less; `exec.CommandContext` natively kills on ctx cancel — Rust needs `tokio::select!` + manual kill.**

### Op 3 — Strict error handling

**Rust (9 lines) — `?` propagates, typesafe:**
```rust
fn read_goal(path: &Path) -> Result<Goal, GoalError> {
    let text = fs::read_to_string(path)
        .map_err(|e| GoalError::Read(e))?;
    let g: Goal = serde_json::from_str(&text)
        .map_err(|e| GoalError::Parse(e))?;
    if g.goal_text.is_empty() {
        return Err(GoalError::Empty);
    }
    Ok(g)
}
```

**Go (14 lines) — `if err != nil` × N:**
```go
func ReadGoal(path string) (Goal, error) {
    text, err := os.ReadFile(path)
    if err != nil {
        return Goal{}, fmt.Errorf("read goal: %w", err)
    }
    var g Goal
    if err := json.Unmarshal(text, &g); err != nil {
        return Goal{}, fmt.Errorf("parse goal: %w", err)
    }
    if g.GoalText == "" {
        return Goal{}, errors.New("goal text empty")
    }
    return g, nil
}
```

**Lines: Rust 35% less; `Result<T,E>` enforced at compile time — cannot ignore an error.**

## Strict-types comparison

| | Rust | Go |
|---|---|---|
| Null safety | `Option<T>`, no null | `nil` everywhere, runtime panics |
| Error handling | `Result<T,E>`, must handle | `error`, ignorable (`_ = err`) |
| Enums w/ data | first-class | string constants only |
| Sum types for ACP events | `enum AcpEvent { Session{id}, AgentEnd{..} }` — exhaustive match | `switch evt.Type` — compiler won't catch missing cases |
| Generics | full | limited |
| Pattern matching | exhaustive, compiler-checked | none |
| Field-tag validation | serde + `validator` crate | manual |

**Rust is meaningfully stricter.** The decisive factor: `enum` with data + exhaustive `match` = the compiler tells you when a new ACP event type is unhandled. Go's `switch` silently falls through to default → a new event type added to the ACP spec is a silent bug.

For ACP JSON parsing specifically, a missed `agent_end` or `session` event is exactly the kind of silent bug that breaks verifier-loop (no SID → no resume → no final output → null verdict). Rust's exhaustive match catches that at compile time.

## Boilerplate summary

| Op | Rust | Go | Notes |
|---|---|---|---|
| Struct+JSON | 17 | 17 | tie |
| Spawn+stream | 20-30 | 18 | Go simpler (CommandContext) |
| Error handling | 9 | 14 | Rust shorter + stricter |
| Project setup | 6 files (Cargo.toml, src/main.rs, etc) | 3 files (go.mod, main.go) | Go simpler |
| Build/install | `cargo build --release` | `go build` | tie |
| Cross-compile | one line (rustup target) | one line (GOOS/GOARCH) | tie |

**Net:** Roughly equal line count. Rust adds ~1 day of project setup overhead; Go is "create one file and go."

## Why Rust wins for this project

- **Exhaustive `enum` + `match`** on ACP events and `VerdictStatus` — compiler catches unhandled events and invalid state transitions. A missed `agent_end`/`session` event is exactly the silent bug that breaks fail-closed (no SID → no resume → null verdict).
- **`Result<T,E>` enforced at compile time** — fail-closed is the central safety property of this design; Rust makes ignoring an error a compile error, Go's `error` is ignorable via `_ =`.
- **`Option<T>` instead of `nil`** — no nil-deref panics on the goal store / verdict slots.
- **Memory profile** — short-lived orchestrator (spawn → wait → hash → exit), baseline ~3MB per process vs Go ~10MB vs Node ~50MB. At "many loops running concurrently" this matters.
- **Single static binary** — no runtime dependency on the host; `jewilo`/`jewije` are drop-in.

## Alternatives considered

- **Go**: simpler project setup (~1 day saved), native `exec.CommandContext` kill-on-timeout, but string-typed enums (silent typos like `VerdictNull` vs `VerdictNul`), default-case `switch` (silent missing-event), and ignorable `error` work against the fail-closed guarantee.
- **TypeScript (Node/Bun)**: rejected — runtime dependency, ~50MB baseline, no compiler-enforced error handling. Only wins if shared library code with pi were needed, but the contract is shelling out to `pi -p --mode json`. No shared library needed.

## Resource profile (relevant to "many loops running concurrently")

| | TS (Node/Bun) | Go | Rust |
|---|---|---|---|
| Startup | 30-150ms | ~5ms | ~2ms |
| Mem/proc idle | 30-50MB | 5-10MB | 2-5MB |
| 100 concurrent | 3-5GB | 500MB-1GB | 200-500MB |
| Process spawn/parse | ok | excellent (`os/exec`+`context`) | ok (more ceremony) |
| JSON stream parse | trivial | trivial | ok |
| Single static binary | no (needs runtime) | yes | yes |
| Dev velocity | fast | fast | slow |

The real bottleneck is the spawned V* LLM sessions, not the loop itself — but at scale the coordinator memory still adds up. Go's ~10MB vs Node's ~50MB is 5x; Rust's ~3MB is ~17x better than Node.

## Rust-specific design wins (A1-A3)

```
[A1] ACP event enum
  enum AcpEvent {
      Session { id: String },
      AgentStart,
      TurnStart,
      MessageStart { message: Message },
      MessageEnd { message: Message },
      AgentEnd { messages: Vec<Message>, will_retry: bool },
  }
  → exhaustive match = compiler catches unhandled events
  → Go equivalent needs default case, easy to miss

[A2] VerifierState machine
  enum VerdictStatus { Null, Approve, Reject }
  → Rust: transition fn null→approve reject at compile time
  → Go: string-typed, "ZERRO" typo = silent bug

[A3] Outcome type
  → Result<Hash, Rejection> enforced
  → Go: hash + err optional
```

## Callsout

- **[CA1]** If you later want the verifier-loop *skill* in pi-plugins to share ACP types with the CLI, that's a thin TS shim — Rust CLI stays standalone, skill calls it as a subprocess. No lock-in.
- **[CA2]** Toolchain setup overhead (~1 day): check `rustup --version`. If absent, `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`. Accepted cost; amortised over years of maintenance.
- **[CA3]** tokio adds binary size (~2-5MB) but is required for parallel spawn + timeout-with-kill. Alternative: `std::thread` + manual `wait_timeout` (unstable). tokio is the right call.
