//! LLM integration for the agent.
//!
//! Supports multiple backends:
//! - **OpenAI**: Direct API access with your own key
//! - **Anthropic**: Direct API access with your own key
//! - **Ollama**: Local model inference
//! - **OpenAI-compatible**: Any endpoint that speaks the OpenAI API
// TODO: Gyre native provider

pub mod circuit_breaker;
pub mod claude_oauth;
pub mod cost_tracker;
pub mod costs;
pub mod failover;
#[cfg(any(test, feature = "test-support"))]
pub mod mock;
mod provider;
mod reasoning;
pub mod response_cache;
mod retry;
mod rig_adapter;
pub mod session;
pub mod tracked;

pub use circuit_breaker::{CircuitBreakerConfig, CircuitBreakerProvider};
pub use failover::{CooldownConfig, FailoverProvider};
#[cfg(any(test, feature = "test-support"))]
pub use mock::MockLlmProvider;
pub use provider::{
    ChatMessage, CompletionRequest, CompletionResponse, FinishReason, LlmProvider, ModelMetadata,
    Role, ToolCall, ToolCompletionRequest, ToolCompletionResponse, ToolDefinition, ToolResult,
};
pub use reasoning::{
    ActionPlan, Reasoning, ReasoningContext, RespondOutput, RespondResult, TokenUsage,
    ToolSelection,
};
pub use response_cache::{CachedProvider, ResponseCacheConfig};
pub use rig_adapter::RigAdapter;
pub use session::{SessionConfig, SessionManager, create_session_manager};
pub use tracked::TrackedProvider;

use std::future::Future;
use std::sync::Arc;

use bytes::Bytes;
use rig::client::CompletionClient;
use rig::http_client::{
    self as rig_http, HeaderValue, HttpClientExt, MultipartForm, Request, Response,
};
use secrecy::ExposeSecret;

use crate::config::{LlmBackend, LlmConfig, ResilienceConfig};
use crate::error::LlmError;

/// HTTP client wrapper that replaces `x-api-key` with `Authorization: Bearer`
/// and adds Claude Code-compatible headers for OAuth token authentication.
///
/// The rig Anthropic client hardcodes `x-api-key` as the auth header, but
/// Claude.ai subscription OAuth tokens (sk-ant-oat01-...) require
/// `Authorization: Bearer` plus specific beta/user-agent headers that the
/// Anthropic API checks server-side.
#[derive(Debug, Clone)]
struct OAuthHttpClient {
    inner: reqwest::Client,
    bearer_token: String,
}

impl Default for OAuthHttpClient {
    fn default() -> Self {
        Self {
            inner: reqwest::Client::default(),
            bearer_token: String::new(),
        }
    }
}

impl OAuthHttpClient {
    /// Replace `x-api-key` with `Authorization: Bearer` and inject
    /// Claude Code-compatible headers required for OAuth acceptance.
    fn fix_auth_headers(&self, headers: &mut rig_http::HeaderMap) {
        // Remove the x-api-key that rig's Anthropic client sets by default.
        headers.remove("x-api-key");

        // Set Authorization: Bearer <token>.
        if let Ok(val) = HeaderValue::from_str(&format!("Bearer {}", self.bearer_token)) {
            headers.insert(reqwest::header::AUTHORIZATION, val);
        }

        // Required beta flags: oauth-2025-04-20 enables OAuth token acceptance,
        // claude-code-20250219 identifies the client capability set.
        // Preserve any existing anthropic-beta value (e.g. from rig) and append.
        let existing_beta = headers
            .get("anthropic-beta")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        let oauth_flags = "claude-code-20250219,oauth-2025-04-20";
        let new_beta = if existing_beta.is_empty() {
            oauth_flags.to_string()
        } else {
            format!("{},{}", existing_beta, oauth_flags)
        };
        if let Ok(val) = HeaderValue::from_str(&new_beta) {
            headers.insert("anthropic-beta", val);
        }

        // User-agent and app identifier expected by the server-side OAuth gate.
        headers.insert(
            reqwest::header::USER_AGENT,
            HeaderValue::from_static("claude-cli/2.1.2 (external, cli)"),
        );
        headers.insert("x-app", HeaderValue::from_static("cli"));
    }
}

impl HttpClientExt for OAuthHttpClient {
    fn send<T, U>(
        &self,
        req: Request<T>,
    ) -> impl Future<Output = rig_http::Result<Response<rig_http::LazyBody<U>>>> + Send + 'static
    where
        T: Into<Bytes> + Send,
        U: From<Bytes> + Send + 'static,
    {
        let (mut parts, body) = req.into_parts();
        self.fix_auth_headers(&mut parts.headers);
        let req = Request::from_parts(parts, body);
        self.inner.send(req)
    }

