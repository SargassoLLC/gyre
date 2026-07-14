//! Hook that grants automatic approval for a set of trusted tools.
//!
//! The worker's approval gate (`requires_approval()`) blocks tools that need
//! explicit user consent before execution. Memory tools
//! (`memory_search`, `memory_write`, `memory_read`, `memory_tree`) are
//! workspace-internal — they read and write the agent's own persistent store,
//! not external services. Routine jobs need them to do useful work, so this
//! hook vouches for a configurable set of tools via `trusted_tools()`,
//! allowing them through the approval gate during autonomous execution.
//!
//! This is a first-party consumer of the additive hook tool-trust mechanism
//! (`Hook::trusted_tools()` / `HookRegistry::is_tool_trusted`): registering it
//! can only extend trust, never drop a grant, and destructive parameter
//! combinations (`Tool::requires_approval_for`) still demand a human
//! regardless of trust. The hook itself is a lifecycle no-op; all work is
//! done through its `trusted_tools()` declaration.

use std::time::Duration;

use async_trait::async_trait;

use crate::hooks::hook::{Hook, HookContext, HookError, HookEvent, HookOutcome, HookPoint};

/// A hook that allows a fixed set of named tools to bypass the
/// `requires_approval()` gate during autonomous (routine / background) jobs.
///
/// Tool names are matched exactly. The hook itself is a lifecycle no-op;
/// all work is done through `trusted_tools()` declarations queried by
/// `HookRegistry::is_tool_trusted`.
pub struct TrustedToolsHook {
    /// Tool names that are unconditionally approved.
    trusted: Vec<String>,
}

impl TrustedToolsHook {
    /// Build a hook with the given set of trusted tool names.
    pub fn new(trusted_tools: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            trusted: trusted_tools.into_iter().map(|s| s.into()).collect(),
        }
    }

    /// Convenience constructor for the default memory-tool set:
    /// `memory_search`, `memory_write`, `memory_read`, `memory_tree`.
    ///
    /// These tools only access the workspace store that belongs to this
    /// agent/user — there is no external network request and no credential
    /// exposure, so granting them without interactive approval is safe by
    /// default.
    pub fn memory_tools() -> Self {
        Self::new([
            "memory_search",
            "memory_write",
            "memory_read",
            "memory_tree",
        ])
    }
}

#[async_trait]
impl Hook for TrustedToolsHook {
    fn name(&self) -> &str {
        "trusted_tools"
    }

    fn hook_points(&self) -> &[HookPoint] {
        // This hook runs at BeforeToolCall only to satisfy the trait contract;
        // its actual grant mechanism is trusted_tools(), not execute().
        &[HookPoint::BeforeToolCall]
    }

    fn timeout(&self) -> Duration {
        // Pure in-memory lookup — 100ms is generous.
        Duration::from_millis(100)
    }

    fn trusted_tools(&self) -> Vec<String> {
        self.trusted.clone()
    }

    async fn execute(
        &self,
        _event: &HookEvent,
        _ctx: &HookContext,
    ) -> Result<HookOutcome, HookError> {
        // The grant is declared via trusted_tools(); no runtime transformation needed.
        Ok(HookOutcome::ok())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool_call_event(name: &str) -> HookEvent {
        HookEvent::ToolCall {
            tool_name: name.to_string(),
            parameters: serde_json::json!({}),
            user_id: "test-user".to_string(),
            context: "job:test".to_string(),
        }
    }

    #[test]
    fn memory_tools_set_is_correct() {
        let hook = TrustedToolsHook::memory_tools();
        let trusted = hook.trusted_tools();
        assert!(trusted.contains(&"memory_search".to_string()));
        assert!(trusted.contains(&"memory_write".to_string()));
        assert!(trusted.contains(&"memory_read".to_string()));
        assert!(trusted.contains(&"memory_tree".to_string()));
        assert_eq!(trusted.len(), 4);
    }

    #[test]
    fn custom_trusted_set() {
        let hook = TrustedToolsHook::new(["my_internal_tool"]);
        let trusted = hook.trusted_tools();
        assert!(trusted.contains(&"my_internal_tool".to_string()));
        assert!(!trusted.contains(&"memory_search".to_string()));
    }

    #[tokio::test]
    async fn execute_is_a_noop() {
        let hook = TrustedToolsHook::memory_tools();
        let ctx = HookContext::default();
        let event = tool_call_event("memory_write");
        let result = hook.execute(&event, &ctx).await;
        assert!(result.is_ok());
        assert!(matches!(
            result.unwrap(),
            HookOutcome::Continue { modified: None }
        ));
    }

    #[tokio::test]
    async fn execute_is_noop_for_non_trusted_too() {
        let hook = TrustedToolsHook::memory_tools();
        let ctx = HookContext::default();
        let event = tool_call_event("shell");
        let result = hook.execute(&event, &ctx).await;
        assert!(result.is_ok());
    }

    #[test]
    fn hook_name_is_stable() {
        let hook = TrustedToolsHook::memory_tools();
        assert_eq!(hook.name(), "trusted_tools");
    }

    #[test]
    fn hook_points_covers_tool_call() {
        let hook = TrustedToolsHook::memory_tools();
        assert!(hook.hook_points().contains(&HookPoint::BeforeToolCall));
    }
}
