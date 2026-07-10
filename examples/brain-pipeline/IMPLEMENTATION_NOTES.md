# Brain Pipeline — Implementation Notes (for the next session)

## What is DONE (2026-07-10)

- **Retrieval wiring (the part that was broken in the reference system):**
  `src/agent/dispatcher.rs` now auto-recalls workspace memories into every
  turn — hybrid search on the incoming message, top-k injected as a
  `<recalled_memories>` block in the composite system prompt.
  Knob: `MEMORY_AUTO_RECALL_TOP_K` (AgentConfig, default 3, 0 = off).
  Formatter + truncation tested (`format_recalled_memories`).
- Blueprint (this directory): manifest, SKILL.md, 4 stage prompts.
- The surface stage works end-to-end today (lightweight action, reads
  `surface/candidates.md` + `context/priorities.md` via context_paths).

## What BLOCKS full pipeline execution

`RoutineAction::FullJob` falls back to a single tool-less LLM call —
`src/agent/routine_engine.rs` (`execute_routine`, the `FullJob` match arm
logs "scheduler integration pending"). Stages 1/2/4 need `memory_write`/
`memory_read`/`memory_tree`, so they run degraded until routines can
execute tool-using jobs.

## Next session: FullJob → Scheduler integration

1. In `execute_routine`, route `RoutineAction::FullJob` to the existing
   `Scheduler` (`src/agent/scheduler.rs`) as a real job with the routine's
   title/description and `max_iterations`; the worker path already
   executes tools with safety + hooks.
2. Thread the routine's `RoutineNotification` target through job
   completion so results deliver to the configured channel (the typed
   notification path added in fix 1.2).
3. Record `job_id` on the `RoutineRun` row (field already exists).
4. Tool policy: routine jobs go through the worker approval gate — tools
   with `requires_approval()` need either hook trust (fix 1.4) or a
   first-party "routine memory hook" that vouches for the memory tools
   (they are workspace-internal; a reasonable default grant).
5. Estimated scope: ~150-250 LOC + tests, one focused session.

## Design intent to preserve

- Prompts are goal statements; do not turn them into step recipes.
- No enforcement-code novelty checks; the ingest prompt's
  "check memory_search before writing" is the pattern.
- The morning brief is the user-facing quality bar for the whole system.
