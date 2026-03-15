# Security Review — Phase 10: Curiosity Engine

**Reviewer:** Teagan (Security Specialist)
**Date:** 2026-02-18
**Scope:** `src/cognitive/curiosity.rs`, `src/cli/explore.rs`
**Commit:** Post-Phase 10 commit (`889abee`)

---

## Findings

### 1. SQL Injection — PASS

All queries in `ResearchQueue` use rusqlite's `params![]` macro with positional
parameters (`?1`, `?2`, etc.). No string interpolation in SQL. Topic, source,
and failure_reason strings are all bound as parameters.

**Verdict:** No injection vectors found.

### 2. Daily Limit Bypass via mark_failed() Re-flood — FIXED (Critical)

**Before:** `mark_failed()` set status to `"failed"` and incremented `attempts`,
but there was no upper bound. A failed task with the same topic could be
re-pushed (dedup only checks `pending`/`in_progress`), allowing unlimited
retries and potential resource exhaustion.

**Fix:** Added `MAX_ATTEMPTS = 3` constant. `mark_failed()` now reads current
attempts, increments, and if `attempts >= MAX_ATTEMPTS`, sets status to
`"exhausted"` instead of `"failed"`. Exhausted tasks are terminal — they will
never be retried or re-queued.

New `TaskStatus::Exhausted` variant added with `"exhausted"` serialization.

### 3. Background Ticker Panic Safety — FIXED (High)

**Before:** `start_curiosity_loop()` called `engine.run_cycle()` directly inside
`tokio::spawn`. If `run_cycle()` panicked (e.g., poisoned mutex, malformed data),
the entire spawned task would die silently and never recover. The curiosity
engine would be permanently dead with no indication.

**Fix:** Wrapped the `run_cycle()` future in `AssertUnwindSafe + FutureExt::catch_unwind()`.
Panics are now caught, logged at ERROR level with the panic message, and the
loop continues to the next cycle. The ticker is now resilient to transient panics.

### 4. Topic Length — FIXED (Medium)

**Before:** `push()` accepted topic strings of arbitrary length. An attacker or
misbehaving gap detector could insert multi-megabyte topics, bloating the SQLite
database and causing display issues.

**Fix:** Added `MAX_TOPIC_LEN = 500` constant. Topics are truncated to 500 bytes
on a valid UTF-8 char boundary before insertion. The `truncate_to_byte_boundary()`
helper ensures no partial multi-byte characters.

### 5. Priority Clamping — PASS

Priority is clamped via `priority.clamp(1.0, 10.0)` at line 130 of `push()`,
**before** the INSERT statement. Verified in existing test `test_priority_clamping`.

### 6. Topic Display Injection (Control Characters) — FIXED (Medium)

**Before:** `gyre explore --queue` and `gyre explore --add` printed `task.topic`
directly to the terminal via `println!`. Topics containing newlines (`\n`),
carriage returns (`\r`), tabs (`\t`), null bytes (`\0`), or ANSI escape sequences
could break table formatting or inject terminal control sequences.

**Fix:** Added `sanitize_display_topic()` which replaces all control characters
(as defined by `char::is_control()`) with spaces. Applied in three locations:
- Queue table display
- `--add` confirmation output
- Cycle report task display

---

## Tests Added

| Test | What it verifies |
|------|-----------------|
| `test_max_attempts_exhausts_task` | After MAX_ATTEMPTS failures, task status is `"exhausted"` |
| `test_topic_length_cap` | Topics > 500 bytes are truncated |
| `test_topic_truncation_unicode_boundary` | Truncation respects UTF-8 char boundaries |
| `test_sanitize_display_topic` | Control chars replaced with spaces |
| `test_task_status_roundtrip` | Updated to include `Exhausted` variant |

---

## Summary

| # | Finding | Severity | Status |
|---|---------|----------|--------|
| 1 | SQL injection | — | Pass (no issues) |
| 2 | mark_failed() re-flood / no max_attempts | Critical | Fixed |
| 3 | Ticker panic kills background loop | High | Fixed |
| 4 | Unbounded topic length | Medium | Fixed |
| 5 | Priority clamping | — | Pass (already correct) |
| 6 | Control char injection in display | Medium | Fixed |
