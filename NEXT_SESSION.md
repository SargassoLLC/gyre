# Next Session Handoff — written 2026-07-14, end of launch sprint
# (updated same day, end of follow-on orchestration wave)

**State:** branch `fresh-start`, pushed to public `github.com/SargassoLLC/gyre`.
Tag `v0.3.0-beta.1` cut. `cargo test --lib`: **1,379 passed / 0 failed** (default,
libsql-only, and all-features builds clean). One adversarial review pass applied
(verdict SHIP-WITH-FIXES; all 5 findings fixed in `f3caefb`).

## What landed this session (do not redo)

| Item | Commits | Notes |
|---|---|---|
| 2.2 EgressPolicy observe/enforce/judge | `0f54805`, `59a0d31`, `7fe6a6c` | `src/safety/egress.rs`; `egress_events` on BOTH backends; `[egress]` config + `EGRESS_*` env; judge fails closed everywhere |
| WASM allowlist → same audit log | `0d79e70` | auditor rides on ToolRegistry; every WASM registration path inherits it |
| FullJob → Scheduler | `21268e1` | routines run real tool-using jobs; `TrustedToolsHook` (memory tools); `job_id` on RoutineRun; typed-notification delivery |
| Review fixes | `f3caefb` | full_job hard deadline (`ROUTINES_FULL_JOB_TIMEOUT_SECS`); unrepairable Stuck→Failed; http userinfo reject; HttpTool::new()→Result |
| Release prep | `f89690b`..`600678b` | version 0.3.0-beta.1; CHANGELOG; repo URLs → SargassoLLC/gyre (sac916/gyre is PRIVATE); Windows E0283 fix; wix regen |

## Done in the follow-on wave (2026-07-14 evening — do not redo)

- `fresh-start` merged → `main`; both branches current on the remote.
- Tier 4 site fixes deployed to getgyre.com (commands, repo links, GitHub nav
  link, real /docs from QUICKSTART.md). Site repo: `~/kimi/getgyre-site`.
- BOTH installers (repo `install.sh` + site `public/install.sh`) fixed and
  verified end-to-end against the live release: `/releases/latest` excludes
  pre-releases (betas!), so both resolve via `/releases?per_page=1`; dist
  archives nest the binary under `gyre-<target>/`.
- Release pipeline consolidated on cargo-dist 0.32.0; `release-gyre.yml`
  retired (it raced dist's host job). Homebrew formula publishing configured
  to `SargassoLLC/homebrew-gyre` (repo created).
- `gyre egress log` CLI + `GET /api/egress/events` + gateway Egress tab.
- Per-job `allowed_tools` persisted in `SandboxJobRecord` (both backends,
  V10 migration + libsql incremental column); restart path preserves the
  deny-all sentinel exactly.
- SIGABRT flake root-caused: tokio's SIGCHLD handling overflows macOS Mach
  ports when parallel tests spawn+kill children. Process-spawning tests now
  serialize on `test_helpers::PROC_MUTEX` (current_thread flavor). Residual
  ~3% from 295 multi-thread tokio runtimes — if CI still flakes, use
  `--test-threads=4`.
- `brain_verify` gate restored: `~/.local/bin/brain_verify` wraps
  `sargasso-brain pool verify` (the capability existed; the CLI never did).
  Hook output contract is compact JSON — the hook greps `"passed":true`.

## Pickup order

1. **Set `HOMEBREW_TAP_TOKEN` secret on SargassoLLC/gyre** (Greg: PAT with
   push access to SargassoLLC/homebrew-gyre) — the homebrew publish job
   fails without it. Then the next tag exercises the whole consolidated
   pipeline end-to-end, including `brew install SargassoLLC/gyre/gyre`.
2. **Demo GIFs** — two placeholder slots on the site (hero + live demo).
   Needs an interactive `gyre setup` → `gyre run` recording session.
3. **Tier 5** (LAUNCH_PLAN.md): waitlist form, "Why we built Gyre" blog
   post (copy exists on site), #announcements channel.
4. `examples/crabtrap-proxy/` compose file (egress design doc, still pending).
5. `gyre blueprint test` full simulation harness (v0.4, SPRINT_PLAN Rev 3).

## Traps for the next implementer

- **brain_verify wrapper contract:** the pre-push hook greps for compact
  `"passed":true` — any reformat of the wrapper's JSON output re-breaks the
  gate silently. The underlying `sargasso-brain pool verify` exits 0 even on
  FAIL; the wrapper keys on output text, not exit code.
- **cargo-dist validates checked-in generated files** (`wix/main.wxs`,
  `release.yml`) against Cargo.toml config — any change to `repository`,
  installers, or targets requires regenerating them (`dist init`) or the plan
  job fails. It normalizes line endings but not content.
- **Windows target only:** `encode_unicode` (via console/dialoguer) makes bare
  `rng.gen()` ambiguous for `Vec<u8>` collects — always annotate.
- **Disk:** the main target/ plus one agent-worktree target/ can fill the
  drive mid-session (hit ENOSPC twice on 2026-07-14). Delete worktree
  target/ dirs as soon as their commits merge; `cargo clean` if tight.
- Stale local tags v0.3.0–v0.5.0 (Feb, pre-restart) exist locally but NOT on
  the remote. Never `git push --tags` from this clone.
- Prior traps still apply: deny-all toolsAllow = `","`; 6/7-field seconds-first
  cron; cross-channel approvals need request_id; attention fails open /
  readiness fails closed.
