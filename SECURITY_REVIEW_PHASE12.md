# Security Review: Phase 12 (Teagan)

**Date:** 2026-02-18
**Commit:** dfafbc8 (feat: Phase 12 - cognitive+identity wired into turn pipeline)
**Reviewer:** Teagan (security specialist)

---

## 1. Cognitive Injection Ordering

**Status:** PASS

The system prompt is assembled in `dispatcher.rs:102-133` in this order:
1. **Agent identity** (`AgentIdentityFiles::system_prompt_block()`) - soul, user context, memory summary
2. **Cognitive prefix** (`format_cognitive_prefix()`) - recent memories, knowledge graph, axioms
3. **Workspace identity** (AGENTS.md, SOUL.md, etc.)

Parts are joined with `\n\n---\n\n` separators.

**Could a crafted cognitive prefix override the original system prompt?**

No. The cognitive prefix is inserted as a *middle* block between identity and workspace content. It uses `### Recent Memories`, `### Knowledge`, and `### Guiding Axioms` headers under a `## Cognitive Context` parent. The identity block uses `## Agent Identity` with `### Soul`, `### User Context` sub-headers. These are structurally distinct and the LLM sees them as separate sections.

The cognitive prefix has an 8 KB size cap (`MAX_COGNITIVE_PREFIX_BYTES` in `turn.rs:8`) that truncates with an ellipsis marker, preventing oversized memory content from consuming the full context window. Identity files are capped at 50 KB each (`MAX_IDENTITY_FILE_BYTES` in `hermit_box.rs:9`).

**Residual note:** Memory entries stored via `auto_store_memories` are extracted from assistant responses, not raw user input, so prompt injection via memory is limited to what the LLM itself produces. However, if an attacker controls user input that the LLM echoes verbatim into a memory, that memory could appear in future cognitive prefixes. This is a known limitation of any memory system - mitigated by the 5-entry recall limit and recency weighting.

## 2. status_summary() Information Leakage

**Status:** PASS

`CognitiveAgent::status_summary()` in `cognitive/agent.rs` exposes:
- `agent:{id}` - the agent identifier (alphanumeric, already public in CLI args)
- `memories:{count}` - integer count only, no content
- `queue:{pending}` - integer count of pending research tasks

No memory content, file paths, or internal state is leaked. The SSE `AgentStatus` event carries the same three fields. The REPL only prints this under `debug` mode.

## 3. --box/--agent Path Validation

**Status:** PASS

`serve.rs` defines its own `validate_base_dir()` (lines 25-67) with the same `BLOCKED_PATH_PREFIXES` as `cognitive_run.rs`, `tribe.rs`, `explore.rs`, and `send.rs`:
- `/dev`, `/proc`, `/sys`, `/run`, `/var/run`

The function canonicalizes the path to resolve symlinks before checking prefixes. After `validate_base_dir`, `CognitiveAgent::open()` calls `HermitBox::open()` which:
1. Validates `agent_id` via `validate_agent_id()` (alphanumeric + hyphens/underscores, max 64 chars, no `..`, no `/` or `\`)
2. Canonicalizes `base_dir` again to prevent symlink races
3. Constructs `{agent_id}_box` under the canonical base

Phase 6 protections (agent_id sanitization, symlink resolution, blocked prefixes) are fully invoked.

**Note:** `validate_base_dir` is duplicated across 5 CLI modules. Recommend extracting to a shared utility in a future cleanup pass.

## 4. MemoryStream::count() SQL Safety

**Status:** PASS

`memory_stream.rs:249-258`:
```rust
db.query_row("SELECT COUNT(*) FROM memory_entries", [], |row| {
    row.get::<_, i64>(0)
})
.unwrap_or(0) as usize
```

- Static SQL string with no interpolation
- Empty parameter array `[]`
- No injection surface
- The `unwrap_or(0)` safely handles query errors

## 5. unwrap() in Phase 12 Code

**Status:** FIXED (2 issues)

### 5a. thread_ops.rs - `.expect("memory_stream lock")` (pre-existing, 2 sites)

**Lines 330 and 837** used `.expect()` on mutex lock, which would panic on a poisoned mutex. While mutex poisoning in practice requires a panic inside a lock guard (unlikely), the defensive pattern is preferred.

**Fix:** Replaced both with `if let Ok(ms) = cog_ctx.memory_stream.lock()` to gracefully skip cognitive post-turn processing if the mutex is poisoned, rather than crashing the agent loop.

### 5b. serve.rs - Gateway auth token not printed

The `run_serve_full_agent` function creates a `GatewayConfig` with `auth_token: None`, which causes `GatewayChannel::new()` to auto-generate a random 32-char token. However, this token was never printed to stderr, making the web gateway inaccessible.

**Fix:** Added `eprintln!("[Serve] Auth token: {}", gw.auth_token())` so the operator can see the generated token. Note: the token is only printed to stderr (not logged to files) and the gateway binds to `127.0.0.1` by default (localhost only).

## 6. Additional Observations

### 6a. SseEvent::AgentStatus serialization

The `WsServerMessage::event_type()` in `web/types.rs:548` uses `serde_json::to_value(event).unwrap_or(serde_json::Value::Null)` which is pre-existing and safe for the `AgentStatus` variant (only contains a String and two usize fields, always serializable).

### 6b. WASM channel wrapper

`StatusUpdate::AgentStatus` is mapped to `wit_channel::StatusType::Thinking` in `wasm/wrapper.rs`. The message format string uses `format!("agent:{} memories:{} queue:{}", ...)` which correctly prevents injection since `agent_id` has already been validated to be alphanumeric.

### 6c. Telegram flag uses `_telegram` prefix

The `_telegram` parameter in `run_serve_full_agent` correctly emits a warning message rather than silently ignoring the flag. No security issue, but the underscore prefix is intentional to suppress unused-variable warnings.

---

## Summary

| Check | Status | Action |
|-------|--------|--------|
| Cognitive injection ordering | PASS | No override possible; size-capped |
| status_summary() leakage | PASS | Counts only, no content |
| Path validation (--box) | PASS | Phase 6 protections invoked |
| MemoryStream::count() SQL | PASS | Parameterized, no injection |
| unwrap() in new code | FIXED | 2x expect() -> if let Ok() |
| Auth token visibility | FIXED | Token now printed to stderr |
