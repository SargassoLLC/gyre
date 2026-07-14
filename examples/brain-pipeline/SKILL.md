# Brain Pipeline

A four-stage nightly memory cycle that makes an ambient OS feel alive: what happened today becomes memory, memory gets consolidated, and what matters is waiting for you in the morning.

## What it does

| Stage | Routine | When | What |
|---|---|---|---|
| 1. Ingest | `brain-ingest` | 11:00 pm daily | Extracts durable facts, decisions, and entities from today's daily logs and conversations into `memory/` workspace files |
| 2. Consolidate | `brain-consolidate` | 2:00 am daily | Merges duplicates, links related memories, promotes recurring themes into `MEMORY.md` |
| 3. Surface | `brain-surface` | 6:00 am daily | Reads consolidation output + your priorities, writes `surface/today.md`, and notifies you with the morning brief |
| 4. Distill | `brain-distill` | 11:30 pm Sundays | Compresses daily logs older than 30 days into monthly digests; archives the originals |

**The retrieval half is built into Gyre itself** — this is the part most memory systems get wrong. Every turn, Gyre runs a hybrid search (FTS + vector, RRF-fused) over your workspace with the incoming message and injects the top matches into context as `<recalled_memories>`. Storage without retrieval is a glorified log; this pipeline is useful *because* what it writes automatically resurfaces in live conversations.

Retrieval knob: `MEMORY_AUTO_RECALL_TOP_K` (default `3`, `0` disables). The model can always call `memory_search` explicitly on top.

## Prerequisites

- A configured workspace (Postgres or libSQL backend)
- Embeddings enabled for semantic recall (`EMBEDDING_ENABLED=true` + provider key) — without embeddings, recall is keyword-only
- Routines enabled (`ROUTINES_ENABLED=true`)

## Installation

Ask Gyre to install it:

> "Create the four routines from examples/brain-pipeline/blueprint.json, using the matching prompt files from examples/brain-pipeline/prompts/ as each routine's prompt."

Or create each routine manually with the `routine_create` tool using the trigger/action/notify values in `blueprint.json` and the prompt file contents.

## Configuration

- Surface stage reads `context/priorities.md` — keep it current; it's how the morning brief knows what matters to you.
- Adjust cron schedules in `blueprint.json` to your timezone/rhythm before installing.
- `notify` defaults: only the morning surface messages you; ingest/consolidate/distill run silently unless they fail.

## Customization

- Add `context_paths` to the surface routine for anything else the morning brief should weigh (projects list, calendar export, etc.).
- The prompts state goals, not procedures — tune the *goal* (e.g., "surface at most 5 items") rather than adding step lists.
- Pair with the novelty gate pattern (`docs/patterns/novelty-gate.md`) if you add your own autonomous stages.

## Full-job execution

Stages 1, 2, and 4 are `full_job` routines because they need memory tools. Full-job routines run as real scheduler jobs through the worker reasoning loop with tool access — they can read and write memory during autonomous execution. The workspace-internal memory tools (`memory_search`, `memory_write`, `memory_read`, `memory_tree`) are pre-approved for routines via a first-party trust hook, so they execute without a human in the loop; any other approval-gated tool remains blocked, and destructive parameter combinations still require approval. The surface stage (lightweight, read-only) and the auto-recall retrieval layer complete the pipeline.

Note: `routine_test` dry-runs a `full_job` as a single tool-less LLM call to stay side-effect-free (no memory writes, no notifications), so its judged output is an approximation of a real run. The report flags this in its caveats.
