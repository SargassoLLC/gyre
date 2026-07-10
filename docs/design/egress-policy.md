# EgressPolicy — Responsible AI, local-first (design for v0.4)

**Status:** designed 2026-07-10, implementation scheduled for its own focused session.
**Why it exists:** the main session's native tools (`http`, dynamically built tools) reach the network with host access, gated only by approval prompts. Sub-agents and WASM tools are already sandboxed (Docker, capability allowlists, credential injector, leak detector). This closes the gap and packages the whole stack as Gyre's answer to enterprise "Responsible AI" modules — inside the binary, no proxy container required.

## Shape

One module, `src/safety/egress.rs`, on the native tool HTTP boundary:

```
tool request ──▶ EgressPolicy.check(request)
                   1. leak scan        (existing LeakDetector::scan_http_request)
                   2. rule match       (allow / deny / unmatched)
                   3. unmatched → mode:
                        observe  → allow + audit          (default at beta)
                        enforce  → deny + audit
                        judge    → one LLM call decides   (model judgment, not regex)
                   4. audit event      (always, all modes)
```

### Rules — boundaries, not judgment
Domain/CIDR allow and deny lists in config (`[egress]` in gyre.toml + env overrides). Exact/suffix host matching only — **no pattern may judge intent**; that is the judge's job or the user's.

### Judge — judgment in the model
`judge` mode sends destination + method + tool name + a redacted request summary to the configured model: `{"allow": bool, "reason": "..."}`. Fail closed on parse failure (same rule as `routine_test`). This is CrabTrap's Phase-3 LLM gatekeeper, in-process.

### Audit — observation first
`egress_events` table via the `Database` trait (**both backends** — postgres + libsql, per CLAUDE.md). Fields: ts, tool, method, host, path, decision, mode, rule_or_judge_reason, leak_verdict. Surfaced later via `gyre egress log` + a gateway tab. Event-sourced `job_actions` already covers tool-call decisions; this adds the network layer.

## Integration points
- `src/tools/builtin/http.rs` — wrap the client call.
- `src/tools/builder/` built tools that get HTTP capability — same wrapper.
- WASM tools — already allowlisted per-tool (`tools/wasm/allowlist.rs`); the audit event should be emitted there too so ALL egress lands in one log.
- Shell (`curl` etc.) **cannot** be intercepted in-process — that is an OS boundary. Document the two options honestly: run main in the Docker sandbox profile, or set `HTTP_PROXY`/`HTTPS_PROXY` to a real CrabTrap instance (compose file to be shipped in `examples/crabtrap-proxy/`).

## Config sketch
```toml
[egress]
mode = "observe"            # observe | enforce | judge
allow = ["api.anthropic.com", "api.openai.com", "*.githubusercontent.com"]
deny  = []                   # deny wins over allow
judge_max_latency_ms = 3000  # judge timeout → fall back to enforce-deny
```

## Estimated scope
~300–500 LOC + Database-trait migration on both backends + tests. One focused session. Sequence: audit-only (observe) first — real value with zero blocking risk — then enforce, then judge.

## Non-goals (unchanged from sprint plan Rev 2/3)
No toxicity/bias classifier stacks; no regex-on-intent; no per-model special cases. The leak detector's secret patterns are structural (key formats), not semantic — they stay.
