# Security Review: Phase 6 — HermitBox, Identity, CognitiveContext

**Reviewer:** Teagan (Gyre security specialist)
**Date:** 2026-02-18
**Commit under review:** `be00cea` (Phase 6 — HermitBox persistence, AgentIdentityFiles, etc.)
**Files reviewed:**
- `src/cognitive/hermit_box.rs`
- `src/cognitive/identity.rs`
- `src/cognitive/context.rs`

---

## 1. agent_id Validation — Path Traversal

**Status:** PASS (already solid)

`validate_agent_id()` uses an **allowlist** approach: only `[a-zA-Z0-9_-]` are permitted, max 64 chars, empty rejected. This is the correct strategy — it implicitly blocks:

| Vector | Blocked by |
|--------|-----------|
| `..` | Explicit `..` check + allowlist (`.` not in charset) |
| `../evil` | Allowlist rejects `.` and `/` |
| `./x` | Allowlist rejects `.` |
| `x/y` | Explicit `/` check + allowlist |
| `x\y` | Explicit `\` check + allowlist |
| `x\x00y` (null byte) | Allowlist rejects `\0` |
| 65+ char strings | Explicit length check (max 64) |
| Empty string | Explicit empty check |
| `.` or `..` alone | Allowlist rejects `.` |

**Note:** The explicit `..` and `/`/`\` checks (lines 30-34) are technically redundant given the allowlist on lines 37-44, but they provide defense-in-depth and clearer error messages. No change needed.

---

## 2. Box Isolation — Symlink Attack

**Status:** FIXED (was vulnerable)

**Before:** `HermitBox::open()` used `base_dir` as-is without resolving symlinks. If an attacker could create a symlink at the base_dir path pointing elsewhere (e.g., `/etc/` or another user's home), the box would be created outside the intended storage tree.

**Attack scenario:**
```
ln -s /tmp/evil /data/gyre/hermit_boxes
HermitBox::open("/data/gyre/hermit_bases", "kimi")
# Creates /tmp/evil/kimi_box/ — outside intended storage
```

**Fix applied:** Added `base_dir.canonicalize()` as the first operation in `HermitBox::open()`. This resolves all symlinks to their real paths, so the `box_dir` always resolves to a real filesystem location. If `base_dir` doesn't exist yet, `canonicalize()` returns an error (correct behavior — caller must ensure base exists).

**Residual risk:** LOW. After canonicalization, the agent_id allowlist ensures the child directory name is safe. A TOCTOU window exists (symlink created between canonicalize and create_dir_all), but this requires local filesystem access which is outside our threat model for agent isolation.

---

## 3. Identity Injection — Size Cap

**Status:** FIXED (was vulnerable to memory exhaustion)

**Before:** `read_soul()`, `read_user()`, `read_memory_summary()` all used `std::fs::read_to_string()` with no size limit. A malicious or corrupted `soul.md` file of arbitrary size (e.g., 500MB) would be read entirely into memory and then injected into the LLM system prompt.

**Impact:**
- Memory exhaustion / OOM
- Excessive token consumption at the LLM API
- Potential denial of service

**Fix applied:** Introduced `read_capped()` helper using `File::take(MAX_IDENTITY_FILE_BYTES)` with a 50KB cap (`50 * 1024` bytes). Files exceeding this limit are silently truncated. Missing files return empty string (existing behavior preserved). Non-UTF-8 files return empty string (graceful degradation).

**Why 50KB:** A 50KB soul.md is approximately 12,500 words — far more than any reasonable identity document. At ~1.3 tokens/word, this is ~16K tokens, which is manageable within context windows but still substantial enough for rich identity.

---

## 4. Atomic Write Pattern

**Status:** PASS (already implemented correctly)

`write_memory_summary()` correctly uses the `.tmp → rename` pattern:
1. Write to `memory.md.tmp`
2. Atomic `rename()` to `memory.md`

This prevents partial writes from corrupting the file if the process crashes mid-write. On POSIX filesystems, `rename()` is atomic within the same directory and filesystem, which is the case here (both files are in `box_dir`).

**Note:** `soul.md` and `user.md` have no write methods on `HermitBox` — they're read-only from the agent's perspective (written by setup/admin tooling). No fix needed.

---

## 5. Panic Safety — `.expect()` on Mutex Locks

**Status:** FIXED (3 instances replaced)

**Before:** Three methods used `.expect("...lock")` on `Mutex::lock()`:
- `remember()` — line 122
- `recall()` — line 135
- `axiom_context()` — line 141

If any thread panics while holding one of these locks, the mutex becomes **poisoned**, and subsequent `.expect()` calls would **panic the entire process** — cascading a single thread's failure into a full crash.

**Fix applied:** Replaced all three with `.map_err(|_| rusqlite::Error::InvalidParameterName("...lock poisoned".into()))?` which converts the poisoned lock into a recoverable `rusqlite::Error`. Callers already handle `rusqlite::Result`, so this fits the existing error contract.

**Note:** `identity.rs` and `context.rs` contain no `.expect()` or `.unwrap()` calls — clean.

---

## 6. Additional Observations

### CognitiveContext::from_hermit_box — Struct Cloning

`from_hermit_box(&HermitBox)` reconstructs a new `HermitBox` by cloning fields. This is safe (Arc clones are cheap, PathBuf clone is a heap alloc) but architecturally awkward — the comment in the code acknowledges this. The `from_hermit_box_arc(Arc<HermitBox>)` variant is the preferred API. No security issue, but callers should migrate to the Arc variant.

### identity.rs — No Sanitization of Identity Content

`system_prompt_block()` injects soul/user/memory content directly into the system prompt with markdown headers. There is **no sanitization** of the content itself. If `soul.md` contains prompt injection payloads (e.g., "Ignore all previous instructions..."), these will be injected verbatim.

**Risk:** LOW for self-hosted single-user deployment. The identity files are written by the user/admin, not by untrusted agents. However, if the memory summary is auto-generated from agent output (which it can be via `write_memory_summary`), there's a theoretical injection path: agent convinces system to write malicious content to `memory.md`, which is then injected into future system prompts.

**Recommendation:** Future work should consider wrapping identity content in `<identity>` XML tags with a preamble instructing the LLM to treat the content as data, not instructions. Not critical for Phase 6.

---

## Summary

| Finding | Severity | Status |
|---------|----------|--------|
| agent_id validation | N/A | Already solid (allowlist) |
| Symlink escape via base_dir | MEDIUM | **Fixed** — canonicalize() added |
| Identity file size — no cap | MEDIUM | **Fixed** — 50KB cap via read_capped() |
| Atomic write for memory.md | N/A | Already correct (.tmp + rename) |
| .expect() panic on poisoned mutex | LOW | **Fixed** — proper error propagation |
| Identity content injection | LOW | Noted — future hardening recommended |

**Test results:** 29 cognitive integration tests pass, 1179 library unit tests pass. Zero regressions.
