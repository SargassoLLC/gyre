# WhatsApp Channel Setup

This guide covers configuring the WhatsApp channel for Gyre via the WhatsApp Business Cloud API.

## Overview

The WhatsApp channel lets you interact with Gyre via WhatsApp messages. It supports:

- **Cloud API**: Hosted by Meta — no server infrastructure required
- **Webhook delivery**: Incoming messages delivered via HTTP webhook
- **DM pairing**: Approve unknown users before they can message the agent
- **24-hour messaging window**: Reply to users within 24 hours of their last message

## Prerequisites

- Gyre installed and configured (`gyre onboard`)
- A [Meta Developer account](https://developers.facebook.com/)
- Access to WhatsApp Business API (free tier available for testing)

## Quick Start

### 1. Create a Meta App

1. Go to [developers.facebook.com](https://developers.facebook.com/) and log in
2. Click **My Apps → Create App**
3. Select **Business** as the app type
4. Name it (e.g., "Gyre WhatsApp") and click **Create App**

### 2. Set Up WhatsApp Business API

1. In your app dashboard, click **Add Product** and select **WhatsApp**
2. Click **Set Up** to configure WhatsApp Business
3. Follow the prompts to link or create a WhatsApp Business Account
4. Meta provides a **test phone number** you can use immediately for development

### 3. Get Your Credentials

From the WhatsApp section of your app dashboard:

- **Phone Number ID**: Found under **API Setup** (e.g., `123456789012345`)
- **Permanent Access Token**: Generate under **API Setup → Generate Access Token**
  - For production, create a System User in Business Settings and generate a permanent token

> **Note**: The temporary token from the dashboard expires after 24 hours. For persistent use, create a permanent token via a System User.

### 4. Configure the Webhook

Incoming messages are delivered via webhook. You need a publicly accessible URL.

1. Start a tunnel for local development:
   ```bash
   # ngrok
   ngrok http 8080

   # Cloudflare
   cloudflared tunnel --url http://localhost:8080
   ```

2. In your app dashboard, navigate to **WhatsApp → Configuration**
3. Under **Webhook**, click **Edit** and enter:
   - **Callback URL**: `https://your-tunnel-url/whatsapp/webhook`
   - **Verify Token**: A secret string you choose (e.g., `gyre_verify_abc123`)
4. Click **Verify and Save**
5. Under **Webhook Fields**, subscribe to `messages`

### 5. Configure via Setup Wizard

```bash
gyre onboard
```

When prompted, enable the WhatsApp channel and provide:
- Phone Number ID
- Access Token
- Webhook Verify Token

The wizard will:

- Validate the credentials
- Store tokens in the encrypted secrets store
- Configure the webhook endpoint

### 6. (Alternative) Manual Configuration

Set the environment variables directly:

```bash
export WHATSAPP_PHONE_NUMBER_ID="123456789012345"
export WHATSAPP_ACCESS_TOKEN="EAABx..."
export WHATSAPP_WEBHOOK_VERIFY_TOKEN="gyre_verify_abc123"
```

## DM Pairing

When an unknown user messages your WhatsApp number, they receive a pairing code. You must approve them before they can message the agent.

### Flow

1. Unknown user sends a message to your WhatsApp Business number
2. Bot replies: `To pair with this bot, run: gyre pairing approve whatsapp ABC12345`
3. You run: `gyre pairing approve whatsapp ABC12345`
4. User is added to the allow list; future messages are delivered

### Commands

```bash
# List pending pairing requests
gyre pairing list whatsapp

# List as JSON
gyre pairing list whatsapp --json

# Approve a user by code
gyre pairing approve whatsapp ABC12345
```

### Configuration

Edit `~/.gyre/channels/whatsapp.capabilities.json` (or the config injected by the host):

| Option | Values | Default | Description |
|--------|--------|---------|-------------|
| `dm_policy` | `open`, `allowlist`, `pairing` | `pairing` | `open` = allow all; `allowlist` = config + approved only; `pairing` = allowlist + send pairing reply to unknown |
| `allow_from` | `["phone_number", "*"]` | `[]` | Pre-approved phone numbers (with country code, e.g., `+14155551234`). `*` allows everyone. |
| `owner_id` | Phone number | `null` | When set, only this number can message (overrides dm_policy) |

## Important Limitations

### 24-Hour Messaging Window

WhatsApp enforces a strict messaging policy:

- **User-initiated**: You can only reply to a user within **24 hours** of their last message
- **After 24 hours**: You must use a pre-approved **Message Template** to re-engage
- **Template messages**: Must be submitted to Meta for approval before use

This means Gyre can respond to incoming messages freely, but cannot proactively reach out unless the user has messaged within the last 24 hours.

### Rate Limits

- **Test numbers**: Limited to 5 recipients
- **Verified business**: Tiered limits (250 → 1K → 10K → 100K conversations/day)
- **API rate limit**: 80 messages/second for Cloud API

### Media Support

WhatsApp supports text, images, documents, and audio. The channel currently handles:

- **Inbound**: Text messages, image/document captions
- **Outbound**: Text messages (media responses planned)

## Manual Installation

If the channel isn't installed via the wizard:

```bash
# Build the WhatsApp channel (requires wasm32-wasip2 target)
rustup target add wasm32-wasip2
./channels-src/whatsapp/build.sh

# Install
mkdir -p ~/.gyre/channels
cp channels-src/whatsapp/whatsapp.wasm channels-src/whatsapp/whatsapp.capabilities.json ~/.gyre/channels/
```

## Secrets

The channel expects secrets named `whatsapp_access_token` and `whatsapp_webhook_verify_token`. Configure via:

- **Setup wizard**: Saves to encrypted secrets store
- **Environment**: `WHATSAPP_ACCESS_TOKEN=EAABx...` and `WHATSAPP_WEBHOOK_VERIFY_TOKEN=your_token`
- **Secrets store**: `gyre` CLI (if available)

## Troubleshooting

### Webhook verification fails

- Ensure the **Verify Token** in Meta's dashboard matches your `WHATSAPP_WEBHOOK_VERIFY_TOKEN` exactly
- The callback URL must be HTTPS — use a tunnel for local development
- Check that Gyre is running and the webhook endpoint is accessible

### Messages not delivered to Gyre

- Verify you subscribed to the `messages` webhook field
- Check that your tunnel is running and the URL hasn't changed
- Look for webhook delivery errors in the Meta App Dashboard under **Webhooks → Activity Log**

### "Message failed to send" or API errors

- Verify your access token is valid and hasn't expired (temporary tokens last 24 hours)
- Check that the Phone Number ID matches your WhatsApp Business number
- For "outside 24-hour window" errors, the user must message you first

### Template message required

- After 24 hours of inactivity, you cannot send free-form messages
- Create a message template in **WhatsApp Manager → Message Templates**
- Templates require Meta approval (usually within minutes for simple templates)

### Test number limitations

- Meta's test number can only message up to 5 verified recipient numbers
- Add test recipients under **API Setup → To** in the dashboard
- For broader use, verify your own business phone number

### "Invalid phone number" errors

- Phone numbers must include country code without `+` prefix in API calls
- The channel handles formatting, but verify the Phone Number ID (not the phone number itself) is correct
