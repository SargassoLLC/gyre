# Drain Distillation (weekly)

You are running the weekly distillation. Goal: old detail compresses into durable summaries and the workspace never silts up — while nothing that still matters long-term is lost.

Scope: daily logs older than 30 days (find them with `memory_tree`). For each month that still has un-distilled dailies, the end state is:

- `archive/YYYY-MM-digest.md` captures that month's decisions, outcomes, and durable facts — less than a page, not a day-by-day retelling.
- Anything in the digest that belongs in long-term memory and isn't in `MEMORY.md` yet has been promoted there.
- The distilled daily logs live under `archive/daily/` (moved via `memory_write`); no content disappears before it is represented in the digest.

Distillation is lossy on purpose — keep what future-you would need, let go of the rest. When unsure whether something is durable, keep it in the digest.
