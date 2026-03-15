//! Usage-tracking wrapper for LLM providers.
//!
//! Wraps any `LlmProvider` and records token usage to a `CostTracker`
//! after each completion call.

use std::sync::Arc;

use async_trait::async_trait;
use rust_decimal::Decimal;

use crate::error::LlmError;
use crate::llm::cost_tracker::CostTracker;
use crate::llm::provider::{
    CompletionRequest, CompletionResponse, LlmProvider, ModelMetadata, ToolCompletionRequest,
    ToolCompletionResponse,
};

/// Wraps an `LlmProvider` and records usage to a shared `CostTracker`.
pub struct TrackedProvider {
    inner: Arc<dyn LlmProvider>,
    tracker: Arc<CostTracker>,
    session_id: Option<String>,
}

impl TrackedProvider {
    /// Create a new tracked provider.
    pub fn new(
        inner: Arc<dyn LlmProvider>,
        tracker: Arc<CostTracker>,
        session_id: Option<String>,
    ) -> Self {
        Self {
            inner,
            tracker,
            session_id,
        }
    }
}

#[async_trait]
impl LlmProvider for TrackedProvider {
    fn model_name(&self) -> &str {
        self.inner.model_name()
    }

    fn cost_per_token(&self) -> (Decimal, Decimal) {
        self.inner.cost_per_token()
    }

    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let response = self.inner.complete(request).await?;
        self.tracker.record(
            &self.inner.active_model_name(),
            response.input_tokens,
            response.output_tokens,
            self.session_id.as_deref(),
        );
        Ok(response)
    }

    async fn complete_with_tools(
        &self,
        request: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        let response = self.inner.complete_with_tools(request).await?;
        self.tracker.record(
            &self.inner.active_model_name(),
            response.input_tokens,
            response.output_tokens,
            self.session_id.as_deref(),
        );
        Ok(response)
    }

    async fn list_models(&self) -> Result<Vec<String>, LlmError> {
        self.inner.list_models().await
    }

    async fn model_metadata(&self) -> Result<ModelMetadata, LlmError> {
        self.inner.model_metadata().await
    }

    fn active_model_name(&self) -> String {
        self.inner.active_model_name()
    }

    fn set_model(&self, model: &str) -> Result<(), LlmError> {
        self.inner.set_model(model)
    }

    fn seed_response_chain(&self, thread_id: &str, response_id: String) {
        self.inner.seed_response_chain(thread_id, response_id);
    }

    fn get_response_chain_id(&self, thread_id: &str) -> Option<String> {
        self.inner.get_response_chain_id(thread_id)
    }

    fn calculate_cost(&self, input_tokens: u32, output_tokens: u32) -> Decimal {
        self.inner.calculate_cost(input_tokens, output_tokens)
    }
}
