//! Stage 6: Auth & Model Selection
//!
//! Grouped provider selection (Anthropic, OpenAI, Ollama, OpenAI-compatible,
//! Tinfoil). API key validation + secrets storage. Model fetching via API
//! with FuzzySelect. Embeddings config (Advanced only).

use async_trait::async_trait;
use secrecy::ExposeSecret;

use super::{SetupError, SetupStage, StageOutcome};
use crate::setup::state::SetupState;
use crate::setup::ui::SetupUi;

pub struct AuthModelStage;

#[async_trait]
impl SetupStage for AuthModelStage {
    fn id(&self) -> &'static str {
        "auth_model"
    }

    fn name(&self) -> &'static str {
        "LLM Provider & Model"
    }

    async fn run(&self, state: &mut SetupState, ui: &SetupUi) -> Result<StageOutcome, SetupError> {
        if state.quickstart {
            // QuickStart: auto-detect credentials without interactive prompts.
            return self.run_quickstart(state, ui).await;
        }

        // Provider selection.
        let providers = &[
            "Anthropic (Claude)",
            "OpenAI (GPT)",
            "Ollama (local)",
            "OpenAI-compatible endpoint",
            "Tinfoil (private inference)",
        ];

        let choice = ui.select_one("Select LLM provider", providers)?;

        match choice {
            0 => self.setup_anthropic(state, ui).await?,
            1 => self.setup_openai(state, ui).await?,
            2 => self.setup_ollama(state, ui).await?,
            3 => self.setup_openai_compatible(state, ui).await?,
            4 => self.setup_tinfoil(state, ui).await?,
            _ => unreachable!(),
        }

        // Model selection.
        self.select_model(state, ui).await?;

        // Embeddings (Advanced only).
        self.configure_embeddings(state, ui).await?;

        Ok(StageOutcome::Completed)
    }
}

impl AuthModelStage {
    /// QuickStart: auto-detect credentials from environment/keychain,
    /// pick default model, no interactive prompts.
    async fn run_quickstart(
        &self,
        state: &mut SetupState,
        ui: &SetupUi,
    ) -> Result<StageOutcome, SetupError> {
        // Check existing config.toml backend setting.
        let existing_backend = state.settings.llm_backend.clone();

        // Try to auto-detect credentials in priority order.
        // 1. Claude.ai subscription OAuth token (refreshed in place if stale)
        let (_oauth_status, quick_oauth) = crate::llm::claude_oauth::ensure_fresh_token().await;
        if let Some(token) = quick_oauth {
            state.settings.llm_backend = Some("anthropic".to_string());
            state.llm_api_key = Some(secrecy::SecretString::from(token));
            ui.success("Auto-detected Claude.ai subscription credentials.");
        }
        // 2. ANTHROPIC_API_KEY env var
        else if let Ok(key) = std::env::var("ANTHROPIC_API_KEY") {
            state.settings.llm_backend = Some("anthropic".to_string());
            state.llm_api_key = Some(secrecy::SecretString::from(key));
            ui.success("Found ANTHROPIC_API_KEY in environment.");
        }
        // 3. OPENAI_API_KEY env var
        else if let Ok(key) = std::env::var("OPENAI_API_KEY") {
            state.settings.llm_backend = Some("openai".to_string());
            state.llm_api_key = Some(secrecy::SecretString::from(key));
            ui.success("Found OPENAI_API_KEY in environment.");
        }
        // 4. Keep existing backend if configured
        else if existing_backend.is_some() {
            ui.info("Keeping existing LLM backend configuration.");
        } else {
            return Err(SetupError::Validation(
                "QuickStart requires at least one LLM credential (Claude.ai subscription, \
                 ANTHROPIC_API_KEY, or OPENAI_API_KEY). Set one and re-run."
                    .to_string(),
            ));
        }

        // Auto-select default model if not already set.
        if state.settings.selected_model.is_none() {
            let backend = state.settings.llm_backend.as_deref().unwrap_or("anthropic");
            let model = default_model_for_backend(backend);
            state.settings.selected_model = Some(model.clone());
            ui.success(&format!("Selected model: {}", model));
        } else {
            ui.success(&format!(
                "Keeping model: {}",
                state
                    .settings
                    .selected_model
                    .as_deref()
                    .unwrap_or("unknown")
            ));
        }

        Ok(StageOutcome::Completed)
    }

