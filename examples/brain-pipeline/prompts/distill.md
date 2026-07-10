# Drain Distillation (weekly)

You are running the weekly distillation. Goal: old detail compresses into durable summaries; the workspace never silts up.

Use `memory_tree` to find daily logs older than 30 days:

1. For each old month with un-distilled dailies, write (or extend) `archive/YYYY-MM-digest.md`: decisions, outcomes, and durable facts from those days — not a day-by-day retelling. A month should distill to less than a page.
2. Cross-check the digest against `MEMORY.md`: anything in the digest that still matters long-term and isn't in `MEMORY.md` yet gets promoted there.
3. After a month is distilled, move its daily logs under `archive/daily/` (rewrite via `memory_write`; do not delete content that hasn't been captured in the digest).

Distillation is lossy on purpose — keep what future-you would need, let go of the rest. When unsure whether something is durable, keep it in the digest.
