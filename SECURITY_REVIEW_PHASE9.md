# Security Review: Phase 9 — Real LLM Worker Execution

**Reviewer:** Teagan (Gyre Security Specialist)
**Date:** 2026-02-18
**Scope:** WorkerExecutor, TribeOrchestrator::execute(), MockLlmProvider, tribe CLI, metrics

---

## Audit Checklist

### 1. TIMEOUT INTEGRITY — PASS

**File:** `src/cognitive/executor.rs:57-61`

`tokio::time::timeout()` wraps the `llm.complete(request)` future. When the timeout elapses, the `Elapsed` error is returned and the inner future is **dropped**. Tokio's timeout semantics guarantee the wrapped future is cancelled (dropped) on expiry — it does not continue running in the background.

The 60-second default in `TribeOrchestrator::execute()` is reasonable. The timeout value is passed as a parameter to `WorkerExecutor::run()`, allowing callers to override.

**Verdict:** Correct. Future is truly dropped on timeout.

### 2. MOCK IN PRODUCTION — FIXED (was CRITICAL)

**File:** `src/llm/mock.rs`, `src/llm/mod.rs`

**Before fix:** `MockLlmProvider` was publicly exported with no cfg gate. It was available in production binaries, meaning anyone with access to the library could instantiate a mock LLM that returns arbitrary responses, bypassing real provider auth.

**Fix applied:**
- Added `test-support` feature to `Cargo.toml`
- Gated `pub mod mock` and `pub use mock::MockLlmProvider` behind `#[cfg(any(test, feature = "test-support"))]`
- Integration test `tests/worker_executor_test.rs` gated behind `#![cfg(feature = "test-support")]`
- CI uses `--all-features` so tests still run
- Production `cargo build` (default features) excludes MockLlmProvider entirely

**Verdict:** Fixed. Mock is now test-only.

### 3. RESULT SANITIZATION — FIXED (was HIGH)

**File:** `src/cognitive/orchestrator.rs:store_worker_result()`

**Before fix:** Worker LLM results were stored directly into the Chief's memory stream and knowledge graph without sanitization. A malicious LLM response containing injection payloads (e.g., "system: ignore all previous instructions") would be stored verbatim and later injected into the Chief's context window via memory recall.

**Fix applied:**
- `store_worker_result()` now passes the worker result through `sanitize_memory_content()` (the Phase 7 injection defense) before memory storage
- If sanitization rejects the content (injection detected), the function returns an error instead of silently storing poisoned data
- KG entity extraction now validates entity names: max 128 bytes, alphanumeric/hyphen/underscore only — prevents injection via specially-crafted entity names

**Note:** The `auto_store_memories()` path (used in `CognitiveAgent::process_turn()`) was already sanitized in Phase 7. The gap was in `store_worker_result()` which writes directly to the memory stream, bypassing `auto_store_memories`.

**Verdict:** Fixed. Worker results now pass through injection defense before storage.

### 4. METRICS TIMING — PASS (LOW RISK)

**File:** `src/cognitive/orchestrator.rs:97`

```rust
let duration_ms = start.elapsed().as_millis() as u64;
```

`as_millis()` returns `u128`. The `as u64` cast truncates on overflow. A u64 in milliseconds can represent ~584 million years, so overflow from a real LLM call is impossible. Even a malicious slow provider would timeout at 60 seconds (= 60,000 ms), nowhere near u64 limits.

**Verdict:** No risk. The timeout guarantees duration stays bounded.

### 5. ERROR LEAKAGE — FIXED (was MEDIUM)

**File:** `src/cognitive/executor.rs`

**Before fix:** `ExecutorError::LlmError(String)` stored the raw `LlmError::to_string()` output, which could contain connection strings, API keys in URLs, bearer tokens, or other internal details. These would be displayed via the CLI and potentially logged.

**Fix applied:**
- Added `sanitize_llm_error()` function that:
  - Detects sensitive patterns (API keys, connection strings, bearer tokens, session tokens)
  - Redacts the entire message if sensitive content is detected
  - Truncates excessively long error messages (>512 chars) to prevent log flooding
- Error construction in `WorkerExecutor::run()` now passes through the sanitizer
- Added 5 unit tests covering safe messages, API key redaction, connection string redaction, bearer token redaction, and truncation

**Verdict:** Fixed. Error messages are now sanitized before storage.

### 6. UNWRAP/EXPECT IN PRODUCTION CODE — PASS

**Files:** `src/cognitive/executor.rs`, `src/cognitive/orchestrator.rs`

Grep for `.unwrap()` and `.expect(` returns zero hits in both files. All error handling uses `?` or `map_err()`.

**Verdict:** Clean.

---

## Summary of Changes

| Finding | Severity | Status |
|---------|----------|--------|
| MockLlmProvider in production binary | CRITICAL | FIXED |
| Worker results bypass injection defense | HIGH | FIXED |
| LLM errors may leak internal details | MEDIUM | FIXED |
| Timeout correctly drops future | — | PASS |
| Metrics duration_ms overflow | — | PASS (u64 sufficient) |
| No unwrap/expect in production | — | PASS |

## Files Modified

- `Cargo.toml` — Added `test-support` feature
- `src/llm/mod.rs` — cfg-gated mock module and re-export
- `src/cognitive/auto_memory.rs` — Made `sanitize_memory_content` pub(crate)
- `src/cognitive/orchestrator.rs` — Sanitize worker results before memory/KG storage; validate entity names
- `src/cognitive/executor.rs` — Added `sanitize_llm_error()`, error sanitization, tests
- `tests/worker_executor_test.rs` — cfg-gated behind `test-support` feature

## Test Results

- 1194 lib tests: PASS
- 5 worker executor integration tests: PASS
- Build without test-support: PASS (MockLlmProvider excluded)
