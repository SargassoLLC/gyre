# Running Gyre with Docker

## Quick Start (Simple — libSQL, no external DB)

```bash
cp deploy/env.example .env
# Edit .env: set ANTHROPIC_API_KEY and GATEWAY_AUTH_TOKEN
docker-compose -f docker-compose.simple.yml up
```

This runs a single container with an embedded libSQL database. Zero external dependencies.

## Quick Start (Full — PostgreSQL)

```bash
cp deploy/env.example .env
# Edit .env: set ANTHROPIC_API_KEY and GATEWAY_AUTH_TOKEN
docker-compose up
```

This starts PostgreSQL (pgvector) and Gyre. The agent waits for Postgres to be healthy before starting.

## Required Environment Variables

| Variable | Required? | Default | Description |
|----------|-----------|---------|-------------|
| `LLM_BACKEND` | Yes | `anthropic` | LLM provider (`anthropic` or `openai`) |
| `ANTHROPIC_API_KEY` | If anthropic | — | Anthropic API key |
| `OPENAI_API_KEY` | If openai | — | OpenAI API key |
| `GATEWAY_AUTH_TOKEN` | Yes | — | Bearer token for API access. Generate: `openssl rand -hex 32` |
| `DATABASE_BACKEND` | No | `libsql` | Database backend (`libsql` or `postgres`) |
| `DATABASE_URL` | If postgres | — | PostgreSQL connection string |
| `GATEWAY_HOST` | No | `0.0.0.0` | Gateway bind address |
| `GATEWAY_PORT` | No | `3000` | Gateway port |

See `deploy/env.example` for the full list of optional variables (embeddings, heartbeat, sandbox, resilience, etc.).

## Building Images

```bash
# Main agent
docker build -t gyre:latest .

# Worker (for Docker sandbox execution)
docker build -f Dockerfile.worker -t gyre-worker:latest .

# WASM build sandbox
docker build -f docker/sandbox.Dockerfile -t gyre-sandbox:latest .
```

## Health Check

All containers include a `HEALTHCHECK` instruction that runs `gyre health`. You can also run it manually:

```bash
docker exec <container> gyre health
```

This pings the gateway's `/api/health` endpoint and exits 0 if healthy, 1 if not.

## Volumes

| Volume | Purpose |
|--------|---------|
| `pgdata` | PostgreSQL data (full stack mode) |
| `gyre-data` | Gyre local data (libSQL database, tool storage, etc.) |

## Troubleshooting

**Container exits immediately**: Check that `.env` has valid `LLM_BACKEND` and API key values.

**Health check failing**: The gateway needs `GATEWAY_ENABLED=true` (default in env.example). Check logs with `docker-compose logs gyre`.

**PostgreSQL connection refused**: Ensure `DATABASE_URL` uses the Docker service name (`postgres`) as the host, not `localhost`.
