# Jewilo Fix Summary

## Problem
Jewilo was unable to run due to misconfigured ACP adapters. The original config specified `hermes-verifier` which:
1. Does not exist as a binary
2. Hermes CLI does not support `--mode json` flag
3. Hermes `acp` subcommand hangs and doesn't properly read from stdin

## Root Cause Analysis
1. **hermes-verifier** binary not found on PATH
2. **hermes** CLI uses `hermes acp` for ACP mode, not `--mode json`
3. **hermes acp** does not properly read prompts from stdin (hangs indefinitely)
4. Only **pi** adapter (`pi --mode json`) correctly implements stdin-based ACP protocol

## Solution Implemented

### 1. Added new built-in adapter: `pi-rag`
**File**: `src/acp/adapters.rs`
- Added `pi-rag` adapter that uses `pi --model bhd-litellm/rag-quick --mode json`
- This allows dump verifiers to use a specific model as required
- Updated error message to include `pi-rag` in valid adapter list

### 2. Updated configuration
**File**: `~/.verifier-loop/config.json`
```json
{
  "n": 2,
  "m": 3,
  "maxTurn": 200,
  "dumpAdapter": "pi-rag",
  "smartAdapter": "pi",
  "confirmCount": 1,
  "gitDiffMaxChars": 8000,
  "verifierTimeoutSec": 300,
  "minGoalChars": 500
}
```

### 3. Configuration Changes Explained
- **dumpAdapter**: Changed from `hermes-verifier` to `pi-rag` (uses `pi --model bhd-litellm/rag-quick`)
- **smartAdapter**: Changed from `hermes-verifier` to `pi` (uses standard pi)
- **verifierTimeoutSec**: Reduced from 1800 to 300 for faster iteration during testing
- Both adapters use stdin transport (working ACP protocol implementation)

## Technical Details

### ACP Adapter Requirements
1. Must accept `--mode json` flag for ACP JSON protocol
2. Must read prompt from stdin when using stdin transport
3. Must emit ACP events: `{"type":"session",...}`, `{"type":"message",...}`, etc.
4. Must properly handle `--session {sid}` for resume operations

### Working Adapters
- ✅ `pi --mode json` - Fully functional, reads stdin, emits ACP events
- ✅ `pi-rag` (pi --model bhd-litellm/rag-quick --mode json) - New adapter for model-specific usage
- ❌ `hermes --mode json` - Does not exist
- ❌ `hermes acp` - Hangs, doesn't read stdin properly
- ❌ `acpx --mode json` - Wrong flag (uses --format json) and requires pre-existing session

## Build and Deploy
```bash
cd /home/bhd/Documents/Projects/bhd/verifier-loop
cargo build --release
cp target/release/verifier-loop ~/.local/bin/jewilo
```

## Verification
Jewilo now successfully:
1. Spawns verifier processes without hanging
2. Uses correct ACP adapters (pi and pi-rag)
3. Receives verdicts from verifiers (observed APPROVE/REJECT in testing)
4. Processes multiple rounds when consensus not reached

## Known Limitations
- Verifier execution can be slow depending on LLM backend response time
- Current timeout (300s) may not be sufficient for complex goals with large context
- The `hermes` adapter is not usable for jewilo at this time due to stdin limitations
