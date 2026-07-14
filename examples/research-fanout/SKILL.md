# Research Fan-Out

A nightly pipeline that searches the web across configurable topic pools, adversarially verifies each finding, stores confirmed results to Gyre's workspace memory, and delivers a clean summary to your configured notification channel.

## What it installs

Two routines:

- **research-harvest** (6:00 AM daily) — runs the four-stage pipeline: harvest → skeptic-verify → store to memory → write a digest file.
- **research-digest** (6:30 AM daily) — reads today's digest from memory and sends a notification if there are findings worth your attention.

The pipeline is entirely prompt-driven. No code runs outside Gyre. All storage goes through `memory_write` / `memory_search`.

## Prerequisites

- Gyre must have a web search tool available (built-in or via an installed MCP/WASM extension).
- `memory_write` and `memory_search` are built-in; no extra setup.
- Your notification channel should be configured in Gyre's settings (e.g. TUI, Telegram, Slack).

## Installation

### Option A: via the routine_create tool

Ask Gyre to install the routines by pasting the contents of `blueprint.json` into a `routine_create` call, or say:

> "Install the research-fanout blueprint from examples/research-fanout/blueprint.json"

Gyre will create both routines and confirm.

### Option B: manual import (future)

Once `gyre blueprint import` is available:

```bash
gyre blueprint import examples/research-fanout/
```

### After installing

1. Copy `config/watch-topics.example.json` to your workspace as `research/config/watch-topics.json` and edit it to reflect your actual topics (see Configuration below).
2. Verify the routines appear: ask Gyre "list my routines" or check the web UI under Routines.
3. To run immediately without waiting for the schedule: ask Gyre to run the `research-harvest` routine manually.

## Configuration

### config/watch-topics.json

This file defines your topic pools. Copy the example and edit:

```
workspace/
└── research/
    └── config/
        └── watch-topics.json   ← place here so routines can read it
```

Each pool has:

| Field | Description |
|---|---|
| `id` | Machine identifier, used as a namespace in memory paths. |
| `label` | Human label used in digests and notifications. |
| `topics` | List of search queries or topic descriptions. Be specific — "Q3 earnings surprises in semiconductor sector" will yield better results than "semiconductors". |
| `sources` | Preferred source types (news sites, forums, research aggregators, etc.). The harvester uses these as guidance, not strict filters. |

### Notification channel

The digest routine's `notify.channel` field in `blueprint.json` defaults to your system default. To route digests to a specific channel, update the routine after install:

> "Update routine research-digest to notify on my telegram channel"

### Schedule

The default schedule runs at 6:00 and 6:30 AM. Adjust the cron expressions in `blueprint.json` before installing, or update the routines afterward:

> "Change the research-harvest routine to run at 5 AM instead"

## Customization

### Add or remove pools

Edit `research/config/watch-topics.json`. The pipeline reads this file at runtime, so changes take effect on the next run without reinstalling.

### Adjust how many findings per pool

The harvest prompt (`prompts/harvest.md`) specifies a maximum of 5 findings per pool. Raise or lower this to match how much signal your topics tend to produce.

### Change the verification threshold

By default, the skeptic stage verifies all findings with relevance "high" or "medium". Edit `prompts/skeptic-verify.md` to change which findings get checked. Low-volume, high-trust topic pools may not need verification at all.

### Extend the digest format

Edit `prompts/digest.md`. The pipeline stores findings in a structured path (`research/digests/YYYY-MM-DD.md`) in workspace memory, so you can also query past digests directly:

> "Summarize what the research pipeline found this week about [topic]"

## Refutation learning loop

When the skeptic stage refutes a finding, the pipeline logs the failure type and a one-sentence lesson to `research/refutation-log.md` in workspace memory. The harvest prompt reads the most recent lessons for each pool and includes them as context, so the pipeline improves over time without manual intervention.

To review what the pipeline has learned:

> "Read research/refutation-log.md from my workspace"

## Disabling

> "Disable the research-harvest routine" / "Disable the research-digest routine"

Or delete both routines to remove entirely. Your digest history and refutation log remain in workspace memory.

## Full-job execution

Both routines are `full_job` actions because they need web-search and memory tools. Full-job routines run as real scheduler jobs through the worker reasoning loop with tool access, so the pipeline runs autonomously on its schedule.

The workspace-internal memory tools (`memory_search`, `memory_write`, `memory_read`, `memory_tree`) are pre-approved for routines via a first-party trust hook and execute without a human in the loop. Web-search comes from an installed MCP/WASM extension: whether it runs unattended depends on that extension's own approval policy (declared in its `capabilities.json`). If your search extension is approval-gated, either grant it standing approval or add its tool name to a trust hook the same way memory tools are trusted. Destructive parameter combinations always require approval regardless of trust.

Note: `routine_test` dry-runs a `full_job` as a single tool-less LLM call to stay side-effect-free (no memory writes, no notifications), so its judged output is an approximation of a real run. The report flags this in its caveats.
