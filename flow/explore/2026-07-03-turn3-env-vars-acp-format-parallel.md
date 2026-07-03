# Explore Turn 3 — env-vars-acp-format-parallel-fail-closed

> Date: 2026-07-03
> Phase: explore (ground truth + spawn mechanics)
> Status: complete → turn4

## [user] (verbatim — trust chain L1)

> 1. Env var + prompt {{}} embedded one. Because that ID is different each time isn't it ?
>
> 2. pi / hermes / acpx having their own custom adapter.
> for the rest , make it to be able to configured as json above whenever needed ;
> Assumes that all of them having acp output format.
>
> Current pi already having acp , thus make it acp format instead of just normal pi -p;
>
> 3. Just verdict registered , exit 0;
> 4. V* naturally exit. It is cli-agent , check the cli-agent skill to be more understand these.
> Run pi -p "say hi" --mode json to understand how it works . NOW. Do not keep assuming.
>
> 5. NO. [cannot change verdict after registering]
>
> ---
>
> Also , ensure these clis (v*) are running in parallel. Non-blocking; Gather all in the end.

## [assistant]

**Action taken**: ran `pi -p "say hi" --mode json` and `pi --session <sid> -p "..." --mode json` to ground all assumptions in actual pi behavior. **Stop assuming.**

### Ground Truth — pi -p --mode json Output Stream
```
┌──────────────────────────────────────────────────────────────────┐
│              pi -p --mode json Output Stream                      │
├──────────────────────────────────────────────────────────────────┤
│                                                                  │
│  Line 1: {"type":"session","id":"<UUID>","cwd":"..."}           │
│          ↑ SID for resume — emitted FIRST                        │
│                                                                  │
│  Then events stream in order:                                    │
│  ├── {"type":"agent_start"}                                      │
│  ├── {"type":"turn_start"}                                       │
│  ├── {"type":"message_start","message":{"role":"user",...}}     │
│  ├── {"type":"message_end","message":{"role":"user",...}}       │
│  ├── {"type":"message_start","message":{"role":"assistant"}}    │
│  ├── {"type":"message_update","assistantMessageEvent":{...}}    │
│  │   └── types: text_start, text_delta, text_end                 │
│  │              thinking_start, thinking_delta, thinking_end     │
│  │              tool_call, ...                                    │
│  ├── {"type":"message_end","message":{"role":"assistant"}}      │
│  │   ↑ Final assistant message with usage, stopReason           │
│  ├── {"type":"turn_end","toolResults":[...]}                    │
│  └── {"type":"agent_end","messages":[...],"willRetry":false}    │
│      ↑ Session done — full message history here                 │
│                                                                  │
│  Resume: pi --session <UUID> -p "next" --mode json              │
│  → Same SID, continues the conversation                          │
│                                                                  │
└──────────────────────────────────────────────────────────────────┘
```

### Two CLIs (confirmed)
```
CLI 1: verifier-loop   (A's interface)
  verifier-loop NEW "fix auth" [--context "..."]
  verifier-loop RESUME <goalId> [--fix "..."]
  - Reads config.json
  - Spawns N verifier sessions IN PARALLEL (non-blocking)
  - Gathers all agent_end events
  - Checks n/m match
  - Outputs: hash or rejection

CLI 2: verifier-verdict   (V*'s interface)
  verifier-verdict approve
  verifier-verdict reject --notes "issue 1: ..."
  - Reads identity from ENV (injected by spawn):
      VERIFIER_LOOP_GOAL_ID=abc-123
      VERIFIER_LOOP_VERIFIER_ID=v1
      VERIFIER_LOOP_ROUND=1
  - Writes verdict.json
  - Outputs: "Verdict registered"
  - Exits 0
  - First verdict is FINAL. Second call = rejected.
```

### Why Both Env Var AND Prompt {{}}
```
ENV VAR: machine-readable, for verifier-verdict CLI
  → verifier-verdict reads these
  → writes to correct goal/round/v1/ path

PROMPT {{}}: human-readable context for V* (the LLM)
  → V* sees its identity in the prompt
  → V* knows it's a verifier, not the worker
  → V* can reference "I am V1" in notes

Both needed because:
- V* is an LLM — it reads the PROMPT to understand its role
- verifier-verdict is a CLI — it reads ENV to find the path
- The IDs are different each spawn, so can't hardcode
```

