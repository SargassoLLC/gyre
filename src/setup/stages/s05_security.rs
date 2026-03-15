//! Stage 5: Security
//!
//! Configure the secrets master key source: OS keychain, env var, or skip.
//! Cache the `SecretsCrypto` in state for later stages.

use std::sync::Arc;

use async_trait::async_trait;
use secrecy::SecretString;

use super::{SetupError, SetupStage, StageOutcome};
use crate::secrets::SecretsCrypto;
use crate::settings::KeySource;
use crate::setup::state::SetupState;
use crate::setup::ui::SetupUi;

pub struct SecurityStage;

#[async_trait]
impl SetupStage for SecurityStage {
    fn id(&self) -> &'static str {
        "security"
    }

    fn name(&self) -> &'static str {
        "Security"
    }

    async fn run(&self, state: &mut SetupState, ui: &SetupUi) -> Result<StageOutcome, SetupError> {
        // Check if key already in env.
        if std::env::var("SECRETS_MASTER_KEY").is_ok() {
            ui.info("Secrets master key found in SECRETS_MASTER_KEY environment variable.");
            state.settings.secrets_master_key_source = KeySource::Env;
            self.init_crypto_from_env(state)?;
            ui.success("Security configured (env var).");
            return Ok(StageOutcome::Completed);
        }

        // Check existing keychain key.
        if let Ok(keychain_bytes) = crate::secrets::keychain::get_master_key().await {
            let key_hex: String = keychain_bytes
                .iter()
                .map(|b| format!("{:02x}", b))
                .collect();
            let crypto = SecretsCrypto::new(SecretString::from(key_hex))
                .map_err(|e| SetupError::Other(anyhow::anyhow!("Crypto init failed: {}", e)))?;
            state.secrets_crypto = Some(Arc::new(crypto));

            ui.info("Existing master key found in OS keychain.");
            if state.quickstart || ui.confirm("Use existing keychain key?", true)? {
                state.settings.secrets_master_key_source = KeySource::Keychain;
                state.init_secrets_store();
                ui.success("Security configured (keychain).");
                return Ok(StageOutcome::Completed);
            }
            // User declined — clear cached crypto.
            state.secrets_crypto = None;
        }

        if state.quickstart {
            // QuickStart: auto-generate keychain key.
            return self.setup_keychain(state, ui).await;
        }

        let choice = ui.select_one(
            "How should Gyre store the secrets master key?",
            &[
                "OS Keychain (recommended — macOS Keychain / Linux Secret Service)",
                "Environment variable (SECRETS_MASTER_KEY)",
                "Skip (secrets features disabled)",
            ],
        )?;

        match choice {
            0 => self.setup_keychain(state, ui).await,
            1 => self.setup_env_key(state, ui),
            2 => {
                state.settings.secrets_master_key_source = KeySource::None;
                ui.info("Secrets features disabled. Channel tokens must be set via env vars.");
                Ok(StageOutcome::Completed)
            }
            _ => unreachable!(),
        }
    }
}

impl SecurityStage {
    async fn setup_keychain(
        &self,
        state: &mut SetupState,
        ui: &SetupUi,
    ) -> Result<StageOutcome, SetupError> {
        ui.info("Generating master key...");

        let key = crate::secrets::keychain::generate_master_key();

        crate::secrets::keychain::store_master_key(&key)
            .await
            .map_err(|e| {
                SetupError::Other(anyhow::anyhow!("Failed to store in keychain: {}", e))
            })?;

        let key_hex: String = key.iter().map(|b| format!("{:02x}", b)).collect();
        let crypto = SecretsCrypto::new(SecretString::from(key_hex))
            .map_err(|e| SetupError::Other(anyhow::anyhow!("Crypto init failed: {}", e)))?;

        state.secrets_crypto = Some(Arc::new(crypto));
        state.settings.secrets_master_key_source = KeySource::Keychain;
        state.init_secrets_store();

        ui.success("Master key generated and stored in OS keychain.");
        Ok(StageOutcome::Completed)
    }

    fn setup_env_key(
        &self,
        state: &mut SetupState,
        ui: &SetupUi,
    ) -> Result<StageOutcome, SetupError> {
        let key_hex = crate::secrets::keychain::generate_master_key_hex();
        ui.blank();
        ui.info("Generated key. Add this to your shell profile or .env file:");
        ui.info(&format!("  export SECRETS_MASTER_KEY={}", key_hex));
        ui.blank();

        state.settings.secrets_master_key_source = KeySource::Env;
        ui.success("Configured for environment variable.");
        Ok(StageOutcome::Completed)
    }

    fn init_crypto_from_env(&self, state: &mut SetupState) -> Result<(), SetupError> {
        if let Ok(key) = std::env::var("SECRETS_MASTER_KEY") {
            let crypto = SecretsCrypto::new(SecretString::from(key))
                .map_err(|e| SetupError::Other(anyhow::anyhow!("Crypto init failed: {}", e)))?;
            state.secrets_crypto = Some(Arc::new(crypto));
            state.init_secrets_store();
        }
        Ok(())
    }
}
