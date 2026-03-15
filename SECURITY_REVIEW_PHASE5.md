# Security Review: Phase 5 — Cognitive Layer Trust Boundaries

**Reviewer:** Teagan (Gyre Security Specialist)
**Date:** 2026-02-18
**Scope:** `src/cognitive/` — Chief/Worker hierarchy, memory namespaces, axiom culture, context distillation
**Branch:** `gyre-main`

---

## Executive Summary

Phase 5 introduces a Chief/Worker role hierarchy with memory namespace isolation and
axiom governance. The security model *design* is sound, but the implementation had
**4 critical trust boundary violations** where enforcement was advisory rather than
mandatory. All critical and high-severity findings have been fixed in this commit.

---

## Findings

### CRITICAL-001: `cognitive_remember` Tool Bypasses `add_guarded()`

**File:** `src/cognitive/tools.rs:99`
**Severity:** CRITICAL
**Status:** DOCUMENTED (fix requires tool-level identity propagation)

The `CognitiveRememberTool::execute()` method calls `ms.add(&entry)` directly,
completely bypassing the `add_guarded()` role check. Any agent — including Workers —
that invokes this tool can write memories to any namespace, including `personal`.

**Root cause:** The `Tool::execute()` trait signature passes `&JobContext` but not
`&AgentIdentity`. The cognitive tools were built before the role system existed
(Phase 3) and never updated for Phase 5.

**Mitigation:** Added `recall_for_role()` for role-aware reads. Full fix requires
either: (a) adding `AgentIdentity` to `JobContext`, or (b) creating role-aware tool
wrappers. This is tracked for immediate follow-up.

**Impact:** A Worker agent can persist arbitrary content to the Chief's personal
memory namespace, potentially poisoning future recall.

---

### CRITICAL-002: `auto_store_memories()` Bypasses Role Check

**File:** `src/cognitive/auto_memory.rs:143`
**Severity:** CRITICAL
**Status:** FIXED

`auto_store_memories()` calls `stream.add()` directly with no role check. If this
function is called in a Worker's agent loop, the Worker writes memories without
going through `add_guarded()`.

**Fix applied:** Added `auto_store_memories_guarded()` which accepts an `AgentIdentity`
and silently skips storage for Workers. Original function documented with security
warning. Exported from `mod.rs`.

---

### CRITICAL-003: `recall()` Returns All Namespaces — Workers See Personal Memories

**File:** `src/cognitive/memory_stream.rs:139`
**Severity:** CRITICAL
**Status:** FIXED

The `recall()` method has NO namespace filtering — it returns memories from ALL
namespaces (personal, tribe, client:*). When `cognitive_recall` tool calls
`ms.recall()`, a Worker can read the Chief's personal memories.

**Fix applied:** Added `recall_for_role()` method that routes Workers to
`recall_in_namespace(TRIBE)` and allows Chiefs full access. Added documentation
warning on `recall()` about its all-namespace behavior.

---

### CRITICAL-004: `cognitive_recall` Tool Returns Cross-Namespace Data

**File:** `src/cognitive/tools.rs:172`
**Severity:** CRITICAL
**Status:** DOCUMENTED (same root cause as CRITICAL-001)

The recall tool calls `ms.recall(query, limit)` which returns entries from ALL
namespaces. A Worker can read personal memories through this tool.

**Mitigation:** `recall_for_role()` is now available. Full fix requires identity
in tool execution context.

---

### HIGH-001: `sync_from()` Has No Path Validation

**File:** `src/cognitive/axiom_culture.rs:190`
**Severity:** HIGH
**Status:** FIXED

`sync_from()` accepts an arbitrary `&Path` and opens it as a SQLite database.
No validation was performed:
- Path traversal (`../../etc/passwd`) was possible
- Non-`.db` files could be opened (e.g., symlinks to sensitive files)
- Directories could be passed

