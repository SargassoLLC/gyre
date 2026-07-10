# Pattern: Novelty Gate

A novelty gate is a step at the start of a routine's prompt that instructs the agent to search its own memory before doing any work, and to proceed only if the planned work is genuinely new relative to what it finds.

## The problem it solves

Autonomous routines run on a schedule, not on demand. Without a novelty check, a routine that monitors a topic will produce the same report every time the cron fires — even if nothing has changed since the last run. The user gets noise, the memory fills with duplicates, and the routine trains itself to be ignored.

## Why enforcement code is the wrong shape

It is tempting to solve this in code: track a hash of the last result, compare before running, skip if equal. The problem is that this makes the runtime a shadow supervisor — it decides what is "new" using a rule the user cannot inspect or override. The model is better at that judgment than a hash comparison, because "new" is a semantic property, not a structural one. A slightly rephrased version of the same finding should be caught; a genuinely different finding about the same entity should not be blocked. Putting novelty logic in code also makes the behavior invisible: users cannot read a hash check the way they can read a prompt.

## Reference prompt template

Add this block at the beginning of any routine prompt that should avoid re-doing recent work:

```
Before starting, search your workspace memory for recent work on this topic:

  memory_search query="{topic or routine name} recent findings" limit=5

Review what you find. If the planned work duplicates something completed in the last {window — e.g. 24 hours / 7 days}, stop and report needs_attention: false in your closing check-in, with the summary noting that the work already exists. Otherwise, proceed.

When you do proceed, note what made this run novel relative to past results (one sentence). Include that note in your output or state file.
```

Adjust the query and time window to match the routine's scope. A nightly digest should check the last 24 hours. A weekly synthesis should check the last 7 days. A one-off research task may need no novelty gate at all.

## Wiring it into a routine

### In the routine's prompt

Embed the novelty gate at the top of the `description` or `prompt` field. The agent reads it as the first instruction and acts accordingly.

Example for a `full_job` action:

```json
{
  "type": "full_job",
  "title": "Weekly Tech Digest",
  "description": "Before doing anything, call memory_search with query='weekly tech digest' and limit=3. If a digest from the current week already exists in the results, stop and report needs_attention: false. Otherwise, run the full pipeline described in prompts/weekly-digest.md.",
  "max_iterations": 15
}
```

### In the routine's state file

Routines can use workspace memory to track their own state across runs. The convention is:

```
workspace/
└── routines/
    └── {routine-name}/
        └── state.md
```

The routine reads `routines/{name}/state.md` at the start and writes an updated entry at the end:

```
memory_read path="routines/weekly-tech-digest/state.md"
```

After completing a run, write a brief summary:

```
memory_write path="routines/weekly-tech-digest/state.md"
content="Last run: YYYY-MM-DD. Found N confirmed findings. Topics covered: [list]. Next run should focus on: [anything left open]."
```

This state file serves two purposes: the novelty gate (did this run recently?) and continuity (what should the next run pick up?). It is readable by the user and editable if the user wants to reset or redirect the routine.

## Example: research-fanout

The `research-harvest` routine in `examples/research-fanout/` applies this pattern:

1. The harvest prompt begins by calling `memory_search` for today's digest.
2. If a digest exists, the prompt instructs the agent to stop and report needs_attention: false.
3. If no digest exists, the agent proceeds with the full harvest.

The state is stored at `research/digests/YYYY-MM-DD.md` — the digest itself is the state record. No separate state file is needed because the presence of the digest is the signal.

## What the agent should say when it gates

Routine runs end with a structured JSON check-in — `{"needs_attention": <bool>, "summary": "..."}` (the engine appends the exact format instructions to every routine prompt). When the gate stops a run, the agent reports:

```json
{"needs_attention": false, "summary": ""}
```

The engine treats `needs_attention: false` as an OK run: no notification, run logged for audit. This is structured output, not sentinel-string matching — the flag is unambiguous even when the summary text discusses what was or wasn't found.

## Summary

The novelty gate is a single `memory_search` call and a branch in the prompt. It requires no code changes, is visible to the user in the prompt text, and uses the model's judgment to decide what counts as genuinely new. State is stored in workspace memory using paths the user can read, edit, or reset directly.
