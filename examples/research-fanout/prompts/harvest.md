# Research Harvest Prompt

This prompt runs inside the `research-harvest` routine (Stage 1 of 4). It is sent once per topic pool as a parallel job. The orchestrating turn collects all results before proceeding to verification.

---

## Orchestrator instructions (runs in research-harvest routine)

Read the watch topics config from your workspace:

```
memory_read path="research/config/watch-topics.json"
```

Check whether today's digest already exists:

```
memory_search query="research digest YYYY-MM-DD" limit=1
```

If a digest for today already exists in memory, stop — the pipeline already ran today. Report needs_attention: false in your closing check-in.

Otherwise, for each pool in the config, run a harvest pass with the prompt below.

---

## Per-pool harvest prompt

You are searching for recent developments in the **{pool.label}** domain.

Search for news and updates from the last 48 hours on the following topics:

{pool.topics — one per line}

Prefer these source types: {pool.sources — comma-separated}

Return your findings as a JSON object:

```json
{
  "pool": "{pool.id}",
  "findings": [
    {
      "claim": "One-sentence factual claim, specific and verifiable",
      "source_url": "The URL where you found this",
      "source_date": "YYYY-MM-DD",
      "relevance": "high | medium | low",
      "entities": ["named entities mentioned"],
      "excerpt": "A short quote or data point from the source"
    }
  ]
}
```

Rules:
- Maximum 5 findings per pool.
- Only include genuinely new information from the last 48 hours.
- Every finding must have a real source URL — do not invent URLs.
- If nothing new is found, return `{"pool": "{pool.id}", "findings": []}`.
- Prefer specific, falsifiable claims over vague summaries.

{KNOWN_PITFALLS}
Before searching, read recent lessons from the refutation log for this pool:

```
memory_search query="refutation-log pool:{pool.id}" limit=5
```

If lessons are found, include them as context while searching. Avoid the specific errors described. If no lessons are found, omit this section.
{/KNOWN_PITFALLS}

---

After all pool harvests complete, collect the results and pass them to the verify stage (see `prompts/skeptic-verify.md`).