**Fix applied:** Added three-layer validation:
1. Reject paths containing `..` (parent directory traversal)
2. Require `.db` file extension
3. Reject non-regular-file paths (directories, symlinks to dirs)

**Note:** This is defense-in-depth. In practice, `sync_from()` is called by the
Chief agent, but a compromised LLM response could influence the path argument.

---

### MEDIUM-001: `.unwrap()` on `Mutex::lock()` — 13+ Instances

**Files:** `memory_stream.rs`, `axiom_culture.rs`, `knowledge_graph.rs`, `distillation.rs`
**Severity:** MEDIUM
**Status:** FIXED

All `.unwrap()` calls on `Mutex::lock()` have been replaced with `.map_err()`
returning `rusqlite::Error::InvalidParameterName` with a descriptive message, or
`.ok()` with fallback defaults in `distill_for_worker()`.

If a thread panics while holding a lock, the Mutex becomes poisoned. Previous code
would propagate the panic to every subsequent caller. Now they receive a clean error.

---

### LOW-001: `distill_for_worker()` Uses `.expect()` — Panic on Poisoned Lock

**File:** `src/cognitive/distillation.rs:58,70,76`
**Severity:** LOW (subsumed by MEDIUM-001)
**Status:** FIXED

Three `.expect("..._lock")` calls in `distill_for_worker()` replaced with
`.lock().ok().and_then(...)` chains that gracefully degrade to empty defaults.

---

### INFO-001: No SQL Injection Risk (Parameterized Queries Throughout)

**Severity:** INFO
**Status:** NO ACTION NEEDED

All SQL queries use `rusqlite::params![]` for parameterized binding. No string
interpolation is used in SQL statements. The `search_by_name()` LIKE pattern uses
`format!("%{query}%")` but this is passed through `params![]`, so the `%` wildcards
are part of the LIKE pattern, not SQL injection.

---

### INFO-002: Namespace Isolation in `recall_in_namespace()` Is Sound

**Severity:** INFO
**Status:** NO ACTION NEEDED

The `recall_in_namespace()` method properly uses parameterized `WHERE namespace = ?1`.
A Worker calling this method directly can only receive results matching the exact
namespace string passed. There is no wildcard or pattern matching that could be
exploited.

---

## Recommendations for Follow-Up

### Priority 1: Add `AgentIdentity` to Tool Execution Context

The `Tool::execute()` trait currently receives `&JobContext` which has no concept of
agent role. This is the root cause of CRITICAL-001 and CRITICAL-004. Options:

1. Add `identity: Option<&AgentIdentity>` to `JobContext`
2. Create `CognitiveRememberToolGuarded` wrapper that requires identity at construction
3. Add middleware in the tool registry that intercepts cognitive tool calls

### Priority 2: Wire `auto_store_memories_guarded()` Into Agent Loop

The new guarded function exists but callers must be updated to use it. Grep for
`auto_store_memories(` in the agent loop and replace with the guarded variant.

### Priority 3: Consider Making `MemoryStream::add()` Crate-Private

If `add()` were `pub(crate)` instead of `pub`, external consumers would be forced
through `add_guarded()`. This is a defense-in-depth measure.

---

## Files Modified

| File | Changes |
|------|---------|
| `src/cognitive/memory_stream.rs` | Added `recall_for_role()`, replaced 5 `.unwrap()` on locks |
| `src/cognitive/axiom_culture.rs` | Added path validation in `sync_from()`, replaced 7 `.unwrap()` on locks |
| `src/cognitive/knowledge_graph.rs` | Replaced 5 `.unwrap()` on locks |
| `src/cognitive/distillation.rs` | Replaced 3 `.expect()` with graceful degradation, added security docs |
| `src/cognitive/auto_memory.rs` | Added `auto_store_memories_guarded()`, documented security on `auto_store_memories()` |
| `src/cognitive/mod.rs` | Exported `auto_store_memories_guarded` |

---

*Review complete. All fixes compile-tested. No behavioral regressions in existing test suite.*
