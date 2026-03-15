# Setup Specification

This document is the authoritative specification for Gyre's setup wizard.
Any code change to `src/setup/` **must** keep this document in sync.

---

## Entry Point

```
gyre setup [--quick] [--skip-risk-ack] [--reconfigure <stage>] [--headless <file>]
```

The `gyre setup` command replaces the former `gyre onboard` and `gyre init`
commands (both removed).

- `--quick`: QuickStart mode — auto-detects credentials, picks defaults,
  no interactive prompts. Completes in ~15 seconds on existing installs.
- `--skip-risk-ack`: Skip the risk acknowledgment stage.
- `--reconfigure <stage>`: Re-run a single stage by ID.
- `--headless <file>`: Non-interactive mode, reads answers from a JSON file.

Auto-detection via `check_onboard_needed()` in `main.rs` triggers on first
run when no database is configured. The `--no-onboard` flag suppresses this.

---

## Architecture

```
src/setup/
  mod.rs              — Module root, re-exports
  engine.rs           — SetupEngine orchestrator (stage sequencing, resume)
  ui.rs               — SetupUi (dialoguer wrapper, headless fallback)
  state.rs            — SetupState, StageStatus, DetectedConfig
  config_schema.rs    — MultiAgentSettings, AgentDefinition, etc.
  stages/
    mod.rs            — SetupStage trait + stage registry
    s01_risk_ack.rs   — Security warning + acknowledgment
    s02_detect.rs     — Config state detection (fresh/legacy/existing)
    s03_flow.rs       — QuickStart vs Advanced selection
    s04_database.rs   — Database setup (PostgreSQL + libSQL)
    s05_security.rs   — Secrets master key (keychain/env)
    s06_auth_model.rs — LLM provider + model selection
    s07_agents.rs     — Multi-agent creation (hermit boxes)
    s08_channels.rs   — Channel enable + per-agent binding
    s09_gateway.rs    — Web gateway + daemon installation
    s10_finalize.rs   — Persist config, print summary
```

---

## The 10-Stage Wizard

### Stage 1: Risk Acknowledgment
Security warning about system access. Skippable via `--skip-risk-ack`.

### Stage 2: Configuration Detection
Scans for `config.toml`, `.env`, agent boxes, API keys. Loads existing
settings from TOML (or legacy JSON). Classifies as Fresh/Legacy/Existing.

### Stage 3: Flow Selection
QuickStart vs Advanced. QuickStart auto-selected when `--quick` is passed.

### Stage 4: Database Setup
PostgreSQL or libSQL. QuickStart auto-picks libSQL at default path.
Writes bootstrap vars to `~/.gyre/.env`.

### Stage 5: Security
Secrets master key: OS Keychain, env var, or skip.
QuickStart auto-accepts existing keychain key or generates a new one.

### Stage 6: Auth & Model Selection
Auto-detects credentials in QuickStart mode:
1. Claude.ai subscription OAuth token (from Claude Code keychain)
2. `ANTHROPIC_API_KEY` env var
3. `OPENAI_API_KEY` env var
4. Existing backend from config.toml

Preserves existing model selection; only sets default if none configured.
Embeddings configuration is Advanced-only.

### Stage 7: Agent Creation
Creates hermit box with soul.md, TELOS/, memory/, knowledge/, axioms/.
QuickStart creates a single "gyre" agent with defaults.
Advanced mode supports multi-agent loop with personality Q&A.

### Stage 8: Channels & Bindings (skipped in QuickStart)
Discover available channels, configure per-channel settings,
bind channels to agents.

### Stage 9: Gateway & Daemon (skipped in QuickStart)
Web gateway port/host/auth. Daemon installation (launchd/systemd).
Heartbeat configuration.

### Stage 10: Finalization
Writes `config.toml`, `.env`, and database settings.
Prints configuration summary and next steps.

---

## SetupStage Trait

```rust
#[async_trait]
pub trait SetupStage: Send + Sync {
    fn id(&self) -> &'static str;
    fn name(&self) -> &'static str;
    fn skippable_in_quickstart(&self) -> bool { false }
    async fn is_satisfied(&self, state: &SetupState) -> bool { false }
    async fn run(&self, state: &mut SetupState, ui: &SetupUi)
        -> Result<StageOutcome, SetupError>;
}
```

Stages 8 and 9 return `skippable_in_quickstart() = true`.

---

## Settings Persistence

### Two-Layer Architecture

**Layer 1: `~/.gyre/.env`** — Bootstrap vars needed before DB connection:
```env
DATABASE_BACKEND=libsql
LIBSQL_PATH=/Users/name/.gyre/gyre.db
```

**Layer 2: `~/.gyre/config.toml`** — All settings in TOML format.

**Layer 3: Database `settings` table** — Key-value pairs for runtime access.

### Config Load Priority

`env var > config.toml > database settings > defaults`

---

## Platform Caveats

### macOS Keychain
- `get_generic_password()` triggers system dialogs (unlock + authorize)
- Cache the result after first access to avoid repeat prompts
- Never probe keychain in read-only commands
- Service name: `"gyre"`, account: `"master_key"`

### OAuth Token Authentication
Claude.ai subscription tokens (`sk-ant-oat01-*`) require stealth headers:
- `Authorization: Bearer <token>` (not `x-api-key`)
- `anthropic-beta: claude-code-20250219,oauth-2025-04-20`
- `user-agent: claude-cli/2.1.2 (external, cli)`
- `x-app: cli`
- System prompt must start with: "You are Claude Code, Anthropic's official CLI for Claude."

Implemented in `src/llm/mod.rs` via `OAuthHttpClient`.

---

## Testing

```bash
# Quick setup test (non-interactive)
gyre setup --quick --skip-risk-ack

# Run setup unit tests
cargo test --lib -- setup

# Headless mode (CI)
gyre setup --headless answers.json
```

---

## Modification Checklist

1. Update this README first
2. If adding a stage: implement `SetupStage`, add to `stages::all_stages()`
3. If adding settings: extend `Settings` struct, add TOML serialization
4. If touching keychain: cache results, test on macOS
5. Run:
   ```bash
   cargo fmt
   cargo clippy --all --all-features
   cargo test --lib -- setup
   ```
