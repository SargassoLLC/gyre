<p align="center">
  <img src="gyre.png" alt="Gyre" width="200"/>
</p>

<h1 align="center">Gyre</h1>

<p align="center">
  <strong>Your secure personal AI assistant, always on your side</strong>
</p>

<p align="center">
  <a href="#quick-start">Quick Start</a> •
  <a href="#philosophy">Philosophy</a> •
  <a href="#features">Features</a> •
  <a href="#installation">Installation</a> •
  <a href="#configuration">Configuration</a> •
  <a href="#security">Security</a> •
  <a href="#architecture">Architecture</a>
</p>

---

## Quick Start

```bash
# Install
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/sac916/gyre/releases/latest/download/gyre-installer.sh | sh

# Set up (one-time wizard, ~2 min)
gyre setup

# Run
gyre run
```

Gyre uses libSQL by default — no external database needed. Your data lives at `~/.gyre/gyre.db`.

Requires [Claude Code](https://claude.ai/code) for zero-config auth, **or** set `ANTHROPIC_API_KEY` / `OPENAI_API_KEY` / configure Ollama before running `gyre setup`.

📖 **[Full Quickstart Guide →](docs/QUICKSTART.md)** — step-by-step with troubleshooting, channel setup, and examples.

## Philosophy

Gyre is built on a simple principle: **your AI assistant should work for you, not against you**.

In a world where AI systems are increasingly opaque about data handling and aligned with corporate interests, Gyre takes a different approach:

- **Your data stays yours** - All information is stored locally, encrypted, and never leaves your control
- **Transparency by design** - Open source, auditable, no hidden telemetry or data harvesting
- **Self-expanding capabilities** - Build new tools on the fly without waiting for vendor updates
- **Defense in depth** - Multiple security layers protect against prompt injection and data exfiltration

Gyre is the AI assistant you can actually trust with your personal and professional life.

## Features

### Security First

- **WASM Sandbox** - Untrusted tools run in isolated WebAssembly containers with capability-based permissions
- **Credential Protection** - Secrets are never exposed to tools; injected at the host boundary with leak detection
- **Prompt Injection Defense** - Pattern detection, content sanitization, and policy enforcement
- **Endpoint Allowlisting** - HTTP requests only to explicitly approved hosts and paths

### Always Available

- **Multi-channel** - REPL, HTTP webhooks, WASM channels (Telegram, Slack), and web gateway
- **Docker Sandbox** - Isolated container execution with per-job tokens and orchestrator/worker pattern
- **Web Gateway** - Browser UI with real-time SSE/WebSocket streaming
- **Routines** - Cron schedules, event triggers, webhook handlers for background automation
- **Heartbeat System** - Proactive background execution for monitoring and maintenance tasks
- **Parallel Jobs** - Handle multiple requests concurrently with isolated contexts
- **Self-repair** - Automatic detection and recovery of stuck operations

### Self-Expanding

- **Dynamic Tool Building** - Describe what you need, and Gyre builds it as a WASM tool
- **MCP Protocol** - Connect to Model Context Protocol servers for additional capabilities
- **Plugin Architecture** - Drop in new WASM tools and channels without restarting

### Persistent Memory

- **Hybrid Search** - Full-text + vector search using Reciprocal Rank Fusion
- **Workspace Filesystem** - Flexible path-based storage for notes, logs, and context
- **Identity Files** - Maintain consistent personality and preferences across sessions

## Installation

### Prerequisites

- **To install from binary**: `curl` (macOS, Linux, WSL) or PowerShell (Windows)
- **To build from source**: Rust 1.85+
- **For LLM**: [Claude Code](https://claude.ai/code) installed (zero-config OAuth) OR an API key for any supported provider (Anthropic, OpenAI, Ollama, etc.)

No external database is required. Gyre ships with an embedded libSQL database that is auto-created at `~/.gyre/gyre.db`.

## Download or Build

Visit [Releases page](https://github.com/sac916/gyre/releases/) to see the latest updates.

<details>
  <summary>Install via Windows Installer (Windows)</summary>

Download the [Windows Installer](https://github.com/sac916/gyre/releases/latest/download/gyre-x86_64-pc-windows-msvc.msi) and run it.

</details>

<details>
  <summary>Install via powershell script (Windows)</summary>

```sh
irm https://github.com/sac916/gyre/releases/latest/download/gyre-installer.ps1 | iex
```

</details>

<details>
  <summary>Install via shell script (macOS, Linux, Windows/WSL)</summary>

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/sac916/gyre/releases/latest/download/gyre-installer.sh | sh
```
</details>

<details>
  <summary>Compile the source code (Cargo on Windows, Linux, macOS)</summary>

Install it with `cargo`, just make sure you have [Rust](https://rustup.rs) installed on your computer.

```bash
# Clone the repository
git clone https://github.com/sac916/gyre.git
cd gyre

# Build
cargo build --release

# Run tests
cargo test
```

For **full release** (after modifying channel sources), run `./scripts/build-all.sh` to rebuild channels first.

</details>

### Docker

The fastest way to get Gyre running:

```bash
cp deploy/env.example .env
# Edit .env with your API keys
docker-compose -f docker-compose.simple.yml up   # single container, libSQL
# OR
docker-compose up                                 # full stack with PostgreSQL
```

See [docs/DOCKER.md](docs/DOCKER.md) for details, env var reference, and troubleshooting.

### Channel Setup

Connect Gyre to your preferred messaging platform:

- [Telegram](docs/TELEGRAM_SETUP.md) — Recommended, easiest setup
- [Discord](docs/DISCORD_SETUP.md) — Great for communities and servers
- [Slack](docs/SLACK_SETUP.md) — For workspace and team use
- [WhatsApp](docs/WHATSAPP_SETUP.md) — Via WhatsApp Business API

### PostgreSQL (optional)

PostgreSQL is **not required** — Gyre defaults to an embedded libSQL database. If you prefer PostgreSQL for production use:

```bash
# Create database
createdb gyre

# Enable pgvector
psql gyre -c "CREATE EXTENSION IF NOT EXISTS vector;"

# Tell Gyre to use it
export DATABASE_BACKEND=postgres
export DATABASE_URL=postgres://user:pass@localhost/gyre
```

## Configuration

Run the setup wizard to configure Gyre:

```bash
gyre setup
```

The wizard handles database backend selection, LLM provider configuration,
and secrets encryption (using your system keychain). All settings are stored
in the database (migrated from `~/.gyre/settings.json` on first run).

## Security

Gyre implements defense in depth to protect your data and prevent misuse.

### WASM Sandbox

All untrusted tools run in isolated WebAssembly containers:

- **Capability-based permissions** - Explicit opt-in for HTTP, secrets, tool invocation
- **Endpoint allowlisting** - HTTP requests only to approved hosts/paths
- **Credential injection** - Secrets injected at host boundary, never exposed to WASM code
- **Leak detection** - Scans requests and responses for secret exfiltration attempts
- **Rate limiting** - Per-tool request limits to prevent abuse
- **Resource limits** - Memory, CPU, and execution time constraints

```
WASM ──► Allowlist ──► Leak Scan ──► Credential ──► Execute ──► Leak Scan ──► WASM
         Validator     (request)     Injector       Request     (response)
```

### Prompt Injection Defense

External content passes through multiple security layers:

- Pattern-based detection of injection attempts
- Content sanitization and escaping
- Policy rules with severity levels (Block/Warn/Review/Sanitize)
- Tool output wrapping for safe LLM context injection

### Data Protection

- All data stored locally (libSQL at `~/.gyre/gyre.db` or your own PostgreSQL)
- Secrets encrypted with AES-256-GCM
- No telemetry, analytics, or data sharing
- Full audit log of all tool executions

## Architecture

```
┌────────────────────────────────────────────────────────────────┐
│                          Channels                              │
│  ┌──────┐  ┌──────┐   ┌─────────────┐  ┌─────────────┐         │
│  │ REPL │  │ HTTP │   │WASM Channels│  │ Web Gateway │         │
│  └──┬───┘  └──┬───┘   └──────┬──────┘  │ (SSE + WS)  │         │
│     │         │              │         └──────┬──────┘         │
│     └─────────┴──────────────┴────────────────┘                │
│                              │                                 │
│                    ┌─────────▼─────────┐                       │
│                    │    Agent Loop     │  Intent routing       │
│                    └────┬──────────┬───┘                       │
│                         │          │                           │
│              ┌──────────▼────┐  ┌──▼───────────────┐           │
│              │  Scheduler    │  │ Routines Engine  │           │
│              │(parallel jobs)│  │(cron, event, wh) │           │
│              └──────┬────────┘  └────────┬─────────┘           │
│                     │                    │                     │
│       ┌─────────────┼────────────────────┘                     │
│       │             │                                          │
│   ┌───▼─────┐  ┌────▼────────────────┐                         │
│   │ Local   │  │    Orchestrator     │                         │
│   │Workers  │  │  ┌───────────────┐  │                         │
│   │(in-proc)│  │  │ Docker Sandbox│  │                         │
│   └───┬─────┘  │  │   Containers  │  │                         │
│       │        │  │ ┌───────────┐ │  │                         │
│       │        │  │ │Worker / CC│ │  │                         │
│       │        │  │ └───────────┘ │  │                         │
│       │        │  └───────────────┘  │                         │
│       │        └─────────┬───────────┘                         │
│       └──────────────────┤                                     │
│                          │                                     │
│              ┌───────────▼──────────┐                          │
│              │    Tool Registry     │                          │
│              │  Built-in, MCP, WASM │                          │
│              └──────────────────────┘                          │
└────────────────────────────────────────────────────────────────┘
```

### Core Components

| Component | Purpose |
|-----------|---------|
| **Agent Loop** | Main message handling and job coordination |
| **Router** | Classifies user intent (command, query, task) |
| **Scheduler** | Manages parallel job execution with priorities |
| **Worker** | Executes jobs with LLM reasoning and tool calls |
| **Orchestrator** | Container lifecycle, LLM proxying, per-job auth |
| **Web Gateway** | Browser UI with chat, memory, jobs, logs, extensions, routines |
| **Routines Engine** | Scheduled (cron) and reactive (event, webhook) background tasks |
| **Workspace** | Persistent memory with hybrid search |
| **Safety Layer** | Prompt injection defense and content sanitization |

## Usage

```bash
# First-time setup
gyre setup

# Start interactive REPL
gyre run

# With debug logging
RUST_LOG=gyre=debug gyre run
```

## Development

```bash
# Format code
cargo fmt

# Lint
cargo clippy --all --benches --tests --examples --all-features

# Run tests
cargo test

# Run specific test
cargo test test_name
```

- **Channel sources**: Run `./scripts/build-all.sh` before `cargo build` to rebuild WASM channels.

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT License ([LICENSE-MIT](LICENSE-MIT))

at your option.
