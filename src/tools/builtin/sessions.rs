//! Inter-session messaging tools.
//!
//! `sessions_send` lets an agent (a subagent job, a routine, or the main
//! session itself) deliver a message to a user's channels IMMEDIATELY via
//! the channel manager. There is deliberately no queue: the reference
//! implementation's failure mode was messages to a session that wasn't in
//! an active conversation queuing silently and never reaching a delivery
//! channel. Here delivery is attempted synchronously and failures are
//! returned to the caller as tool errors — visible, never swallowed.
//!
//! `sessions_list` enumerates active sessions so agents can see who/what
//! they can address.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;

use crate::agent::SessionManager;
use crate::channels::{ChannelManager, OutgoingResponse};
use crate::context::JobContext;
use crate::tools::tool::{Tool, ToolError, ToolOutput};

/// Tool: send a message to a user's channels, delivered now.
pub struct SessionsSendTool {
    channels: Arc<ChannelManager>,
}

impl SessionsSendTool {
    pub fn new(channels: Arc<ChannelManager>) -> Self {
        Self { channels }
    }
}

#[async_trait]
impl Tool for SessionsSendTool {
    fn name(&self) -> &str {
        "sessions_send"
    }

    fn description(&self) -> &str {
        "Send a message to the user's channels right now (e.g. from a background \
         job or routine). Delivers immediately via the channel manager — there is \
         no queue. Returns which channels the message was delivered to; fails \
         loudly if no channel accepted it."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "message": {
                    "type": "string",
                    "description": "The message content to deliver."
                },
                "channel": {
                    "type": "string",
                    "description": "Deliver on this channel only (e.g. \"telegram\", \"gateway\"). \
                                    If omitted or delivery on it fails, all channels are tried."
                },
                "user": {
                    "type": "string",
                    "description": "Target user ID. Defaults to the current session's user."
                }
            },
            "required": ["message"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let message = params
            .get("message")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| ToolError::InvalidParameters(
                "'message' is required and must be a non-empty string".to_string(),
            ))?;

        let user = params
            .get("user")
            .and_then(|v| v.as_str())
            .unwrap_or(&ctx.user_id)
            .to_string();

        let target_channel = params.get("channel").and_then(|v| v.as_str());

        let response = OutgoingResponse {
            content: message.to_string(),
            thread_id: None,
            metadata: json!({
                "source": "sessions_send",
                "from_job": ctx.job_id.to_string(),
            }),
        };

        // Targeted delivery first, if requested.
        if let Some(channel) = target_channel {
            match self
                .channels
                .broadcast(channel, &user, response.clone())
                .await
            {
                Ok(()) => {
                    return Ok(ToolOutput::text(
                        format!("Delivered to '{}' for user '{}'.", channel, user),
                        start.elapsed(),
                    ));
                }
                Err(e) => {
                    tracing::warn!(
                        channel = %channel,
                        "sessions_send targeted delivery failed ({}), trying all channels",
                        e
                    );
                }
            }
        }

        // Broadcast to all channels; report exactly what happened.
        let results = self.channels.broadcast_all(&user, response).await;

        if results.is_empty() {
            return Err(ToolError::ExecutionFailed(
                "No channels are registered; the message was NOT delivered.".to_string(),
            ));
        }

        let (delivered, failed): (Vec<_>, Vec<_>) =
            results.into_iter().partition(|(_, r)| r.is_ok());

        if delivered.is_empty() {
            let reasons = failed
                .iter()
                .map(|(ch, r)| match r {
                    Err(e) => format!("{}: {}", ch, e),
                    Ok(()) => unreachable!("partitioned on is_ok"),
                })
                .collect::<Vec<_>>()
                .join("; ");
            return Err(ToolError::ExecutionFailed(format!(
                "Delivery failed on every channel — the message did NOT reach \
                 the user. Errors: {}",
                reasons
            )));
        }

        let delivered_names = delivered
            .iter()
            .map(|(ch, _)| ch.as_str())
            .collect::<Vec<_>>()
            .join(", ");

        // NOTE: channels that don't implement broadcast() return Ok as a
        // no-op (trait default), so "accepted" is per-channel best effort.
        let mut summary = format!(
            "Message accepted for user '{}' on: {}.",
            user, delivered_names
        );
        if !failed.is_empty() {
            let failed_names = failed
                .iter()
                .map(|(ch, _)| ch.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            summary.push_str(&format!(" Failed on: {}.", failed_names));
        }

        Ok(ToolOutput::text(summary, start.elapsed()))
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}

/// Tool: list active sessions.
pub struct SessionsListTool {
    session_manager: Arc<SessionManager>,
}

impl SessionsListTool {
    pub fn new(session_manager: Arc<SessionManager>) -> Self {
        Self { session_manager }
    }
}

#[async_trait]
impl Tool for SessionsListTool {
    fn name(&self) -> &str {
        "sessions_list"
    }

    fn description(&self) -> &str {
        "List active sessions: user, session id, thread count, last activity, \
         and whether any thread is awaiting a tool approval."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({ "type": "object", "properties": {} })
    }