    fn send_multipart<U>(
        &self,
        req: Request<MultipartForm>,
    ) -> impl Future<Output = rig_http::Result<Response<rig_http::LazyBody<U>>>> + Send + 'static
    where
        U: From<Bytes> + Send + 'static,
    {
        let (mut parts, body) = req.into_parts();
        self.fix_auth_headers(&mut parts.headers);
        let req = Request::from_parts(parts, body);
        self.inner.send_multipart(req)
    }

    fn send_streaming<T>(
        &self,
        req: Request<T>,
    ) -> impl Future<Output = rig_http::Result<rig_http::StreamingResponse>> + Send
    where
        T: Into<Bytes>,
    {
        let (mut parts, body) = req.into_parts();
        self.fix_auth_headers(&mut parts.headers);
        let req = Request::from_parts(parts, body);
        self.inner.send_streaming(req)
    }
}

/// Create an LLM provider based on configuration, wrapped with resilience layers
/// and usage tracking.
///
/// Wrapping order: base → CircuitBreaker (if enabled) → Cache (if enabled) → Failover (if enabled) → Tracked.
// TODO: Gyre native provider
pub fn create_llm_provider(
    config: &LlmConfig,
    resilience: &ResilienceConfig,
    _session: Arc<SessionManager>,
) -> Result<Arc<dyn LlmProvider>, LlmError> {
    let base = create_base_provider(config)?;
    let provider = wrap_with_resilience(base, resilience, false)?;

    // Wrap with usage tracking (session ID from process start)
    let tracker = Arc::new(cost_tracker::CostTracker::new());
    let session_id = format!(
        "{}-{}",
        chrono::Utc::now().format("%Y%m%d-%H%M%S"),
        std::process::id()
    );
    let tracked = Arc::new(TrackedProvider::new(provider, tracker, Some(session_id)));

    Ok(tracked)
}

/// Create a bare LLM provider without resilience wrapping.
fn create_base_provider(config: &LlmConfig) -> Result<Arc<dyn LlmProvider>, LlmError> {
    match config.backend {
        LlmBackend::OpenAi => create_openai_provider(config),
        LlmBackend::Anthropic => create_anthropic_provider(config),
        LlmBackend::Ollama => create_ollama_provider(config),
        LlmBackend::OpenAiCompatible => create_openai_compatible_provider(config),
        LlmBackend::Tinfoil => create_tinfoil_provider(config),
    }
}

/// Wrap a provider with resilience layers based on configuration.
///
/// When `force_cache` is true, the response cache is enabled regardless of config
/// (used for the cheap LLM provider where caching is always beneficial).
fn wrap_with_resilience(
    mut provider: Arc<dyn LlmProvider>,
    resilience: &ResilienceConfig,
    force_cache: bool,
) -> Result<Arc<dyn LlmProvider>, LlmError> {
    // 1. Circuit breaker (innermost wrapper — closest to the base provider)
    if resilience.circuit_breaker_enabled {
        provider = Arc::new(CircuitBreakerProvider::new(
            provider,
            CircuitBreakerConfig {
                failure_threshold: resilience.circuit_breaker_failure_threshold,
                recovery_timeout: std::time::Duration::from_secs(
                    resilience.circuit_breaker_reset_timeout_secs,
                ),
                half_open_successes_needed: 2,
            },
        ));
        tracing::debug!(
            "Circuit breaker enabled (threshold: {}, reset: {}s)",
            resilience.circuit_breaker_failure_threshold,
            resilience.circuit_breaker_reset_timeout_secs
        );
    }

    // 2. Response cache
    if resilience.response_cache_enabled || force_cache {
        provider = Arc::new(CachedProvider::new(
            provider,
            ResponseCacheConfig {
                ttl: std::time::Duration::from_secs(resilience.response_cache_ttl_secs),
                max_entries: resilience.response_cache_max_entries,
            },
        ));
        tracing::debug!(
            "Response cache enabled (ttl: {}s, max: {})",
            resilience.response_cache_ttl_secs,
            resilience.response_cache_max_entries
        );
    }

    // 3. Failover (outermost wrapper)
    if resilience.failover_enabled {
        if let Some(ref fallback_backend) = resilience.failover_fallback_backend {
            match create_fallback_provider(resilience, fallback_backend) {
                Ok(fallback) => {
                    provider = Arc::new(FailoverProvider::with_cooldown(
                        vec![provider, fallback],
                        CooldownConfig {
                            cooldown_duration: std::time::Duration::from_secs(
                                resilience.failover_cooldown_secs,
                            ),
                            failure_threshold: 3,
                        },
                    )?);
                    tracing::debug!("Failover enabled (fallback: {})", fallback_backend);
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to create fallback provider '{}': {}. Failover disabled.",
                        fallback_backend,
                        e
                    );
                }
            }
        }
    }

    Ok(provider)
}

