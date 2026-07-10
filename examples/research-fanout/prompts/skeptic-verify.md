# Skeptic Verify Prompt

This prompt runs inside the `research-harvest` routine (Stage 2 of 4). It is applied to each finding with relevance "high" or "medium". Low-relevance findings are passed through as "unverifiable" without a verification pass.

---

## Per-finding skeptic prompt

Your job is to challenge this claim. Try to refute it.

**Claim:** {finding.claim}
**Source:** {finding.source_url}
**Domain:** {pool.label}

Search for contradicting evidence. Look for:
- Counter-evidence that disproves the claim
- More recent information that supersedes it
- Signs the source is unreliable or misrepresenting data

Return a JSON verdict:

```json
{
  "verdict": "confirmed | refuted | unverifiable",
  "confidence": 0.0,
  "counter_evidence": "What you found against it, or null",
  "verification_source": "URL of your verification source, or null"
}
```

Verdict rules:
- `confirmed` — only if you found corroborating evidence from a different source.
- `refuted` — if you found clear contradicting evidence.
- `unverifiable` — if you could not find evidence either way.

Default to `unverifiable` when uncertain. Do not confirm without a second source.

---

## After all verdicts are collected (Stage 3: store)

For each finding, take action based on its verdict:

**confirmed** — Write to workspace memory:

```
memory_write path="research/findings/YYYY-MM-DD/{pool.id}-{index}.md"
content="[RESEARCH] {claim} | source: {source_url} | verified: YYYY-MM-DD | pool: {pool.id} | verification_source: {verification_source}"
```

**unverifiable** — Write with an UNVERIFIED tag:

```
memory_write path="research/findings/YYYY-MM-DD/{pool.id}-{index}-unverified.md"
content="[UNVERIFIED] {claim} | source: {source_url} | date: YYYY-MM-DD | pool: {pool.id}"
```

**refuted** — Do not store as a finding. Log to the refutation log (see Stage 3.5 below) and note in the digest.

After all findings are stored, write the digest:

```
memory_write path="research/digests/YYYY-MM-DD.md"
content="[digest content — see digest.md for format]"
```

---

## Stage 3.5: Refutation learning loop

For each refuted finding, append a lesson to the refutation log:

```
memory_write path="research/refutation-log.md" append=true
content="[REFUTATION] date: YYYY-MM-DD | pool: {pool.id} | failure_type: {type} | lesson: {one-sentence lesson} | evidence: {what the skeptic found}"
```

Failure type taxonomy:
- `conflation` — mixed up two related but distinct metrics or entities
- `wrong_specifics` — the broad trend was real but specific details were wrong
- `hallucinated_data` — claimed a data point that does not exist in the source
- `stale_info` — was true at some point but no longer accurate
- `source_unreliable` — the source itself was low-quality or misrepresented

The harvest prompt reads these lessons before searching (see `prompts/harvest.md`), creating a feedback loop that improves accuracy over time.
