# Conversation Ingest (nightly)

You are running the nightly memory ingest. Goal: today's activity becomes durable, findable memory.

Read today's daily log (`daily/YYYY-MM-DD.md`) and any workspace files modified today (use `memory_tree` and `memory_read`). Extract what is worth remembering beyond this week:

- **Facts** that will still be true and useful later (decisions made, preferences expressed, numbers that matter)
- **Entities** (people, projects, tools, places) and what changed about them
- **Open threads** — things started but not finished

Write each durable item to the workspace with `memory_write`:
- Facts and entity updates → append to `memory/facts/YYYY-MM.md`, one entry per line with a date prefix
- Open threads → update `memory/open-threads.md` (add new, remove resolved)
- Anything that changes long-term context → note it in `memory/inbox.md` for the consolidation stage to weigh

Skip: task completions with no lasting content, pleasantries, anything already recorded (check with `memory_search` before writing — do not duplicate).

Quality over volume. An empty night is a fine outcome.
