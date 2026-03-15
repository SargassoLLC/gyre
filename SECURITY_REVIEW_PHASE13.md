# Security Review — Phase 13 (TELOS, UOCS, Learning Loop)

**Reviewer:** Teagan (Gyre security specialist)
**Date:** 2026-02-18
**Scope:** `src/cognitive/hermit_box.rs`, `src/cognitive/uocs.rs`, `src/cognitive/learning.rs`

---

## Checklist

### 1. `append_telos_file()` name parameter validation

**Status:** PASS (pre-existing)

The `name` parameter is validated to require `.md` extension and an
alphanumeric/hyphen/underscore stem. Tested attack vectors:

| Input | Result | Why |
|-------|--------|-----|
| `../evil.md` | Err | `.` not in allowed charset |
| `x/y.md` | Err | `/` not in allowed charset |
| `x\0y.md` | Err | null byte not in allowed charset |
| `x\y.md` | Err | `\` not in allowed charset |
| `.md` | Err | empty stem |
| `MISSION.md` | Ok | valid |

**Fix applied:** Extracted `validate_telos_filename()` as a shared helper used
by both `append_telos_file()` and `read_telos_file()` (see finding #2).

### 2. `read_telos_file()` missing validation

**Status:** FIXED (was CRITICAL)

`read_telos_file(name)` joined user-supplied `name` directly to the telos
directory path with no validation. A caller could pass `../../etc/passwd` and
read arbitrary files outside the telos directory.

**Fix:** Added `validate_telos_filename()` guard — returns empty string on
invalid names, same rules as `append_telos_file()`.

### 3. `UocsWriter::write_memory()` file path safety

**Status:** PASS

File paths use `entry.id.to_string()[..8]` (UUID prefix) and
`entry.created_at.format("%Y-%m-%d")` (date). Both are generated internally,
not user-controlled. No injection vector.

### 4. INDEX.md atomic write

**Status:** PASS (pre-existing)

Both `write_memory()` (line 68-91) and `regenerate_index()` (line 101-167)
use the `.tmp` → rename pattern for atomic writes.

### 5. `search_content()` symlink following

**Status:** FIXED (was HIGH)

`walkdir::WalkDir::new()` follows symlinks by default. An attacker who can
create a symlink inside the memory directory (e.g., `memory/lessons/evil -> /etc/`)
could read arbitrary files on the filesystem via the search API.

**Fix:** Added `.follow_links(false)` and `entry.path_is_symlink()` skip guard,
matching the same pattern used in `src/cli/agents.rs` (Phase 8 fix) and
`src/skills/registry.rs`.

### 6. LearningLoop frontmatter injection

**Status:** FIXED (was MEDIUM)

`reflect()` appended `entry.content` directly into telos markdown files. A
crafted Lesson memory containing `---` lines could inject YAML frontmatter
blocks that break the telos file structure and could cause downstream parsers
to misinterpret file contents.

**Fix:** Added `sanitize_telos_content()` that strips lines consisting solely
of `---` (frontmatter markers) and null bytes. Applied to all three append
paths (EXPERIENCES.md, BELIEFS.md, GOALS.md).

### 7. UTF-8 boundary panics

**Status:** FIXED (was MEDIUM)

Two locations used byte-offset string slicing without checking char boundaries:

- `learning.rs:114` — `&m.content[..80]` in handoff preview truncation
- `uocs.rs:230` — `&trimmed[..max_summary]` in index summary extraction

Both would panic on multi-byte UTF-8 content (e.g., emoji, CJK characters).

**Fix:** Added `is_char_boundary()` scan loop before slicing, same pattern
already used in `uocs.rs:82` for content truncation.

### 8. `unwrap()` / `expect()` in production code

**Status:** PASS

Grep scan of all Phase 13 files (`hermit_box.rs`, `uocs.rs`, `learning.rs`)
found zero `unwrap()` or `expect()` calls in production code paths. All
instances are in `#[cfg(test)]` blocks only.

Note: `uocs.rs:134` uses `unwrap_or_default()` which is safe (returns empty
`OsStr` on `None`).

---

## Tests Added

14 new unit tests covering security invariants:

**hermit_box::tests** (9 tests):
- `telos_filename_rejects_path_traversal`
- `telos_filename_rejects_slashes`
- `telos_filename_rejects_null_bytes`
- `telos_filename_rejects_backslash`
- `telos_filename_rejects_dots_only`
- `telos_filename_rejects_no_extension`
- `telos_filename_accepts_valid_names`
- `agent_id_rejects_traversal`
- `agent_id_accepts_valid`

**learning::tests** (5 tests):
- `sanitize_strips_frontmatter_markers`
- `sanitize_strips_null_bytes`
- `sanitize_preserves_normal_content`
- `sanitize_strips_padded_frontmatter_markers`
- `record_turn_triggers_at_threshold`

---

## Files Changed

| File | Changes |
|------|---------|
| `src/cognitive/hermit_box.rs` | Extracted `validate_telos_filename()` helper; added validation to `read_telos_file()`; added 9 security tests |
| `src/cognitive/uocs.rs` | Added `.follow_links(false)` + symlink skip to `search_content()`; fixed UTF-8 boundary in `extract_summary()` |
| `src/cognitive/learning.rs` | Added `sanitize_telos_content()` frontmatter sanitizer; applied to all `reflect()` paths; fixed UTF-8 boundary in handoff preview; added 5 tests |

## Pre-existing Notes

- Integration tests (`tests/openai_compat_integration.rs`, `tests/workspace_integration.rs`) and examples (`examples/test_heartbeat.rs`) have pre-existing compile errors from the `gyre` → `gyre` rename. Not in scope for this review.
- Three pre-existing warnings (unused import, dead code in `llm/retry.rs`) — not Phase 13 code.
