# Gyre Blueprint Library

A blueprint is a self-contained, installable capability for Gyre. Each blueprint lives in its own directory under `examples/` and ships with everything needed to understand, install, and customize it.

## Directory structure

```
examples/
└── <blueprint-name>/
    ├── blueprint.json      # machine-readable manifest (required)
    ├── SKILL.md            # human-readable guide (required)
    ├── prompts/            # prompt templates used by routines (optional)
    │   └── *.md
    └── config/             # example configuration files (optional)
        └── *.example.json
```

## blueprint.json

The manifest declares everything Gyre needs to install and wire up the blueprint.

```json
{
  "name": "example-blueprint",
  "version": "0.1.0",
  "description": "One sentence.",
  "required_tools": ["memory_search", "memory_write"],
  "routines": [
    {
      "name": "my-routine",
      "trigger": { "type": "cron", "schedule": "0 8 * * *" },
      "action": {
        "type": "full_job",
        "title": "My Routine",
        "description": "See prompts/my-routine.md"
      },
      "guardrails": { "cooldown_secs": 3600, "max_concurrent": 1 },
      "notify": { "on_attention": true, "on_failure": true, "on_success": false }
    }
  ]
}
```

### Fields

| Field | Type | Description |
|---|---|---|
| `name` | string | Unique identifier. Lowercase, hyphen-separated. |
| `version` | string | Semantic version. |
| `description` | string | One sentence. Used in `gyre blueprint list`. |
| `required_tools` | string[] | Tool names the routines call. Gyre verifies these are available at install time. |
| `routines` | object[] | Routines to install. Each maps directly to the `Routine` type in `src/agent/routine.rs`. |

Routine `trigger`, `action`, `guardrails`, and `notify` fields match the Rust types exactly — see `src/agent/routine.rs` for the full schema and valid values.

## SKILL.md

The human-readable companion. Every SKILL.md covers:

1. **What it does** — what the blueprint installs and why.
2. **Prerequisites** — tools, extensions, or workspace files needed before installing.
3. **Installation** — how to install (via `routine_create` tool call or `gyre routine import`).
4. **Configuration** — which files to copy and edit, and what the key options mean.
5. **Customization** — how to adapt the routines, schedules, or prompts for your situation.

## Prompts

Prompt files are plain Markdown. Routines reference them by path in their `description` or `prompt` field. Keeping prompts in separate files makes them easier to read and edit without touching `blueprint.json`.

Convention: one file per routine, named to match the routine.

## Config

Config files hold user-editable data (watch lists, topic pools, thresholds) that is separate from the prompt logic. Provide `.example.json` files — users copy and rename them, then edit to suit.

## Conventions

- **No model version strings.** Blueprints run on whatever model the user configured.
- **No hardcoded endpoints, tokens, or usernames.** Use Gyre's secrets store or workspace config files.
- **Memory via tools, not shell.** Use `memory_write` and `memory_search`, never `curl` to internal APIs.
- **Goals, not step-by-step recipes.** Prompts state the outcome; the model figures out execution.
- **Novelty is prompt-level.** If a routine should avoid re-doing work, it calls `memory_search` first and decides based on what it finds. See `docs/patterns/novelty-gate.md`.
