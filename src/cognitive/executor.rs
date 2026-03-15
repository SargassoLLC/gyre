//! Worker executor: runs a WorkerJob against a real LLM provider with timeout.

use std::fmt;
use std::time::Duration;

use crate::cognitive::orchestrator::{TribeOrchestrator, WorkerJob, WorkerJobStatus};
use crate::llm::{ChatMessage, CompletionRequest, LlmProvider};

/// Errors from worker execution.
///
/// **Security note:** `LlmError` stores a sanitized error message.
/// Internal details (connection strings, API keys) are stripped before storage.
#[derive(Debug)]
pub enum ExecutorError {
    Timeout,
    LlmError(String),
    EmptyResponse,
    JobNotReady,
}

/// Sanitize an LLM error message to prevent leaking internal details.
///
/// Strips content that looks like it contains API keys, connection strings,
/// or other sensitive infrastructure details. Returns a generic message
/// if the error is deemed sensitive.
fn sanitize_llm_error(err: &str) -> String {
    let lower = err.to_lowercase();
    // Detect patterns that suggest internal detail leakage
    let sensitive_patterns = [
        "api_key=",
        "apikey=",
        "bearer ",
        "authorization:",
        "password=",
        "secret=",
        "token=",
        "postgres://",
        "postgresql://",
        "mysql://",
        "redis://",
        "amqp://",
        "mongodb://",
        "sk-",   // OpenAI key prefix
        "sess_", // Gyre session prefix
    ];
    for pattern in &sensitive_patterns {
        if lower.contains(pattern) {
            return "LLM provider returned an error (details redacted)".to_string();
        }
    }
    // Truncate excessively long error messages
    const MAX_ERROR_LEN: usize = 512;
    if err.len() > MAX_ERROR_LEN {
        let mut end = MAX_ERROR_LEN;
        while !err.is_char_boundary(end) && end > 0 {
            end -= 1;
        }
        format!("{}…(truncated)", &err[..end])
    } else {
        err.to_string()
    }
}

impl fmt::Display for ExecutorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Timeout => write!(f, "Worker execution timed out"),
            Self::LlmError(e) => write!(f, "LLM error: {e}"),
            Self::EmptyResponse => write!(f, "LLM returned empty response"),
            Self::JobNotReady => write!(f, "Job is not in Pending/Running state"),
        }
    }
}

impl std::error::Error for ExecutorError {}

/// Executes a WorkerJob against an LLM provider.
pub struct WorkerExecutor;

impl WorkerExecutor {
    /// Run a worker job against the given LLM provider with a timeout.
    ///
    /// Builds a `CompletionRequest` using the job's system prompt and task,
    /// wraps the LLM call in `tokio::time::timeout`, and returns the response text.
    pub async fn run(
        job: &WorkerJob,
        llm: &dyn LlmProvider,
        timeout_secs: u64,
    ) -> Result<String, ExecutorError> {
        // Only run jobs that are Pending or Running
        match &job.status {
            WorkerJobStatus::Pending | WorkerJobStatus::Running => {}
            _ => return Err(ExecutorError::JobNotReady),
        }

        let system_prompt = TribeOrchestrator::worker_system_prompt(job);
        let messages = vec![
            ChatMessage::system(system_prompt),
            ChatMessage::user(&job.task),
        ];
        let request = CompletionRequest::new(messages);

        let result =
            tokio::time::timeout(Duration::from_secs(timeout_secs), llm.complete(request)).await;

        match result {
            Err(_elapsed) => Err(ExecutorError::Timeout),
            Ok(Err(llm_err)) => Err(ExecutorError::LlmError(sanitize_llm_error(
                &llm_err.to_string(),
            ))),
            Ok(Ok(response)) => {
                if response.content.trim().is_empty() {
                    Err(ExecutorError::EmptyResponse)
                } else {
                    Ok(response.content)
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_executor_error_display() {
        assert_eq!(
            ExecutorError::Timeout.to_string(),
            "Worker execution timed out"
        );
        assert_eq!(
            ExecutorError::LlmError("connection refused".to_string()).to_string(),
            "LLM error: connection refused"
        );
        assert_eq!(
            ExecutorError::EmptyResponse.to_string(),
            "LLM returned empty response"
        );
        assert_eq!(
            ExecutorError::JobNotReady.to_string(),
            "Job is not in Pending/Running state"
        );
    }

    #[test]
    fn test_sanitize_llm_error_safe_message() {
        let result = sanitize_llm_error("connection refused");
        assert_eq!(result, "connection refused");
    }

    #[test]
    fn test_sanitize_llm_error_redacts_api_key() {
        let result = sanitize_llm_error("request failed: api_key=sk-abc123 is invalid");
        assert_eq!(result, "LLM provider returned an error (details redacted)");
    }

    #[test]
    fn test_sanitize_llm_error_redacts_connection_string() {
        let result = sanitize_llm_error("connection to postgres://user:pass@host/db failed");
        assert_eq!(result, "LLM provider returned an error (details redacted)");
    }

    #[test]
    fn test_sanitize_llm_error_redacts_bearer_token() {
        let result = sanitize_llm_error("Authorization: Bearer eyJhbGciOiJIUzI1NiJ9...");
        assert_eq!(result, "LLM provider returned an error (details redacted)");
    }

    #[test]
    fn test_sanitize_llm_error_truncates_long_messages() {
        let long_msg = "x".repeat(1000);
        let result = sanitize_llm_error(&long_msg);
        assert!(result.len() < 600, "should be truncated");
        assert!(result.ends_with("…(truncated)"));
    }
}
