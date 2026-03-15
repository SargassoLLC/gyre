# Discord Channel Setup

This guide covers configuring the Discord channel for Gyre, including DM pairing for access control.

## Overview

The Discord channel lets you interact with Gyre via Discord DMs and server channels. It supports:

- **Bot integration**: Runs as a Discord bot in your server
- **DM pairing**: Approve unknown users before they can message the agent
- **Server channels**: Mention `@YourBot` or use slash commands to trigger in channels
- **Slash commands**: Register Discord slash commands for structured interaction

## Prerequisites

- Gyre installed and configured (`gyre onboard`)
- A Discord account
- A Discord server where you have "Manage Server" permissions

## Quick Start

### 1. Create a Discord Application

1. Go to the [Discord Developer Portal](https://discord.com/developers/applications)
2. Click **New Application**, give it a name (e.g., "Gyre"), and click **Create**
3. Navigate to **Bot** in the left sidebar
4. Click **Reset Token** and copy the bot token (e.g., `MTIzNDU2Nzg5.AbCdEf.GhIjKlMnOpQrStUvWxYz`)

### 2. Enable Required Intents

In the **Bot** settings page, scroll to **Privileged Gateway Intents** and enable:

- **Message Content Intent** (required — without this, the bot cannot read message text)
- **Server Members Intent** (optional — needed only if you want username-based allowlisting)

### 3. Add the Bot to Your Server

1. Navigate to **OAuth2 → URL Generator** in the left sidebar
2. Under **Scopes**, check `bot` and `applications.commands`
3. Under **Bot Permissions**, check:
   - Send Messages
   - Read Message History
   - Use Slash Commands
   - Embed Links (optional, for rich responses)
4. Copy the generated URL and open it in your browser
5. Select your server and click **Authorize**

### 4. Configure via Setup Wizard

```bash
gyre onboard
```

When prompted, enable the Discord channel and paste your bot token. The wizard will:

- Validate the token
- Register slash commands with Discord
- Configure DM policy

### 5. (Alternative) Manual Configuration

Set the environment variable directly:

```bash
export DISCORD_BOT_TOKEN="your_bot_token_here"
```

## DM Pairing

When an unknown user DMs your bot, they receive a pairing code. You must approve them before they can message the agent.

### Flow

1. Unknown user sends a DM to your bot
2. Bot replies: `To pair with this bot, run: gyre pairing approve discord ABC12345`
3. You run: `gyre pairing approve discord ABC12345`
4. User is added to the allow list; future messages are delivered

### Commands

```bash
# List pending pairing requests
gyre pairing list discord

# List as JSON
gyre pairing list discord --json

# Approve a user by code
gyre pairing approve discord ABC12345
```

### Configuration

Edit `~/.gyre/channels/discord.capabilities.json` (or the config injected by the host):

| Option | Values | Default | Description |
|--------|--------|---------|-------------|
| `dm_policy` | `open`, `allowlist`, `pairing` | `pairing` | `open` = allow all; `allowlist` = config + approved only; `pairing` = allowlist + send pairing reply to unknown |
| `allow_from` | `["user_id", "username#0000", "*"]` | `[]` | Pre-approved Discord user IDs or tags. `*` allows everyone. |
| `owner_id` | Discord user ID | `null` | When set, only this user can message (overrides dm_policy) |
| `guild_ids` | `["server_id"]` | `[]` | Restrict bot to specific servers. Empty = all servers. |
| `respond_to_all_channel_messages` | `true`/`false` | `false` | When true, respond to all channel messages; when false, only @mentions and slash commands |

## Slash Commands

If slash commands are registered during setup, the bot supports:

- `/ask <message>` — Send a message to Gyre
- `/status` — Check agent status
- `/jobs` — List active jobs

Slash commands are registered automatically when the bot starts. If they don't appear, try kicking and re-adding the bot, or wait up to an hour for Discord's cache to refresh.

## Manual Installation

If the channel isn't installed via the wizard:

```bash
# Build the Discord channel (requires wasm32-wasip2 target)
rustup target add wasm32-wasip2
./channels-src/discord/build.sh

# Install
mkdir -p ~/.gyre/channels
cp channels-src/discord/discord.wasm channels-src/discord/discord.capabilities.json ~/.gyre/channels/
```

## Secrets

The channel expects a secret named `discord_bot_token`. Configure via:

- **Setup wizard**: Saves to encrypted secrets store
- **Environment**: `DISCORD_BOT_TOKEN=your_token`
- **Secrets store**: `gyre` CLI (if available)

## Troubleshooting

### Bot is online but not responding to messages

- Verify **Message Content Intent** is enabled in the Developer Portal → Bot settings
- Check that the bot has **Read Message History** and **Send Messages** permissions in the channel
- If using DM pairing, ensure the user has been approved

### Slash commands not showing up

- Discord caches slash commands for up to one hour after registration
- Try restarting Discord (Ctrl+R) or re-adding the bot to the server
- Check logs for registration errors: `RUST_LOG=gyre=debug cargo run`

### Bot not appearing in server

- Verify you used the correct OAuth2 URL with `bot` and `applications.commands` scopes
- Ensure you selected the right server during authorization
- Check that the bot role has permissions in the channels you expect

### "Missing Access" or "Missing Permissions" errors

- The bot's role must be above any roles it needs to interact with
- Grant the bot a role with the required permissions, or use the channel-specific permission overrides

### Pairing code not received

- Verify the channel can send messages (bot has Send Messages permission in DMs)
- Check `dm_policy` is `pairing` (not `allowlist` which blocks without reply)

### "Connection refused" when starting

- Ensure your internet connection is active — Discord uses a WebSocket gateway
- Check that the bot token is valid and hasn't been regenerated
