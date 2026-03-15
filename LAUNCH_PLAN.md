# Gyre Public Launch Plan — Beta → v1.0

**Target:** May 15, 2026  
**Current state:** v0.2.0-beta.1 tagged, site live at getgyre.com  
**Goal:** Polished public launch with real install path, docs, community, and binaries

---

## Track 1 — Infrastructure

The release pipeline needs to actually produce downloadable artifacts.

| Item | Status | Notes |
|---|---|---|
| GitHub Actions binary releases | ⬜ | `cargo-dist` wired in `release.yml` — needs secrets + test run |
| macOS arm64 + x86_64 binaries | ⬜ | cargo-dist should handle this |
| Linux x86_64 binary | ⬜ | cargo-dist |
| Windows installer (.msi) | ⬜ | cargo-dist |
| Homebrew formula (`brew install gyre`) | ⬜ | Site advertises this — needs `homebrew-gyre` tap or homebrew-core PR |
| `getgyre.com/install.sh` points to real release | ⬜ | Currently may point to nonexistent binary |

**First action:** Trigger a test release run — push tag `v0.2.0-beta.2` and watch CI, or manually trigger `release.yml`. Fix any secrets/permissions issues.

---

## Track 2 — Site Fixes

Site is live but has wrong content. Fix before any public traffic.

| Item | Status | Notes |
|---|---|---|
| Fix `gyre init` → `gyre setup` everywhere on site | ⬜ | Wrong command, will confuse new users |
| Fix `gyre serve` → `gyre run` everywhere on site | ⬜ | Same |
| Demo GIF — `gyre setup` → `gyre run` → first Telegram message | ⬜ | Two placeholder slots on site already |
| Wire `/docs` to actual QUICKSTART.md content | ⬜ | Docs page exists but content is thin |
| Add GitHub link in nav | ⬜ | No direct link to repo from site currently |

**First action:** Update site commands. This is a copy change — fast.

---

## Track 3 — Deployment Standards

Professional release hygiene before v1.0.

| Item | Status | Notes |
|---|---|---|
| CHANGELOG.md | ⬜ | Human-readable release notes per version |
| Versioning policy | ⬜ | Define what bumps major/minor/patch for Gyre |
| Release notes template | ⬜ | What goes in each GitHub Release description |
| Security policy (SECURITY.md) | ✅ | Already exists (Phase 10 security review) |

---

## Track 4 — Community

Gyre needs a place for early adopters to land.

| Item | Status | Notes |
|---|---|---|
| Waitlist / early access form | ⬜ | Capture interest before public launch |
| `#announcements` channel for releases | ⬜ | Gyre bot posts on new releases |
| First blog post — "Why we built Gyre" | ⬜ | Copy exists on site — formalize as post |

---

## Track 5 — Product Ship Blockers

These came from the Stripe Minions analysis (Mar 6). Required before v1.0.

| Item | Status | Notes |
|---|---|---|
| Blueprint Engine pattern | ⬜ | Deterministic + AI interleaved for curiosity engine runs |
| Docker sandbox per sub-agent | ⬜ | Agent isolation — security requirement |
| NanoClaw validation layer | ⬜ | 0 CI rounds = brittle; need automated validation |

---

## Milestones

| Milestone | Target | Key deliverables |
|---|---|---|
| **M1 — Install works** | Apr 1 | Binary CI runs, install.sh downloads real binary, brew tap live |
| **M2 — Site clean** | Apr 7 | Commands fixed, demo GIF recorded, docs wired |
| **M4 — Ship blockers done** | May 1 | Blueprint Engine, Docker sandbox, NanoClaw CI |
| **v1.0 launch** | May 15 | Everything above + CHANGELOG + launch post |

---

## What's Already Done ✅

- Phases 1-10 built and code-complete
- v0.2.0-beta.1 tagged + pushed to gyre-main
- QUICKSTART.md written
- keychain headless hang fixed
- 1314 tests passing
- Site live with solid copy and design
- Security review complete (Teagan, Phase 10)
- 4 working code examples
- WASM OAuth nonce poll implemented
