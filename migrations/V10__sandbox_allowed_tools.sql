-- Persist the per-job tool allowlist on sandbox job records.
--
-- This allows the restart handler to replay the original job's tool
-- restrictions instead of falling back to the configured default.
--
-- NULL  = job was not tool-restricted (any tool allowed by default config)
-- ","   = deny-all sentinel (empty effective list; preserved on restart)
-- "a,b" = comma-separated list of allowed tool patterns
ALTER TABLE agent_jobs ADD COLUMN IF NOT EXISTS allowed_tools TEXT;