### ACP Adapter Contract (shared parser, adapter-specific spawn)
```
interface ACPAdapter {
  spawnCmd(prompt, env): { command, env }
  resumeCmd(sid, prompt, env): { command }
  parseStream(line): ACLEvent          // SHARED across all backends
  extractSID(sessionEvent): string
  extractFinalOutput(agentEndEvent): string
}

Built-in adapters:
├── pi:     spawnCmd = `pi -p "<prompt>" --mode json`
│           resumeCmd = `pi --session <sid> -p "<prompt>" ...`
├── hermes: spawnCmd = `hermes <prompt> --mode json`
│           resumeCmd = `hermes --session <sid> <prompt> ...`
├── acpx:   spawnCmd = `acpx <prompt> --mode json`
│           resumeCmd = `acpx --session <sid> <prompt> ...`

Custom (config.json):
└── { "type": "custom", "spawn": "<template>", "resume": "<template>", "jsonFlag": "--mode json" }

All share the SAME stream parser (ACP format is universal)
```

### Parallel Spawn + Gather
```
verifier-loop NEW "fix auth"
│
│  ┌─ Spawn V1 ─────────────────────────────┐
│  │  ENV: VERIFIER_LOOP_GOAL_ID=abc-123    │
│  │       VERIFIER_LOOP_VERIFIER_ID=v1     │
│  │       VERIFIER_LOOP_ROUND=1            │
│  │  CMD: pi -p "<prompt>" --mode json     │  PARALLEL
│  │  V1 reviews...                          │  NON-BLOCKING
│  │  V1 calls: verifier-verdict reject ...  │
│  │  V1 naturally exits (agent_end)         │
│  └────────────────────────────────────────┘
│  ┌─ Spawn V2 ─────────────────────────────┐
│  │  ...                                    │
│  └────────────────────────────────────────┘
│  ┌─ Spawn V3 (if 2/3 config) ─────────────┐
│  │  ...                                    │
│  └────────────────────────────────────────┘
│  ┌─ WAIT for all agent_end ───────────────┐
│  └────────────────────────────────────────┘
│  Read all verdict.json files
│  Check: approved >= n ? (e.g., 2 out of 3)
```

### V* Spawn Flow Detail
```
1. verifier-loop NEW "fix auth"
2. CLI generates goalId, writes goal.json (immutable)
3. CLI pre-creates verdict.json {status:null} for each V*
4. CLI captures git status, diff, file times (frozen snapshot)
5. CLI renders prompt template with variables
6. CLI spawns V1, V2, ... Vm IN PARALLEL:
   ENV: VERIFIER_LOOP_GOAL_ID, _VERIFIER_ID, _ROUND
   CMD: pi -p "<prompt>" --mode json
7. V* reviews, then calls:
   verifier-verdict reject --goalId <id> --notes "issue 1..."
   (goalId read from ENV; V* doesn't need to pass it)
8. verifier-verdict CLI:
   - Validates goalId exists
   - Validates verifierId + round match environment
   - Writes verdict.json
   - Captures V*'s final output (from stream)
   - Outputs "Verdict registered", exits 0
9. V* naturally exits (agent_end)
10. verifier-loop detects V* done, reads verdict.json
```

### Fail-Closed Matrix
```
Scenario                         │ Result
─────────────────────────────────┼───────────────────────────
V* crashes before verdict        │ verdict = NULL → no pass
V* times out                     │ verdict = NULL → no pass
V* forgets to call verdict       │ verdict = NULL → no pass
V* calls verdict twice           │ first wins, second rejected
V* calls APPROVE without review  │ that's the 40% error rate
n/m not reached                  │ output rejection to A
A deletes ~/.verifier-loop/      │ hash invalid, no proof
A tampers completion.json        │ hash won't match audit
A forges a hash                  │ won't match any goalId
```

### Session Reuse on RESUME (maxTurn)
```
verifier-loop RESUME abc-123 --fix "fixed issues 1 and 2"
│
│  Load goal state
│  For each V* in this round:
│  ├── V1: turns used = 1 < maxTurn(3) → REUSE
│  │   CMD: pi --session <V1-sid> -p "round 2, fixed..." json
│  │   ENV: VERIFIER_LOOP_ROUND=2 (round changes!)
│  │   (verifierId stays v1)
│  │
│  └── If V1 had used 3 turns already:
│      ├── FRESH spawn (new session, new SID)
│      └── Old V1-sid archived in round-1/v1/
```

### Open Questions Remaining
1. RESUME prompt template (what V* sees on round 2+)
2. config.json final schema
3. completion hash exact formula

Status at end of turn 3: Ground truth verified via pi -p --mode json. Parallel spawn mechanics clear. Fail-closed confirmed. Three items remain for finalization. → turn4
