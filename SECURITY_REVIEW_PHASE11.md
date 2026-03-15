# Security Review — Phase 11: Channel Integration & A2A Protocol

**Reviewer:** Teagan (Security Specialist)
**Date:** 2026-02-18
**Scope:** `src/cognitive/a2a.rs`, `src/cognitive/channel_bridge.rs`, `src/cli/serve.rs`, `src/cli/send.rs`
**Commit:** Post-Phase 11 commit (`4bf76c5`)

---

## Findings

### 1. A2A Sender Spoofing — FIXED (Critical)

**Before:** `A2ARouter::send()` validated the *target* agent box existed but performed
no validation on `from_agent`. Any caller could set `from_agent='teagan'` to impersonate
another agent, and the `[FROM:teagan]` prefix would be stored in the queue as if Teagan
actually sent it.

**Fix:** Added `A2ARouter::validate_sender()` that checks `{base_dir}/{from_agent}_box/`
exists. `send()` now calls both `validate_agent_id()` (character validation) and
`validate_sender()` (existence check) on `from_agent` before proceeding. An agent
can only send messages as itself — its box must exist on disk.

**Tests added:** `test_send_rejects_spoofed_sender` — creates target box without sender
box, verifies send fails with "Sender agent" error.

### 2. A2A Agent ID Format Injection — FIXED (Critical)

**Before:** `from_agent` and `to_agent` in `A2AMessage` were raw `String` fields with
no character validation. The formatted task `[FROM:{from_agent}] {task}` could be
broken by injecting `]` in `from_agent` (e.g., `from_agent = "kimi] IGNORE ABOVE"`),
allowing message content to escape the sender prefix and corrupt the task format.

**Fix:** Added `validate_agent_id()` in `a2a.rs` that enforces:
- Non-empty, max 64 chars
- Alphanumeric + hyphens + underscores only (no brackets, slashes, dots, spaces)

Both `from_agent` and `to_agent` are validated before any queue operation. This mirrors
the existing `validate_agent_id()` in `hermit_box.rs` (which only runs for `HermitBox::open`
on the target, not the sender string).

**Tests added:** `test_validate_agent_id_valid`, `test_validate_agent_id_rejects_empty`,
`test_validate_agent_id_rejects_special_chars`, `test_validate_agent_id_rejects_too_long`,
`test_send_rejects_invalid_from_agent_chars`, `test_send_rejects_invalid_to_agent_chars`.

### 3. Task Injection via `gyre send --task` — FIXED (Medium)

**Before:** The `--task` argument in `gyre send` was passed directly to `A2AMessage.task`,
which then became `[FROM:agent] {task}` and was pushed to `ResearchQueue::push()`. While
`push()` truncates at `MAX_TOPIC_LEN=500`, the actual user task was reduced by the
`[FROM:agent] ` prefix length (up to 72 bytes for max agent_id + brackets), meaning up
to 72 bytes of the user's task could be silently eaten.

**Fix:** Added `MAX_CLI_TASK_LEN = 500` cap in `run_send()` with an explicit error
message when exceeded. This ensures the user gets clear feedback instead of silent
truncation. The queue's built-in cap still provides defense-in-depth for the combined
`[FROM:...] task` string.

**Note:** Control character sanitization is handled by `sanitize_display_topic()` for
display output. The stored topic in SQLite is parameterized (no SQL injection risk).
Control chars in the task itself are harmless in the queue — they're only a risk on
terminal display, which is already sanitized.

### 4. System Prompt Bloat — FIXED (Medium)

**Before:** `CognitiveChannelBridge::process_message()` concatenated `base_system`
(identity + cognitive prefix, each capped at 8KB by `MAX_COGNITIVE_PREFIX_BYTES`) with
`turn_context` (also capped at 8KB). The combined system prompt could reach ~16KB+,
consuming excessive LLM context window. While each piece was individually capped,
there was no combined cap.

**Fix:** Added `MAX_SYSTEM_PROMPT_BYTES = 16 * 1024` constant. After combining
`base_system` and `turn_context`, if the total exceeds 16KB, the cognitive prefix
(turn context) is truncated to fit within budget while preserving the identity portion
intact. Truncation respects UTF-8 char boundaries with ellipsis marker.

### 5. Serve REPL Rate Limiting — FIXED (Low)

**Before:** The `gyre serve` REPL loop reads stdin synchronously with `read_line()`,
then `await`s `process_message()`. This is inherently sequential — only one LLM call
is in-flight at any time. However, `CognitiveChannelBridge::process_message()` is a
public API that could be called concurrently by future callers.

**Fix:** Added a `tokio::sync::Semaphore(1)` to `CognitiveChannelBridge` that limits
concurrent `process_message()` calls to 1. The permit is acquired at the start of
`process_message()` and released automatically when the method returns. This provides
defense-in-depth for any future multi-caller usage patterns.

### 6. Shutdown Memory Snapshot Atomicity — PASS

`HermitBox::write_memory_summary()` already uses the atomic `.tmp → rename` pattern
established in Phase 6:

```rust
let tmp = target.with_extension("md.tmp");
std::fs::write(&tmp, content)?;
std::fs::rename(&tmp, &target)?;
```

The `gyre serve` shutdown path calls `agent.hermit_box.write_memory_summary()` directly,
not a raw `std::fs::write`. No fix needed.

### 7. SQL Injection in ResearchQueue — PASS

All queries in `ResearchQueue::push()`, `pop_next()`, `peek()`, `mark_done()`, and
`mark_failed()` use rusqlite's `params![]` macro with positional parameters. The
`formatted_task` string (`[FROM:agent] task`) is bound as a parameter, not interpolated.

### 8. Identity File Size — PASS (Existing)

`HermitBox` already caps identity files (`soul.md`, `user.md`, `memory.md`) to 50KB
via `read_capped()` (established in Phase 6). The cognitive turn prefix has its own
8KB cap in `turn.rs`. Both are adequate.

---

## Summary

| # | Finding | Severity | Status |
|---|---------|----------|--------|
| 1 | A2A sender spoofing (no `from_agent` validation) | Critical | FIXED |
| 2 | A2A agent ID format injection (`]` in IDs) | Critical | FIXED |
| 3 | Task injection via `gyre send --task` (silent truncation) | Medium | FIXED |
| 4 | Combined system prompt bloat (no total cap) | Medium | FIXED |
| 5 | REPL rate limiting (public API concurrency) | Low | FIXED |
| 6 | Shutdown memory snapshot atomicity | N/A | PASS |
| 7 | SQL injection in ResearchQueue | N/A | PASS |
| 8 | Identity file size caps | N/A | PASS |

**Files modified:**
- `src/cognitive/a2a.rs` — sender validation, agent ID sanitization, 7 new tests
- `src/cognitive/channel_bridge.rs` — combined system prompt cap (16KB), Semaphore(1) rate limit
- `src/cli/send.rs` — explicit task length cap with error message

**Tests:** 7 new tests, all passing. 12 existing curiosity tests still passing.