/// Create a fallback provider from resilience failover config.
fn create_fallback_provider(
    resilience: &ResilienceConfig,
    backend_str: &str,
) -> Result<Arc<dyn LlmProvider>, LlmError> {
    use secrecy::ExposeSecret;

    let backend: LlmBackend = backend_str.parse().map_err(|e| LlmError::RequestFailed {
        provider: "failover".to_string(),
        reason: format!("Invalid fallback backend '{}': {}", backend_str, e),
    })?;

    // Build a minimal LlmConfig for the fallback
    let model = resilience
        .failover_fallback_model
        .clone()
        .unwrap_or_else(|| match backend {
            LlmBackend::Anthropic => "claude-sonnet-4-5-20250929".to_string(),
            LlmBackend::OpenAi => "gpt-4o".to_string(),
            LlmBackend::Ollama => "llama3".to_string(),
            _ => "default".to_string(),
        });

    let base_url = resilience
        .failover_fallback_base_url
        .clone()
        .unwrap_or_else(|| match backend {
            LlmBackend::Ollama => "http://localhost:11434".to_string(),
            _ => String::new(),
        });

    let api_key = resilience
        .failover_fallback_api_key
        .as_ref()
        .map(|k| k.expose_secret().to_string())
        .unwrap_or_default();

    // Build a temporary LlmConfig for the fallback backend
    let mut fallback_config = LlmConfig {
        backend,
        openai: None,
        anthropic: None,
        ollama: None,
        openai_compatible: None,
        tinfoil: None,
    };

    match backend {
        LlmBackend::Ollama => {
            fallback_config.ollama = Some(crate::config::OllamaConfig { base_url, model });
        }
        LlmBackend::OpenAi => {
            fallback_config.openai = Some(crate::config::OpenAiDirectConfig {
                api_key: secrecy::SecretString::from(api_key),
                model,
            });
        }
        LlmBackend::Anthropic => {
            fallback_config.anthropic = Some(crate::config::AnthropicDirectConfig {
                api_key: secrecy::SecretString::from(api_key),
                model,
            });
        }
        LlmBackend::OpenAiCompatible => {
            fallback_config.openai_compatible = Some(crate::config::OpenAiCompatibleConfig {
                base_url,
                model,
                api_key: if api_key.is_empty() {
                    None
                } else {
                    Some(secrecy::SecretString::from(api_key))
                },
            });
        }
        LlmBackend::Tinfoil => {
            fallback_config.tinfoil = Some(crate::config::TinfoilConfig {
                api_key: secrecy::SecretString::from(api_key),
                model,
            });
        }
    }

    create_base_provider(&fallback_config)
}

fn create_openai_provider(config: &LlmConfig) -> Result<Arc<dyn LlmProvider>, LlmError> {
    let oai = config.openai.as_ref().ok_or_else(|| LlmError::AuthFailed {
        provider: "openai".to_string(),
    })?;

    use rig::providers::openai;

    let client: openai::Client =
        openai::Client::new(oai.api_key.expose_secret()).map_err(|e| LlmError::RequestFailed {
            provider: "openai".to_string(),
            reason: format!("Failed to create OpenAI client: {}", e),
        })?;

    let model = client.completion_model(&oai.model);
    tracing::info!("Using OpenAI direct API (model: {})", oai.model);
    Ok(Arc::new(RigAdapter::new(model, &oai.model)))
}

fn create_anthropic_provider(config: &LlmConfig) -> Result<Arc<dyn LlmProvider>, LlmError> {
    let anth = config
        .anthropic
        .as_ref()
        .ok_or_else(|| LlmError::AuthFailed {
            provider: "anthropic".to_string(),
        })?;

    use rig::providers::anthropic;

    let api_key_str = anth.api_key.expose_secret();
    let is_oauth = api_key_str.starts_with("sk-ant-oat");

    if is_oauth {
        // Claude.ai subscription OAuth: use Authorization: Bearer instead of x-api-key.
        let oauth_client = OAuthHttpClient {
            inner: reqwest::Client::new(),
            bearer_token: api_key_str.to_string(),
        };

        let client = anthropic::Client::<OAuthHttpClient>::builder()
            .api_key(api_key_str)
            .http_client(oauth_client)
            .build()
            .map_err(|e| LlmError::RequestFailed {
                provider: "anthropic".to_string(),
                reason: format!("Failed to create Anthropic OAuth client: {}", e),
            })?;

        let model = client.completion_model(&anth.model);
        tracing::info!(
            "Using Anthropic API via Claude.ai subscription (model: {})",
            anth.model
        );
        Ok(Arc::new(
            RigAdapter::new(model, &anth.model)
                .with_preamble_prefix("You are Claude Code, Anthropic's official CLI for Claude."),
        ))
    } else {
        // Standard API key: use x-api-key header.
        let client: anthropic::Client =
            anthropic::Client::new(api_key_str).map_err(|e| LlmError::RequestFailed {
                provider: "anthropic".to_string(),
                reason: format!("Failed to create Anthropic client: {}", e),
            })?;

        let model = client.completion_model(&anth.model);
        tracing::info!("Using Anthropic direct API (model: {})", anth.model);
        Ok(Arc::new(RigAdapter::new(model, &anth.model)))
    }
}