    async fn execute(
        &self,
        _params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let sessions = self.session_manager.list_sessions().await;
        let listing = serde_json::to_string_pretty(&sessions).map_err(|e| {
            ToolError::ExecutionFailed(format!("failed to serialize session list: {e}"))
        })?;

        Ok(ToolOutput::text(listing, start.elapsed()))
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channels::{Channel, IncomingMessage, MessageStream};
    use crate::error::ChannelError;
    use std::sync::Mutex as StdMutex;

    fn test_ctx() -> JobContext {
        JobContext::with_user("user-1", "job for tests", "test job description")
    }

    /// A channel that records broadcasts.
    struct RecordingChannel {
        name: String,
        sent: Arc<StdMutex<Vec<(String, String)>>>,
        fail: bool,
    }

    #[async_trait]
    impl Channel for RecordingChannel {
        fn name(&self) -> &str {
            &self.name
        }
        async fn start(&self) -> Result<MessageStream, ChannelError> {
            Ok(Box::pin(futures::stream::empty::<IncomingMessage>()))
        }
        async fn respond(
            &self,
            _msg: &IncomingMessage,
            _response: OutgoingResponse,
        ) -> Result<(), ChannelError> {
            Ok(())
        }
        async fn broadcast(
            &self,
            user_id: &str,
            response: OutgoingResponse,
        ) -> Result<(), ChannelError> {
            if self.fail {
                return Err(ChannelError::SendFailed {
                    name: self.name.clone(),
                    reason: "simulated failure".to_string(),
                });
            }
            self.sent
                .lock()
                .unwrap()
                .push((user_id.to_string(), response.content));
            Ok(())
        }
        async fn health_check(&self) -> Result<(), ChannelError> {
            Ok(())
        }
    }

    fn manager_with(channels: Vec<RecordingChannel>) -> Arc<ChannelManager> {
        let mut mgr = ChannelManager::new();
        for ch in channels {
            mgr.add(Box::new(ch));
        }
        Arc::new(mgr)
    }

    #[tokio::test]
    async fn send_delivers_to_targeted_channel() {
        let sent = Arc::new(StdMutex::new(Vec::new()));
        let mgr = manager_with(vec![RecordingChannel {
            name: "telegram".into(),
            sent: Arc::clone(&sent),
            fail: false,
        }]);

        let tool = SessionsSendTool::new(mgr);
        let out = tool
            .execute(
                json!({"message": "hello", "channel": "telegram"}),
                &test_ctx(),
            )
            .await
            .unwrap();

        assert!(out.result.as_str().unwrap_or_default().contains("telegram"));
        let sent = sent.lock().unwrap();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].1, "hello");
        // Defaulted to the calling context's user.
        assert_eq!(sent[0].0, "user-1");
    }

    // The bug this tool exists to prevent: a message with no live
    // delivery path must FAIL LOUDLY, never queue silently.
    #[tokio::test]
    async fn send_fails_loudly_with_no_channels() {
        let tool = SessionsSendTool::new(Arc::new(ChannelManager::new()));
        let err = tool
            .execute(json!({"message": "hello"}), &test_ctx())
            .await
            .unwrap_err();
        assert!(err.to_string().contains("NOT delivered"));
    }

    #[tokio::test]
    async fn send_fails_loudly_when_all_channels_fail() {
        let mgr = manager_with(vec![RecordingChannel {
            name: "telegram".into(),
            sent: Arc::new(StdMutex::new(Vec::new())),
            fail: true,
        }]);

        let tool = SessionsSendTool::new(mgr);
        let err = tool
            .execute(json!({"message": "hello"}), &test_ctx())
            .await
            .unwrap_err();
        assert!(err.to_string().contains("did NOT reach"));
    }

    // Targeted channel down → falls back to the other channels rather
    // than dropping the message.
    #[tokio::test]
    async fn send_falls_back_when_target_fails() {
        let sent = Arc::new(StdMutex::new(Vec::new()));
        let mgr = manager_with(vec![
            RecordingChannel {
                name: "telegram".into(),
                sent: Arc::new(StdMutex::new(Vec::new())),
                fail: true,
            },
            RecordingChannel {
                name: "gateway".into(),
                sent: Arc::clone(&sent),
                fail: false,
            },
        ]);

        let tool = SessionsSendTool::new(mgr);
        let out = tool
            .execute(
                json!({"message": "urgent", "channel": "telegram"}),
                &test_ctx(),
            )
            .await
            .unwrap();

        assert!(out.result.as_str().unwrap_or_default().contains("gateway"));
        assert_eq!(sent.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn send_rejects_empty_message() {
        let tool = SessionsSendTool::new(Arc::new(ChannelManager::new()));
        let err = tool
            .execute(json!({"message": "  "}), &test_ctx())
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidParameters(_)));
    }

    #[tokio::test]
    async fn list_sessions_reports_users() {
        let sm = Arc::new(SessionManager::new());
        let _ = sm.get_or_create_session("alice").await;
        let _ = sm.get_or_create_session("bob").await;

        let tool = SessionsListTool::new(sm);
        let out = tool.execute(json!({}), &test_ctx()).await.unwrap();
        let content = out.result.as_str().unwrap_or_default().to_string();
        assert!(content.contains("alice"));
        assert!(content.contains("bob"));
    }
}
