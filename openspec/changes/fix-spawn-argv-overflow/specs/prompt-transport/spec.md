## ADDED Requirements

### Requirement: Adapters declare a prompt transport
Every backend adapter (built-in `pi`, `hermes`, `acpx`, and any custom adapter) SHALL declare a `transport` field set to exactly one of `stdin` or `goal-file`. The transport determines how the orchestrator delivers the rendered verifier prompt to the spawned process: `stdin` writes the prompt to the child's stdin pipe; `goal-file` writes the prompt to a temporary file and substitutes its path into the spawn template via a `{goalFile}` placeholder. The orchestrator MUST NOT inline the prompt (or any substitute thereof) into the process argv.

#### Scenario: stdin transport pipes prompt to child
- **WHEN** an adapter declares `transport: "stdin"`
- **THEN** the orchestrator spawns the child with `stdin` configured as `Stdio::piped()`
- **AND** writes the full rendered prompt to that pipe before awaiting the child
- **AND** the rendered argv contains no prompt-derived bytes

#### Scenario: goal-file transport substitutes a tempfile path
- **WHEN** an adapter declares `transport: "goal-file"`
- **THEN** the orchestrator writes the rendered prompt to a temporary file under the OS temp dir
- **AND** substitutes the file's absolute path for every `{goalFile}` placeholder in the spawn template
- **AND** unlinks the tempfile after the child has spawned (the child opens the path before exec completes)

#### Scenario: Built-in pi adapter defaults to stdin
- **WHEN** config `backend: "pi"` with no transport override
- **THEN** the adapter's transport is `stdin`
- **AND** its spawn template is `pi --mode json` (no `-p` flag)

### Requirement: Inline {prompt} templates are rejected at config load
Custom adapter templates in `config.json` MUST NOT contain the `{prompt}` placeholder. The CLI SHALL reject any custom adapter whose spawn or resume template contains `{prompt}` at config-load time, exiting non-zero with a message instructing migration to `{goalFile}` plus `transport: "goal-file"` (or `transport: "stdin"` with no placeholder).

#### Scenario: Legacy inline-prompt template is rejected
- **WHEN** `config.json` declares an adapter with `spawn: "pi -p \"{prompt}\" --mode json"`
- **THEN** the CLI exits non-zero before spawning any verifier
- **AND** the error message names the offending field and the accepted replacements

#### Scenario: goal-file template is accepted
- **WHEN** `config.json` declares `transport: "goal-file"` and `spawn: "pi --goal-file {goalFile} --mode json"`
- **THEN** config loads successfully

### Requirement: Prompt size is unbounded by argv limits
A verifier spawn SHALL succeed for any rendered prompt size up to available disk/memory, regardless of the host `ARG_MAX` or `MAX_ARG_STRLEN`. Neither transport SHALL place the prompt bytes into the child's argv or environment.

#### Scenario: Large prompt spawns successfully
- **WHEN** the rendered prompt is 1 MiB
- **AND** the adapter uses the `stdin` transport
- **THEN** the child spawns without an `E2BIG` / `Argument list too long` error
- **AND** the verifier process receives the full 1 MiB prompt on stdin

#### Scenario: Large prompt via goal-file spawns successfully
- **WHEN** the rendered prompt is 1 MiB
- **AND** the adapter uses the `goal-file` transport
- **THEN** the child spawns without an `E2BIG` error
- **AND** the tempfile contains the full 1 MiB prompt

### Requirement: Tempfile lifecycle is bounded and fail-safe
When `transport: "goal-file"`, the orchestrator SHALL create the tempfile with a unique name, write the prompt atomically, and unlink it immediately after the child process is spawned (the child inherits the open file descriptor). On spawn failure the orchestrator SHALL unlink the tempfile before propagating the error. Tempfiles MUST NOT persist across runs.

#### Scenario: Tempfile is unlinked after successful spawn
- **WHEN** a goal-file spawn succeeds
- **THEN** the tempfile is removed from the filesystem before the gather barrier
- **AND** the verifier continues to read it via its inherited descriptor

#### Scenario: Tempfile is unlinked on spawn failure
- **WHEN** a goal-file spawn fails at `Command::spawn`
- **THEN** the orchestrator unlinks the tempfile
- **AND** propagates the original spawn error

### Requirement: stdin transport tolerates verifier that exits before reading
When `transport: "stdin"`, a write to the child's stdin pipe MAY fail with `EPIPE` if the verifier exits before consuming the full prompt. The orchestrator SHALL treat `EPIPE` on the stdin write as non-fatal when the child has already produced a verdict or a recognizable ACP stream; otherwise it SHALL surface the `EPIPE` as a verifier error (not a panic, not a silent success).

#### Scenario: EPIPE after verdict is non-fatal
- **WHEN** the verifier registers a verdict and then exits
- **AND** the orchestrator's stdin write returns `EPIPE`
- **THEN** the spawn is treated as successful
- **AND** no error is surfaced

#### Scenario: EPIPE before any ACP output is fatal
- **WHEN** the stdin write returns `EPIPE` before any ACP event was parsed
- **THEN** the verifier's verdict remains `null` (fail-closed)
- **AND** the spawn is recorded as errored