fn create_ollama_provider(config: &LlmConfig) -> Result<Arc<dyn LlmProvider>, LlmError> {
    let oll = config.ollama.as_ref().ok_or_else(|| LlmError::AuthFailed {
        provider: "ollama".to_string(),
    })?;

    use rig::client::Nothing;
    use rig::providers::ollama;

    let client: ollama::Client = ollama::Client::builder()
        .base_url(&oll.base_url)
        .api_key(Nothing)
        .build()
        .map_err(|e| LlmError::RequestFailed {
            provider: "ollama".to_string(),
            reason: format!("Failed to create Ollama client: {}", e),
        })?;

    let model = client.completion_model(&oll.model);
    tracing::info!(
        "Using Ollama (base_url: {}, model: {})",
        oll.base_url,
        oll.model
    );
    Ok(Arc::new(RigAdapter::new(model, &oll.model)))
}

const TINFOIL_BASE_URL: &str = "https://inference.tinfoil.sh/v1";

fn create_tinfoil_provider(config: &LlmConfig) -> Result<Arc<dyn LlmProvider>, LlmError> {
    let tf = config
        .tinfoil
        .as_ref()
        .ok_or_else(|| LlmError::AuthFailed {
            provider: "tinfoil".to_string(),
        })?;

    use rig::providers::openai;

    let client: openai::Client = openai::Client::builder()
        .base_url(TINFOIL_BASE_URL)
        .api_key(tf.api_key.expose_secret())
        .build()
        .map_err(|e| LlmError::RequestFailed {
            provider: "tinfoil".to_string(),
            reason: format!("Failed to create Tinfoil client: {}", e),
        })?;

    // Tinfoil currently only supports the Chat Completions API and not the newer Responses API,
    // so we must explicitly select the completions API here (unlike other OpenAI-compatible providers).
    let client = client.completions_api();
    let model = client.completion_model(&tf.model);
    tracing::info!("Using Tinfoil private inference (model: {})", tf.model);
    Ok(Arc::new(RigAdapter::new(model, &tf.model)))
}

fn create_openai_compatible_provider(config: &LlmConfig) -> Result<Arc<dyn LlmProvider>, LlmError> {
    let compat = config
        .openai_compatible
        .as_ref()
        .ok_or_else(|| LlmError::AuthFailed {
            provider: "openai_compatible".to_string(),
        })?;

    use rig::providers::openai;

    let api_key = compat
        .api_key
        .as_ref()
        .map(|k| k.expose_secret().to_string())
        .unwrap_or_else(|| "no-key".to_string());

    let client: openai::Client = openai::Client::builder()
        .base_url(&compat.base_url)
        .api_key(api_key)
        .build()
        .map_err(|e| LlmError::RequestFailed {
            provider: "openai_compatible".to_string(),
            reason: format!("Failed to create OpenAI-compatible client: {}", e),
        })?;

    // OpenAI-compatible providers (e.g. OpenRouter) are most reliable on Chat Completions.
    // This avoids Responses-API-specific assumptions such as required tool call IDs.
    let model = client.completions_api().completion_model(&compat.model);
    tracing::info!(
        "Using OpenAI-compatible endpoint via Chat Completions API (base_url: {}, model: {})",
        compat.base_url,
        compat.model
    );
    Ok(Arc::new(RigAdapter::new(model, &compat.model)))
}

/// Create a cheap/fast LLM provider for lightweight tasks (heartbeat, routing, evaluation).
///
/// Currently returns None (stub). Will be implemented with Gyre native provider.
/// When a provider is returned, response caching is enabled by default.
// TODO: Gyre native provider
pub fn create_cheap_llm_provider(
    _config: &LlmConfig,
    _resilience: &ResilienceConfig,
    _session: Arc<SessionManager>,
) -> Result<Option<Arc<dyn LlmProvider>>, LlmError> {
    // Stub: no cheap provider yet. When implemented, wrap with:
    //   wrap_with_resilience(base, resilience, true /* force_cache */)
    Ok(None)
}
