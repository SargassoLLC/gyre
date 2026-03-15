# Gyre — Feature Status

This document tracks the implementation status of Gyre's core features.

## Core Agent Features

| Feature | Status | Notes |
|---------|--------|-------|
| Multi-provider LLM | ✅ | Anthropic, OpenAI, Ollama, Gyre |
| LLM failover & retry | ✅ | Circuit breaker with backoff |
| Tool execution | ✅ | WASM sandboxed + native |
| Skill system | ✅ | SKILL.md-based skills |
| Workspace / memory | ✅ | PostgreSQL + libSQL backends |
| WASM channels | ✅ | Telegram, Discord, Slack, WhatsApp |
| MCP support | ✅ | Model Context Protocol client |
| Pairing / access control | ✅ | DM pairing codes |
| Heartbeat system | ✅ | Periodic agent check-ins |
| Sandbox jobs | ✅ | Docker-based sub-agents |
| OpenAI-compatible API | ✅ | /v1/chat/completions, /v1/models |
| HermitBox isolation | ✅ | WASM sandbox for tools |
| OS daemon / service | ✅ | launchd (macOS), systemd (Linux) |
| TUI interface | ✅ | Terminal UI channel |
| Web gateway | ✅ | SSE + WebSocket |

## Database Backends

| Backend | Status |
|---------|--------|
| PostgreSQL (full) | ✅ |
| libSQL / Turso | ✅ |
| SQLite-vec embeddings | 🔄 In progress |

## Channels

| Channel | Status |
|---------|--------|
| Telegram | ✅ |
| Discord | ✅ |
| Slack | ✅ |
| WhatsApp | ✅ |
| REPL / CLI | ✅ |
| TUI | ✅ |
| Web (SSE/WS) | ✅ |
| Custom WASM | ✅ |

## LLM Providers

| Provider | Status |
|---------|--------|
| Anthropic (Claude) | ✅ |
| OpenAI | ✅ |
| Ollama (local) | ✅ |
| Gyre | ✅ |
| OpenAI-compatible | ✅ |
