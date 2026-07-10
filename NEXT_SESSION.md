# Next Session Handoff — written 2026-07-10, end of Tier 1+2 sprint

**State:** branch `fresh-start`, 16 commits ahead of `bcd7d62` (beta.1). `cargo test --lib`: **1,350 passed / 0 failed**. Clippy clean on all sprint-touched code. Two adversarial review passes (correctness + design) applied — findings fixed in `49ea6dd` and `e303820`.

## What landed this sprint (do not redo)

| Item | Commit | Notes |
|---|---|---|
| Test suite compile fix + fixtures | `96d1d0e` | |
| Sprint plan + Lyzr competitive analysis (Rev 1–3) | `9d6c8a6`, `5a3bf70`, `edd7627` | `SPRINT_PLAN_2026-07-10.md`, `COMPETITIVE-lyzr-2026-07-10.md` |
| 1.1 fallback survives explicit model | `c031ae6` | `set_model` → primary only |
| 1.5 per-job toolsAllow (restrict-only) | `43dc827` | deny-all = env `","`, NOT empty string |
| 1.2 typed routine delivery + structured check-ins | `cc78ff8` | `RoutineNotification`, `src/agent/attention.rs` |
| 1.3 cross-channel approvals + real channel persisted | `0c0a7c8` | hardened in `49ea6dd`: cross-channel needs request_id |
| 1.4 additive hook tool-trust | `1e16fc9` | 3 approval gates wired |
| 1.6 sessions_send/sessions_list | `d61167d` | loud failure, no queue |
| 2.4 memory auto-recall + brain-pipeline blueprint | `e8684fb` | `MEMORY_AUTO_RECALL_TOP_K` (default 3) |
| 2.1/2.3 fan-out + novelty-gate blueprints | `78db2c3` | crons fixed in `e303820` (6-field!) |
| routine_test (dry-run + LLM judge) | `eda63fa` | fails closed on judge parse |
| 2.2 EgressPolicy design | `716c057` | `docs/design/egress-policy.md` |

## Pickup order for the follow-on session

1. **EgressPolicy implementation** (~300–500 LOC, the security launch-blocker). Complete design in `docs/design/egress-policy.md`. Sequence inside it: observe-mode audit first (new `egress_events` via the `Database` trait — **both backends**, postgres + libsql, per CLAUDE.md), then enforce, then LLM judge. Integration points listed in the doc.
2. **FullJob → Scheduler integration** (~150–250 LOC). Exact plan in `examples/brain-pipeline/IMPLEMENTATION_NOTES.md`. Unblocks autonomous execution of both tool-using blueprints (currently degraded to tool-less lightweight — honestly documented in each SKILL.md).
3. **Tier 3 release CI**: push tag `v0.3.0-beta.1`, watch cargo-dist in `release.yml`, fix what breaks (LAUNCH_PLAN.md Tier 3 — "everything else gates on this").
4. Smaller carried items: persist per-job allowed_tools in `SandboxJobRecord` (restart path falls back to defaults — comment in `src/channels/web/server.rs` restart handler); `gyre blueprint test` full simulation harness (v0.4, per SPRINT_PLAN Rev 3).

## Traps for the next implementer

- **Deny-all tool allowlist must be emitted as `","`** — an empty env var reads as *unset* via `optional_env()` and silently grants the FULL default toolset in-container. Contract pinned by tests on both sides.
- **Cron schedules in blueprints are seconds-first 6/7-field** (`cron` crate). 5-field UNIX syntax parses as an error and the routine *never fires, silently*.
- **Cross-channel approval requires request_id by design** (security — see `49ea6dd`). Don't "fix" a bare cross-channel "yes" back in.
- Structured check-ins: attention parsing fails **open** (notify), readiness verdicts fail **closed** (not ready). Both directions are deliberate and tested.
- `cargo target/` grew to 28GB and filled the disk mid-sprint; `cargo clean` if builds start failing with ENOSPC.
- Greg's `LAUNCH_PLAN.md` working-tree edit is intentionally uncommitted — leave it.

## Kickoff prompt for next session

> Read `NEXT_SESSION.md`, `docs/design/egress-policy.md`, and `examples/brain-pipeline/IMPLEMENTATION_NOTES.md` in `/Users/yogibear/kimi/gyre-rust/`. Confirm `cargo test --lib` is green, then implement EgressPolicy per the design (observe mode first, commit per phase), then FullJob→Scheduler. Bitter Lesson rules apply (`kimi/reports/bitter-lesson-upgrade-plan-2026-07-03.md` §5).
