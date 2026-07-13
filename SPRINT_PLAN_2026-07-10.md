# Gyre Sprint Plan — 2026-07-10 (Tier 1 + Tier 2, v0.3.0)

**Status:** awaiting "go" from Greg
**Rev 2 (same day):** validated against a full Lyzr.ai feature sweep (see `COMPETITIVE-lyzr-2026-07-10.md`) and the Sargasso repositioning plan (`~/kimi/sargasso/00_Strategy/reposition-plan-2026-07-10.md`). Changes from Rev 1 are in the "Rev 2 — holistic product revisions" section at the bottom; the table and estimates below are updated in place where affected.
**Model:** Fable 5 | **Repo state:** 2 commits (`bcd7d62` beta.1 initial, `96d1d0e` test fix)
**Build health:** `cargo build` ✅, full test suite **1,445 passed / 0 failed** (after fixing a test-compile error + 4 latent fixture bugs in `src/safety/leak_detector.rs`)

---

## What exploration found (differs from SPRINT_BRIEF.md in places)

1. **1.1 Fallback resilience** — `FailoverProvider` (src/llm/failover.rs) is sound and well-tested. The Gyre-shaped bug is in `set_model()` (failover.rs:312-317): it propagates an explicit model to **every** provider in the chain with `?` short-circuit. `/model claude-sonnet-4-6` (src/agent/commands.rs:467) overwrites the fallback provider's model with an Anthropic ID — the chain survives in structure but is dead in practice, or the switch errors outright.
2. **1.2 Cron delivery** — `send_notification()` (src/agent/routine_engine.rs:499-542) **ignores `NotifyConfig.channel` and `.user` entirely**. The forwarder (src/agent/agent_loop.rs:407-428) reads a `notify_user` metadata key the engine never sets, then `broadcast_all`s. Delivery target is dropped at the first hop.
3. **1.3 Stale origin** — Gyre threads are keyed by `(channel, user, external_id)` (session_manager.rs, with cross-channel isolation tests) and `Session` carries no origin fields. Bug may not exist by construction; needs a focused verification pass of the respond path, priced separately.
4. **1.4 Hook tool policies** — Gyre hooks (src/hooks/hook.rs) carry **no tool-policy concept**. This is a small missing feature (additive `trusted_tools()` on the `Hook` trait + union merge in registry + wiring into `session.auto_approved_tools`), not a merge bug.
5. **1.5 toolsAllow inheritance** — confirmed as briefed: `ContainerJobConfig.claude_code_allowed_tools` set once at startup (main.rs:1099); `create_job()` (job_manager.rs:247) takes no per-job allow-list; job_manager.rs:82 defaults to `ClaudeCodeConfig::default()`. Fix: per-job override threaded to `CLAUDE_CODE_ALLOWED_TOOLS` env (job_manager.rs:342), which claude_bridge.rs already consumes.
6. **1.6 sessions_send** — **does not exist** (zero grep hits). Net-new feature. `ChannelManager` already has `broadcast(channel, user)` + `inject_sender()`, so direct-delivery (never silent-queue) is a natural fit.
7. **Discovered issue (not in brief):** `FullJob` routines silently execute as lightweight — scheduler integration is a stub (routine_engine.rs:315-323). Undercuts 1.5 and the Research Fan-Out skill.
8. **Reference path corrections:** fan-out spec is at `~/.openclaw/workspace/research/research-fanout.md` (brief's path wrong); CrabTrap plan at `~/.openclaw/workspace/docs/CRABTRAP_PLAN.md`; Rex's hermit-loop config **not found** at `~/agents/` — Novelty Gate will be written from the pattern description.

## Sprint table

| # | Item | Files to modify | Lines/structs affected | Est. LOC Δ | Complexity | Ambient-AI blocker? |
|---|---|---|---|---|---|---|
| 1.1 | Fallback resilience | src/llm/failover.rs | `set_model()` :312; per-provider model pinning + tests | ~60 | S | harness quality |
| 1.5 | toolsAllow in isolated jobs | src/orchestrator/job_manager.rs :67,:82,:247,:342; src/tools/builtin/job.rs; src/channels/web/server.rs | `ContainerJobConfig`, `create_job()` signature, call sites | ~120 | M | security-adjacent |
| 1.2 | Cron delivery awareness | src/agent/routine_engine.rs :499-542; src/agent/agent_loop.rs :388-428 | `send_notification()`, forwarder; carry `NotifyConfig.{channel,user}` | ~70 | S | harness quality |
| 1.3 | Stale origin reset | verify: src/agent/thread_ops.rs, src/channels/manager.rs | possibly none — may close as "safe by construction + regression tests" | ~30 (tests) | S (+exploration) | harness quality |
| 1.4 | Hook tool-policy merge | src/hooks/hook.rs, src/hooks/registry.rs; approval check in src/agent/ | `trusted_tools()` on `Hook` trait, union in `run()`, feed approval gate | ~100 | M | harness quality |
| 1.6 | sessions_send + routing | new src/tools/builtin/sessions.rs; src/agent/session_manager.rs; src/channels/manager.rs | new tool, active-turn detection, direct `broadcast()` delivery when idle | ~300 | L | multi-agent story |
| 2.4 | Brain Pipeline + retrieval wiring | new examples/brain-pipeline/; src/agent/thread_ops.rs or agent_loop.rs (auto-recall); config flag | turn-start `workspace.search()` top-k injection; 4 routine configs (ingest/consolidate/surface/distill) | ~250 + docs | L | **MEMORY blocker** |
| 2.2 | CrabTrap for main session | new src/safety/egress.rs (or extensions/crabtrap/); src/tools/builtin/http.rs, shell.rs; docs | egress policy on native tool HTTP + HTTP_PROXY wiring docs | ~300-500 | L | **SECURITY blocker** |
| 2.1 | Research Fan-Out | new examples/research-fanout/ (SKILL.md, watch-topics, prompts); maybe FullJob→Scheduler fix | prompt chain from spec, de-sargasso'd (memory_write, not curl) | ~150 docs (+~150 if FullJob fix) | M→L | curiosity pillar |
| 2.3 | Novelty Gate | new docs/patterns/novelty-gate.md + example prompt | pattern doc, memory_search-first template, zero enforcement code | ~120 docs | S | curiosity pillar |

## Ambient AI gap assessment

| Capability | Gyre status | Verdict |
|---|---|---|
| Memory | Better than briefed: hybrid RRF search, memory tools, embedding backfill, identity injection all built. **Missing: automatic recall** — nothing surfaces memories into a live turn unless the model calls `memory_search` itself. | 2.4 is smaller than it looks — pipeline is routines-config; the blocker fix is ~100 LOC of context wiring |
| Security | Docker sandbox, WASM capability sandbox, credential injector, `leak_detector.scan_http_request()` exist — for sub-agents/WASM tools. Native `shell`/`http` in main session have host access, gated only by approval prompts + the regex sanitizer the audit says to delete. | 2.2 is the real gap |
| Curiosity | Routines/cron solid; FullJob→scheduler stub limits multi-stage autonomy | 2.1 + FullJob fix |
| Multi-agent | Jobs/orchestrator/containers ✅; no inter-session messaging | 1.6 closes it |
| Autonomy | Heartbeat + cron ✅ | done |

Extra: `HEARTBEAT_OK`/`ROUTINE_OK` sentinels use `contains()` — audit Theme B names Gyre for structured-output replacement. ~40 LOC, fold into 1.2 while in routine_engine.rs.

## Alternatives (Greg's call)

1. **CrabTrap (2.2)** — *Prescribed:* port Brex proxy as `extensions/crabtrap/`. *Alternative (recommended):* native `EgressPolicy` layer in src/safety/ for all native-tool HTTP — allowlist + optional LLM-judge + audit log — plus first-class HTTP_PROXY support and a documented compose file for real CrabTrap. Tradeoff: native ships in the binary, zero Docker dependency, reuses `scan_http_request`; proxy port is more reference-faithful but adds a mandatory container + MITM-cert management to every install. Both honor "boundaries at the OS / judgment in the model."
2. **Brain Pipeline (2.4)** — *Prescribed:* port the 4-stage sargasso cycle. *Alternative (recommended):* ship the 4 stages as **routine configs** (existing general mechanism, audit rule 6) + auto-recall injection as the one real code change. The Python `curiosity/` system is 40+ modules; porting it rebuilds the compaction-industrial complex in Rust.
3. **sessions_send (1.6)** — deliver via existing `ChannelManager::broadcast` immediately when target isn't mid-turn; no queue, no pub/sub. Queue-then-forward is the silent-queue failure being fixed.
4. **Novelty Gate (2.3)** — pattern doc + memory_search-first prompt only. No enforcement code — a novelty *checker* would be a shadow supervisor (audit rule 7).

## Token + session estimate

| Block | Throughput (in+out) | Output tokens | Notes |
|---|---|---|---|
| Tier 1 (all six) | ~350-450k | ~100-140k | 1.3 includes ~25k exploration; 1.6 is the wide one |
| 2.4 Brain Pipeline | ~120-180k | ~40-60k | mostly config/docs + one surgical context change |
| 2.2 CrabTrap (native) | ~150-250k | ~50-80k | new module + tests; LLM-judge half is uncertain |
| 2.1 Fan-Out | ~60-100k | ~20-30k | +80-120k if FullJob→Scheduler fix in scope |
| 2.3 Novelty Gate | ~20-30k | ~8-12k | cheapest |

**Session split:** Tier 1 + 2.4 + 2.3 fit in one session. **2.2 CrabTrap gets its own focused session** (gated on the design decision anyway). 2.1 fits if FullJob fix deferred.

**Implementation order (commit after every item; stub + IMPLEMENTATION_NOTES.md if quota runs short):**
1.1 → 1.5 → 1.2 (+sentinel fix) → 1.3 → 1.4 → 1.6 → **2.4** → 2.3 → 2.1 → 2.2

---

## Rev 2 — holistic product revisions (after Lyzr sweep, 2026-07-10)

Lyzr ($100M Series B, "enterprise agentic OS") is structurally unable to serve Gyre's segment: they are request-driven enterprise workflow agents at $0.03–0.08/agent-run, sold top-down, cloud/VPC only. Nothing in their stack is local-first, ambient/proactive, personal, or flat-cost. Gyre's "the AI OS" claim lives exactly there — the same capture-what-they-can't-serve strategy as Sargasso's services repositioning. Full analysis: `COMPETITIVE-lyzr-2026-07-10.md`.

**Plan changes:**
1. **Tier 1 unchanged** — Lyzr sells "durable execution, exactly-once"; reliability is table stakes in this category.
2. **2.2 is now a product pillar, not just a fix:** native `EgressPolicy` layer + surfaced audit/decision log (Gyre already event-sources `job_actions`) = "Responsible AI, local-first" — the direct answer to Lyzr's #1 differentiator. Native-Rust route confirmed over Brex proxy port.
3. **Blueprint format:** `examples/` ships with one consistent structure (manifest + SKILL.md + routine configs + prompts). Brain Pipeline, Research Fan-Out, Novelty Gate = blueprints #1–3 of a growable library (Lyzr's 200-blueprint moat is the pattern to seed against). ~1 hour extra structure.
4. **Brain Pipeline stays Tier-2 #1** — must beat Cognis/KG-aaS on "actually works end-to-end locally" or the ambient claim collapses on comparison.
5. **Explicit non-goals (Lyzr's turf + Bitter-Lesson violations):** no-code builder, SSO/RBAC/multi-tenancy, evals/simulation engine, toxicity/bias classifier stacks (shadow supervisors, audit rule 7), multi-framework control plane. Web-gateway run-trace/cost polish is real but Tier 3/6.
6. **Sargasso synergy (note, not scope):** the Docker Compose client-deployment template in the reposition plan (Gap 1) and Gyre's sandbox/orchestrator infra should converge post-v0.3 — one deployable "agent box" serves both the product and the services delivery motion.

Implementation order unchanged.

---

## Rev 3 — simulation absorbed; non-goals refined (Greg pushback, 2026-07-10)

**Simulation/Agent Eval: IN (was wrongly grouped with non-goals).** It passes every audit rule: leverages computation (LLM-generated scenarios + LLM-judge scoring, no hand-coded rubrics), and the 10x test says better models make it *more* valuable. It's CI for agents. Gyre already has the seed — `src/evaluation/` (`SuccessEvaluator`, `LlmEvaluator`, `MetricsCollector`), currently only wired into job execution.

- **v0.4 (full):** `gyre blueprint test <name>` — generate N synthetic scenarios for a routine/blueprint, dry-run, score with `LlmEvaluator`, report a readiness verdict before the user enables the cron. Pre-flight trust for an OS that acts while you sleep; nobody has this local-first.
- **This sprint (optional thin slice, Greg's call):** `gyre routine test` — single dry-run, no notifications/writes, LLM-judge verdict. ~M, ~40-60k tokens, builds entirely on existing evaluation module. Slots after 2.3 in the order if approved.

**Reframed (already have it, differently shaped — say so in positioning):**
- *Architect (NL→agent):* `routine_create` tools + dynamic tool builder = you talk to the OS and it builds the agent. Model-as-builder, not form wizard.
- *AgentMesh:* 1.6 `sessions_send` + orchestrator is the embryo.

**Still non-goals, with reasons:** toxicity/bias classifier stacks (shadow supervisors — audit rule 7; egress boundary + HITL + decision logs cover the real risk); SSO/RBAC/multi-tenancy (services isolation model is one-Gyre-box-per-client — stronger than RBAC, zero identity infra); multi-framework control plane (Lyzr vs LangChain enterprise fight, irrelevant to a personal OS).

---

## Hardening Patterns to fold into the roadmap (field-tested 2026-07-13)

General architecture principles proven out in production this week — candidates for Gyre core:

1. **Single source of truth + runtime parity.** The top failure mode is split-brain: multiple stores of "truth" (or sandboxed vs. host filesystem views) drifting apart with no reconciliation, so the reader lands on the wrong one. Enforce identical state at identical paths across runtimes; give every canonical fact exactly one authoritative home.
2. **Fact-of-record retrieval tier.** Similarity/embedding retrieval rewards *repetition*, not *truth* — a value written 250 times beats the correct value written once. Add an authoritative tier-0 (curated, dated, cited facts) checked before any semantic lane, plus **confidence gating**: below threshold, surface "no authoritative record — consult the canon," never serve low-confidence output as if it were fact.
3. **Verified completion, not self-report.** Agents must not self-declare "done." Completion requires an independent (cheap) verifier that checks evidence against a definition-of-done before status flips; retry-with-critique on failure; escalate after N rejections. Prevents plausible-but-false "done" claims.
4. **Budget governor.** Any autonomous/looping execution needs a hard spend ceiling measured against *real* per-call cost data and checked pre-dispatch — not an advisory note.
5. **Pipeline durability signals.** Background pipelines fail silently for months. Require heartbeats + health checks so a dead stage surfaces within a day, not at the next audit.
6. **Observability before autonomy.** An agent that reads the wrong environment produces phantom work ("rebuild loop"). Preflight-verify the environment before acting; abort loudly if the world looks wrong rather than improvising.
7. **One review surface.** Consolidate autonomous output into a single human-review digest — verified-complete / needs-decision / failed / spend — so oversight is minutes, not archaeology.
