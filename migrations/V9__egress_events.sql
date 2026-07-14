-- Egress events: audit log for the EgressPolicy layer.
--
-- Every outbound network decision from native tools (http, built tools,
-- WASM allowlist) lands here, in every mode (observe/enforce/judge).
-- See docs/design/egress-policy.md.

CREATE TABLE egress_events (
    id UUID PRIMARY KEY,
    ts TIMESTAMPTZ NOT NULL DEFAULT now(),
    tool TEXT NOT NULL,
    method TEXT NOT NULL,
    host TEXT NOT NULL,
    path TEXT NOT NULL DEFAULT '',
    decision TEXT NOT NULL,              -- 'allowed' | 'denied'
    mode TEXT NOT NULL,                  -- 'observe' | 'enforce' | 'judge' | 'rule' | 'leak-scan' | 'wasm-allowlist'
    reason TEXT NOT NULL DEFAULT '',     -- matched rule or judge reason
    leak_verdict TEXT NOT NULL DEFAULT 'clean'
);

CREATE INDEX idx_egress_events_ts ON egress_events(ts DESC);
CREATE INDEX idx_egress_events_host ON egress_events(host);
CREATE INDEX idx_egress_events_decision ON egress_events(decision);