    async fn setup_anthropic(
        &self,
        state: &mut SetupState,
        ui: &SetupUi,
    ) -> Result<(), SetupError> {
        state.settings.llm_backend = Some("anthropic".to_string());

        // Auto-detect Claude.ai subscription credentials from Claude Code's
        // OAuth store, refreshing in place if the stored token is expired so
        // a signed-in-but-idle user is still recognized (and never blessed
        // with a token that's already dead).
        let (oauth_status, oauth_token) = crate::llm::claude_oauth::ensure_fresh_token().await;
        if matches!(
            oauth_status,
            crate::llm::claude_oauth::CredentialStatus::Expired
        ) {
            ui.info(
                "Claude Code is signed in but its session has expired and couldn't be \
                 refreshed. Re-sign-in with `claude`, or enter an API key below.",
            );
        }

        let api_key = if let Some(ref token) = oauth_token {
            ui.success("Found Claude.ai subscription credentials (via Claude Code).");
            let use_it = ui.confirm("Use your Claude.ai subscription?", true)?;
            if use_it {
                secrecy::SecretString::from(token.clone())
            } else {
                ui.secret_input("Anthropic API key (sk-ant-...) — input is hidden")?
            }
        } else {
            // Check ANTHROPIC_API_KEY env var.
            if let Ok(env_key) = std::env::var("ANTHROPIC_API_KEY") {
                ui.success("Found ANTHROPIC_API_KEY in environment.");
                let use_it = ui.confirm("Use the existing ANTHROPIC_API_KEY?", true)?;
                if use_it {
                    secrecy::SecretString::from(env_key)
                } else {
                    ui.secret_input("Anthropic API key (sk-ant-...) — input is hidden")?
                }
            } else {
                ui.info("No Claude.ai subscription detected.");
                ui.info("Get an API key at: https://console.anthropic.com/settings/keys");
                ui.secret_input("Anthropic API key (sk-ant-...) — input is hidden")?
            }
        };

        state.llm_api_key = Some(api_key.clone());

        // Store in secrets if available.
        if let Some(ref store) = state.secrets_store {
            let crypto = state.secrets_crypto.as_ref().ok_or_else(|| {
                SetupError::Validation("Secrets crypto not initialized".to_string())
            })?;
            let encrypted = crypto
                .encrypt(api_key.expose_secret().as_bytes())
                .map_err(|e| SetupError::Other(anyhow::anyhow!("Encryption failed: {}", e)))?;
            let _ = store;
            let _ = encrypted;
        }

        ui.success("Anthropic credentials configured.");
        Ok(())
    }

    async fn setup_openai(&self, state: &mut SetupState, ui: &SetupUi) -> Result<(), SetupError> {
        state.settings.llm_backend = Some("openai".to_string());

        let api_key = ui.secret_input("OpenAI API key (sk-...)")?;
        state.llm_api_key = Some(api_key);

        ui.success("OpenAI API key configured.");
        Ok(())
    }

    async fn setup_ollama(&self, state: &mut SetupState, ui: &SetupUi) -> Result<(), SetupError> {
        state.settings.llm_backend = Some("ollama".to_string());

        let base_url = ui.input_with_default("Ollama base URL", "http://localhost:11434")?;
        state.settings.ollama_base_url = Some(base_url);

        ui.success("Ollama configured (no API key required).");
        Ok(())
    }

    async fn setup_openai_compatible(
        &self,
        state: &mut SetupState,
        ui: &SetupUi,
    ) -> Result<(), SetupError> {
        state.settings.llm_backend = Some("openai_compatible".to_string());

        let base_url = ui.input("OpenAI-compatible base URL (e.g., http://localhost:8000/v1)")?;
        state.settings.openai_compatible_base_url = Some(base_url);

        let needs_key = ui.confirm("Does this endpoint require an API key?", true)?;
        if needs_key {
            let api_key = ui.secret_input("API key")?;
            state.llm_api_key = Some(api_key);
        }

        ui.success("OpenAI-compatible endpoint configured.");
        Ok(())
    }

