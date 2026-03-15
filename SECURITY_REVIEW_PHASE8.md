# Security Review: Phase 8 — Tribe Orchestration

**Reviewer:** Teagan (security specialist)
**Date:** 2026-02-18
**Commit under review:** 566a625 (Phase 8)
**Scope:** TribeOrchestrator, store_worker_result, upsert_by_name, CLI tribe/agents subcommands

---

## Checklist Results

### 1. TRIBE CONTEXT LEAKAGE — PASS

**File:** `src/cognitive/distillation.rs:60`

`distill_for_worker()` calls `recall_in_namespace(namespaces::TRIBE, 5)` which queries `WHERE namespace = 'tribe'`. Personal-namespace memories are never included in the `TribeContext` passed to Workers.

The `recall_in_namespace()` method in `memory_stream.rs:201-229` uses parameterized SQL with the namespace as a bind parameter. No cross-namespace leakage is possible.

**Verdict:** Tribe isolation is correctly enforced.

### 2. SQL INJECTION in `upsert_by_name()` — PASS

**File:** `src/cognitive/knowledge_graph.rs:194-217`

All SQL in `upsert_by_name()` uses `params![]` macro with `?1` through `?7` positional placeholders. Entity names from Worker results pass through rusqlite's parameter binding, which handles escaping. No string interpolation or format!() in SQL strings.

Same pattern confirmed in `upsert_entity()`, `add_edge()`, `search_by_name()`, and `activate()`.

**Verdict:** No SQL injection risk. All queries are parameterized.

### 3. JOB ID INTEGRITY — PASS

**File:** `src/cognitive/orchestrator.rs:44-45, 78-81`

- `job_id` is generated server-side via `Uuid::new_v4().to_string()` in `prepare_job()`. Workers never provide or influence the job ID.
- `store_worker_result()` guards on `WorkerJobStatus::Completed` at line 78-81. Only completed jobs have their results stored. Pending, Running, and Failed statuses are rejected with an error.

**Verdict:** No spoofing or replay risk. Job ID is server-generated; status is gated.

### 4. KG ENTITY FLOODING — FIXED

**File:** `src/cognitive/orchestrator.rs:101-109`

**Before:** Entity extraction used `.take(5)` — only 5 entities per result. While this was a cap, it was below the recommended 10 and lacked a scan-size limit on the input text.

**After (fix applied):**
- Added `MAX_SCAN_BYTES = 4096`: Only the first 4 KB of result text is scanned for entity extraction. This prevents CPU waste when a Worker returns very large results.
- Bumped entity cap to `MAX_ENTITIES_PER_RESULT = 10`: Allows richer knowledge extraction while still bounding growth.
- Character boundary handling ensures no panic on multi-byte UTF-8.

**Verdict:** Fixed. Dual cap (scan size + entity count) prevents flooding.

### 5. AGENTS CLI SYMLINK CHECK — FIXED

**File:** `src/cli/agents.rs:24-34`

**Before:** `entry.path().is_dir()` follows symlinks. A `*_box` symlink pointing outside the base directory would cause the CLI to open and display data from the symlink target.

**After (fix applied):**
- Added `entry.file_type().map(|ft| ft.is_symlink())` check before processing. Symlinks are skipped entirely.
- Note: `DirEntry::file_type()` does NOT follow symlinks (unlike `Path::is_dir()`), so this correctly detects symlink entries.

**Verdict:** Fixed. Symlinks are now rejected.

### 6. UNWRAP PANICS — PASS

**Files scanned:**
- `src/cognitive/orchestrator.rs`
- `src/cognitive/distillation.rs`
- `src/cognitive/knowledge_graph.rs` (upsert_by_name only)
- `src/cli/tribe.rs`
- `src/cli/agents.rs`

No `.unwrap()` or `.expect()` calls found in any Phase 8 code. All fallible operations use `.map_err()`, `.ok()`, `.unwrap_or_default()`, or `?` propagation.

**Verdict:** No panic risk from unwrap/expect.

---

## Additional Observations

### Entity extraction quality (informational, not a security issue)

The entity extraction in `store_worker_result()` uses a naive heuristic: any whitespace-delimited word >5 characters becomes a KG entity. This means punctuation-attached words (e.g., `"hello,"`) and non-meaningful long words get inserted. Future phases should consider:
- Stripping punctuation before entity extraction
- Using NLP-based named entity recognition
- Deduplicating against existing entities by normalized name

### Tribe CLI path validation (confirmed)

`src/cli/tribe.rs` reuses the same `BLOCKED_PATH_PREFIXES` and `validate_base_dir()` pattern from Phase 7's cognitive_run. Path validation is consistent across CLI entry points.

---

## Summary

| Check | Result | Action |
|-------|--------|--------|
| Tribe context leakage | PASS | None |
| SQL injection (upsert_by_name) | PASS | None |
| Job ID integrity | PASS | None |
| KG entity flooding | FIXED | Scan cap (4KB) + entity cap (10) |
| Agents CLI symlink | FIXED | Symlink entries skipped |
| Unwrap panics | PASS | None |
