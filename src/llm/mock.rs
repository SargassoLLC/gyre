//! Mock LLM provider for testing.

use async_trait::async_trait;
use rust_decimal::Decimal;

use crate::error::LlmError;
use crate::llm::provider::{
    CompletionRequest, CompletionResponse, FinishReason, LlmProvider, ToolCompletionRequest,
    ToolCompletionResponse,
};

/// A configurable mock LLM provider for unit and integration tests.
pub struct MockLlmProvider {
    pub response: String,
    pub delay_ms: u64,
    pub should_fail: bool,
}

impl MockLlmProvider {
    /// Create a mock that returns a successful response.
    pub fn success(response: &str) -> Self {
        Self {
            response: response.to_string(),
            delay_ms: 0,
            should_fail: false,
        }
    }

    /// Create a mock that always fails.
    pub fn failing() -> Self {
        Self {
            response: String::new(),
            delay_ms: 0,
            should_fail: true,
        }
    }

    /// Create a mock that responds after a delay (for timeout testing).
    pub fn slow(response: &str, delay_ms: u64) -> Self {
        Self {
            response: response.to_string(),
            delay_ms,
            should_fail: false,
        }
    }
}

#[async_trait]
impl LlmProvider for MockLlmProvider {
    fn model_name(&self) -> &str {
        "mock-model"
    }

    fn cost_per_token(&self) -> (Decimal, Decimal) {
        (Decimal::ZERO, Decimal::ZERO)
    }

    async fn complete(&self, _request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        if self.should_fail {
            return Err(LlmError::RequestFailed {
                provider: "mock".to_string(),
                reason: "mock failure".to_string(),
            });
        }

        if self.delay_ms > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(self.delay_ms)).await;
        }

        Ok(CompletionResponse {
            content: self.response.clone(),
            input_tokens: 10,
            output_tokens: 20,
            finish_reason: FinishReason::Stop,
            response_id: None,
        })
    }

    async fn complete_with_tools(
        &self,
        _request: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        if self.should_fail {
            return Err(LlmError::RequestFailed {
                provider: "mock".to_string(),
                reason: "mock failure".to_string(),
            });
        }

        Ok(ToolCompletionResponse {
            content: Some(self.response.clone()),
            tool_calls: vec![],
            input_tokens: 10,
            output_tokens: 20,
            finish_reason: FinishReason::Stop,
            response_id: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mock_constructors() {
        let s = MockLlmProvider::success("hello");
        assert_eq!(s.response, "hello");
        assert!(!s.should_fail);
        assert_eq!(s.delay_ms, 0);

        let f = MockLlmProvider::failing();
        assert!(f.should_fail);

        let sl = MockLlmProvider::slow("hi", 500);
        assert_eq!(sl.delay_ms, 500);
        assert!(!sl.should_fail);
    }
}
