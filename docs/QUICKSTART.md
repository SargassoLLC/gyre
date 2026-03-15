# Quickstart — Zero to Running in 5 Minutes

This guide gets you from nothing to a working Gyre agent that responds to messages.
By the end you'll have Gyre running in your terminal with persistent memory.

---

## What You Need

| Requirement | Notes |
|---|---|
| **macOS, Linux, or Windows WSL** | Native Windows: use the `.msi` installer instead |
| **An LLM** | [Claude Code](https://claude.ai/code) (zero-config) **or** an API key for Anthropic / OpenAI / Ollama |

No database setup required. Gyre uses an embedded libSQL database by default.

---

## Step 1 — Install Gyre

**macOS / Linux / WSL:**
```bash
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/sac916/gyre/releases/latest/download/gyre-installer.sh | sh
```

**Windows (PowerShell):**
```powershell
irm https://github.com/sac916/gyre/releases/latest/download/gyre-installer.ps1 | iex
```

**From source (Rust 1.85+):**
```bash
git clone https://github.com/sac916/gyre.git && cd gyre
cargo build --release
# Binary is at: target/release/gyre
```

**Verify:**
```bash
gyre --version
```

---

## Step 2 — Choose Your LLM

### Option A: Claude Code (zero-config, recommended)

If you have [Claude Code](https://claude.ai/code) installed, Gyre uses it automatically — no API key needed.

```bash
# Verify Claude Code is installed
claude --version
```

### Option B: API key

Set the appropriate env var before running `gyre setup`:

```bash
# Anthropic
export ANTHROPIC_API_KEY=sk-ant-...

# OpenAI
export OPENAI_API_KEY=sk-...

# Ollama (local, no key needed)
export LLM_BACKEND=ollama
export OLLAMA_MODEL=llama3.2   # or any model you've pulled
```

You can also set these in a `.env` file in your working directory.

---

## Step 3 — Run the Setup Wizard

```bash
gyre setup
```

The wizard takes about 2 minutes and handles:
- LLM provider selection and auth
- Agent identity (name, personality)
- Database location
- Optional channel connections (Telegram, Discord, Slack)

**Skip everything you don't need** — every setting has a sensible default.
You can re-run `gyre setup` at any time to change settings.

---

## Step 4 — Start the Agent

```bash
gyre run
```

You'll see the Gyre boot screen, then a `>` prompt. Type anything to talk to your agent:

```
> Hello! What can you help me with?
```

The agent will introduce itself and describe what it can do based on the tools
you configured during onboarding.

**Tip:** Type `/help` to see all available slash commands.

---

## Step 5 — Verify It's Working

Run these quick checks:

```bash
# 1. Check the agent responds
> What's today's date?

# 2. Check memory is persisting
> Remember that my name is [your name]

# (Start a new session)
gyre run

> What's my name?
# The agent should remember.
```

If memory recall works, your agent is fully operational. ✅

---

## What's Next

### Connect a Messaging Channel

Talk to Gyre from your phone or team workspace:

```bash
gyre setup channels   # guided channel setup
```

Available channels: [Telegram](TELEGRAM_SETUP.md) · [Discord](DISCORD_SETUP.md) · [Slack](SLACK_SETUP.md) · [WhatsApp](WHATSAPP_SETUP.md)

### Give the Agent a Personality

Create `SOUL.md` in your agent's home directory (`~/.gyre/agents/<name>/`):

```markdown
# SOUL.md

You are [name]. [Brief description of personality, tone, focus areas.]
```

Restart Gyre and the agent will adopt the new identity.

### Add Memory Context

Create `MEMORY.md` in the same directory to seed the agent with background knowledge:

```markdown
# MEMORY.md

- My timezone is EST
- I work on [project]
- Prefer concise responses
```

### Run the Code Examples

The `examples/` directory has working Rust code showing the core APIs:

```bash
# Hello world — open an agent and send a message
cargo run --example basic_agent

# Memory store and recall
cargo run --example memory_recall

# Multiple message sources (REPL + inject channel)
cargo run --example multi_channel

# Custom tool implementation
cargo run --example custom_tool

# Heartbeat system deep-dive (debugging tool)
cargo run --example test_heartbeat
```

### Docker Setup

For a persistent background service:

```bash
cp deploy/env.example .env   # fill in API keys
docker-compose -f docker-compose.simple.yml up -d
```

See [docs/DOCKER.md](DOCKER.md) for full container setup.

---

## Troubleshooting

### `gyre: command not found`

The installer adds Gyre to your PATH. Reload your shell:
```bash
source ~/.bashrc   # or ~/.zshrc on macOS
```

### `Error: No LLM provider configured`

Set an API key or install Claude Code:
```bash
export ANTHROPIC_API_KEY=sk-ant-...
gyre run
```

### `Error: Database locked`

Another Gyre instance is running. Find and stop it:
```bash
pkill -f "gyre run"
gyre run
```

### Agent doesn't remember things between sessions

Memory requires the database to be writable. Check:
```bash
ls -la ~/.gyre/
# gyre.db should exist and be readable/writable
```

If you're running in Docker, make sure the volume is mounted:
```yaml
volumes:
  - ~/.gyre:/root/.gyre
```

### LLM errors / timeouts

1. Verify your API key is valid
2. Check your internet connection
3. For Ollama: confirm the model is downloaded (`ollama list`)

---

## Still Stuck?

- 📖 [Full Documentation](https://github.com/sac916/gyre/wiki)
- 💬 [Community Discord](https://discord.gg/gyre)
- 🐛 [File an Issue](https://github.com/sac916/gyre/issues)
