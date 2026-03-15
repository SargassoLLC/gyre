# Slack Channel Setup

This guide covers configuring the Slack channel for Gyre, including Socket Mode for tunnel-free operation.

## Overview

The Slack channel lets you interact with Gyre via Slack DMs and channels. It supports:

- **Socket Mode** (recommended): No tunnel or public URL required
- **HTTP webhook mode**: For production deployments behind a reverse proxy
- **DM pairing**: Approve unknown users before they can message the agent
- **Channel mentions**: `@YourApp` to trigger in channels

## Prerequisites

- Gyre installed and configured (`gyre onboard`)
- A Slack workspace where you have admin access (or permission to install apps)

## Quick Start

### 1. Create a Slack App

1. Go to [api.slack.com/apps](https://api.slack.com/apps)
2. Click **Create New App → From scratch**
3. Name it (e.g., "Gyre") and select your workspace
4. Click **Create App**

### 2. Configure Bot Scopes

Navigate to **OAuth & Permissions** in the left sidebar. Under **Bot Token Scopes**, add:

| Scope | Purpose |
|-------|---------|
| `chat:write` | Send messages |
| `im:read` | Read DMs |
| `im:write` | Open DM conversations |
| `im:history` | Access DM message history |
| `app_mentions:read` | Respond to @mentions in channels |

### 3. Enable Socket Mode

1. Navigate to **Socket Mode** in the left sidebar
2. Toggle **Enable Socket Mode** on
3. Create an app-level token with the `connections:write` scope
4. Copy the app-level token (starts with `xapp-`)

Socket Mode connects via WebSocket — no tunnel or public URL needed.

### 4. Enable Event Subscriptions

1. Navigate to **Event Subscriptions** in the left sidebar
2. Toggle **Enable Events** on
3. Under **Subscribe to bot events**, add:
   - `message.im` — DM messages sent to the bot
   - `app_mention` — @mentions in channels
4. Click **Save Changes**

### 5. Install the App

1. Navigate to **Install App** in the left sidebar
2. Click **Install to Workspace** and authorize
3. Copy the **Bot User OAuth Token** (starts with `xoxb-`)

### 6. Configure via Setup Wizard

```bash
gyre onboard
```

When prompted, enable the Slack channel and paste both tokens. The wizard will:

- Validate the bot token
- Configure Socket Mode with the app-level token
- Set up event subscriptions

### 7. (Alternative) Manual Configuration

Set the environment variables directly:

```bash
export SLACK_BOT_TOKEN="xoxb-your-bot-token"
export SLACK_APP_TOKEN="xapp-your-app-level-token"
```

## DM Pairing

When an unknown user DMs your bot, they receive a pairing code. You must approve them before they can message the agent.

### Flow

1. Unknown user sends a DM to your Slack app
2. Bot replies: `To pair with this bot, run: gyre pairing approve slack ABC12345`
3. You run: `gyre pairing approve slack ABC12345`
4. User is added to the allow list; future messages are delivered

### Commands

```bash
# List pending pairing requests
gyre pairing list slack

# List as JSON
gyre pairing list slack --json

# Approve a user by code
gyre pairing approve slack ABC12345
```

### Configuration

Edit `~/.gyre/channels/slack.capabilities.json` (or the config injected by the host):

| Option | Values | Default | Description |
|--------|--------|---------|-------------|
| `dm_policy` | `open`, `allowlist`, `pairing` | `pairing` | `open` = allow all; `allowlist` = config + approved only; `pairing` = allowlist + send pairing reply to unknown |
| `allow_from` | `["user_id", "username", "*"]` | `[]` | Pre-approved Slack user IDs or usernames. `*` allows everyone. |
| `owner_id` | Slack user ID | `null` | When set, only this user can message (overrides dm_policy) |
| `respond_to_all_channel_messages` | `true`/`false` | `false` | When true, respond to all channel messages; when false, only @mentions |

## Socket Mode vs HTTP Webhook Mode

| Feature | Socket Mode | HTTP Webhook |
|---------|-------------|-------------|
| Tunnel required | No | Yes |
| Setup complexity | Lower | Higher |
| Latency | ~100ms | ~50ms |
| Best for | Local dev, personal use | Production, behind load balancer |

To use HTTP webhook mode instead of Socket Mode:

1. Disable Socket Mode in your app settings
2. Set a **Request URL** under Event Subscriptions (e.g., `https://your-domain.com/slack/events`)
3. Configure a tunnel if running locally:
   ```bash
   ngrok http 8080
   ```
4. Set the tunnel URL: `TUNNEL_URL=https://your-ngrok-url`

## Manual Installation

If the channel isn't installed via the wizard:

```bash
# Build the Slack channel (requires wasm32-wasip2 target)
rustup target add wasm32-wasip2
./channels-src/slack/build.sh

# Install
mkdir -p ~/.gyre/channels
cp channels-src/slack/slack.wasm channels-src/slack/slack.capabilities.json ~/.gyre/channels/
```

## Secrets

The channel expects secrets named `slack_bot_token` and `slack_app_token`. Configure via:

- **Setup wizard**: Saves to encrypted secrets store
- **Environment**: `SLACK_BOT_TOKEN=xoxb-...` and `SLACK_APP_TOKEN=xapp-...`
- **Secrets store**: `gyre` CLI (if available)

## Troubleshooting

### Socket disconnects or "connection_closing"

- Slack drops idle sockets periodically — the channel reconnects automatically
- If reconnection fails, check that your `xapp-` token is valid and hasn't been revoked
- Check logs: `RUST_LOG=gyre=debug cargo run`

### "missing_scope" errors

- Navigate to **OAuth & Permissions** and verify all required scopes are added
- After adding scopes, you must **reinstall the app** to your workspace for the changes to take effect

### Messages not delivered

- Verify `message.im` is subscribed under **Event Subscriptions → Bot Events**
- For channel messages, verify `app_mention` is subscribed
- Check that Socket Mode is enabled if you're not using HTTP webhook mode

### Bot doesn't respond to @mentions

- Ensure `app_mentions:read` scope is added
- Ensure `app_mention` event is subscribed
- The bot must be invited to the channel: `/invite @YourApp`

### Pairing code not received

- Verify the bot has `chat:write` and `im:write` scopes
- Check `dm_policy` is `pairing` (not `allowlist` which blocks without reply)

### "not_authed" or "invalid_auth" errors

- Regenerate the bot token in **OAuth & Permissions → Install App**
- Ensure you're using the **Bot User OAuth Token** (`xoxb-`), not the user token (`xoxp-`)