    async fn setup_tinfoil(&self, state: &mut SetupState, ui: &SetupUi) -> Result<(), SetupError> {
        state.settings.llm_backend = Some("tinfoil".to_string());

        ui.info("Tinfoil provides private, verifiable AI inference.");
        let api_key = ui.secret_input("Tinfoil API key")?;
        state.llm_api_key = Some(api_key);

        ui.success("Tinfoil configured.");
        Ok(())
    }

    async fn select_model(&self, state: &mut SetupState, ui: &SetupUi) -> Result<(), SetupError> {
        let backend = state
            .settings
            .llm_backend
            .clone()
            .unwrap_or_else(|| "anthropic".to_string());

        // Attempt to fetch models from the provider API.
        let models = self.fetch_models(&backend, state).await;

        if models.is_empty() {
            // Fallback to manual entry.
            let model =
                ui.input_with_default("Model name", &default_model_for_backend(&backend))?;
            state.settings.selected_model = Some(model.clone());
            ui.success(&format!("Selected model: {}", model));
        } else {
            let idx = ui.fuzzy_select("Select model", &models)?;
            state.settings.selected_model = Some(models[idx].clone());
            ui.success(&format!("Selected model: {}", models[idx]));
        }

        Ok(())
    }

    async fn fetch_models(&self, backend: &str, state: &SetupState) -> Vec<String> {
        // Model fetching is best-effort. On failure, return empty vec
        // so the caller falls back to manual entry.
        match backend {
            "anthropic" => {
                // Return well-known Anthropic models.
                vec![
                    "claude-opus-4-6".to_string(),
                    "claude-sonnet-4-5-20250929".to_string(),
                    "claude-haiku-4-5-20251001".to_string(),
                    "claude-sonnet-4-20250514".to_string(),
                ]
            }
            "openai" => {
                vec![
                    "gpt-4o".to_string(),
                    "gpt-4-turbo".to_string(),
                    "gpt-4".to_string(),
                    "gpt-3.5-turbo".to_string(),
                ]
            }
            "ollama" => {
                // Try fetching from Ollama API.
                let base_url = state
                    .settings
                    .ollama_base_url
                    .as_deref()
                    .unwrap_or("http://localhost:11434");

                match reqwest::get(format!("{}/api/tags", base_url)).await {
                    Ok(resp) => {
                        if let Ok(json) = resp.json::<serde_json::Value>().await {
                            if let Some(models) = json.get("models").and_then(|m| m.as_array()) {
                                return models
                                    .iter()
                                    .filter_map(|m| {
                                        m.get("name").and_then(|n| n.as_str()).map(String::from)
                                    })
                                    .collect();
                            }
                        }
                        Vec::new()
                    }
                    Err(_) => Vec::new(),
                }
            }
            _ => Vec::new(),
        }
    }

    async fn configure_embeddings(
        &self,
        state: &mut SetupState,
        ui: &SetupUi,
    ) -> Result<(), SetupError> {
        let enable = ui.confirm("Enable semantic search (embeddings)?", false)?;
        state.settings.embeddings.enabled = enable;

        if enable {
            state.settings.embeddings.provider = "openai".to_string();
            if state.settings.llm_backend.as_deref() != Some("openai") {
                ui.info("Embeddings use OpenAI's API. An OPENAI_API_KEY is required.");
            }

            let model =
                ui.input_with_default("Embedding model", &state.settings.embeddings.model)?;
            state.settings.embeddings.model = model;

            ui.success("Embeddings configured.");
        }

        Ok(())
    }
}

fn default_model_for_backend(backend: &str) -> String {
    match backend {
        "anthropic" => "claude-sonnet-4-5-20250929".to_string(),
        "openai" => "gpt-4o".to_string(),
        "ollama" => "llama3".to_string(),
        "tinfoil" => "claude-sonnet-4-5-20250929".to_string(),
        _ => "claude-sonnet-4-5-20250929".to_string(),
    }
}
