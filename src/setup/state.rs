//! Setup wizard state management.
//!
//! Tracks the wizard's progress through stages, holds accumulated
//! configuration, and enables resume/back navigation.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use secrecy::SecretString;

use crate::secrets::{SecretsCrypto, SecretsStore};
use crate::settings::Settings;

/// Overall setup state carried across all stages.
pub struct SetupState {
    /// Accumulated settings (modified by each stage).
    pub settings: Settings,

    /// Status of each stage (keyed by stage ID).
    pub stage_status: HashMap<String, StageStatus>,

    /// Detected configuration state from Stage 2.
    pub detected: DetectedConfig,

    /// Whether the user chose QuickStart mode.
    pub quickstart: bool,

    /// Database pool (created during database stage, postgres only).
    #[cfg(feature = "postgres")]
    pub db_pool: Option<deadpool_postgres::Pool>,

    /// libSQL backend (created during database stage).
    #[cfg(feature = "libsql")]
    pub db_backend: Option<crate::db::libsql_backend::LibSqlBackend>,

    /// Secrets crypto instance (created during security stage).
    pub secrets_crypto: Option<Arc<SecretsCrypto>>,

    /// Secrets store (created after DB + crypto are ready).
    pub secrets_store: Option<Arc<dyn SecretsStore + Send + Sync>>,

    /// Cached LLM API key (from provider setup, used by model fetcher).
    pub llm_api_key: Option<SecretString>,

    /// Agents directory for hermit box creation.
    pub agents_dir: PathBuf,

    /// Wizard metadata (timing, version).
    pub metadata: SetupMetadata,
}

/// Status of an individual setup stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StageStatus {
    /// Not yet attempted.
    Pending,
    /// Completed successfully.
    Completed,
    /// Skipped (QuickStart or already satisfied).
    Skipped,
}

/// Results of configuration state detection (Stage 2).
#[derive(Debug, Clone, Default)]
pub struct DetectedConfig {
    /// Whether ~/.gyre/.env exists.
    pub has_env_file: bool,
    /// Whether settings.json exists.
    pub has_settings_json: bool,
    /// Whether config.toml exists.
    pub has_config_toml: bool,
    /// Whether a database is reachable.
    pub has_database: bool,
    /// Whether any LLM API key is configured.
    pub has_llm_key: bool,
    /// Whether any agent boxes were found.
    pub agent_box_names: Vec<String>,
    /// The detected installation type.
    pub install_type: InstallType,
}

/// Type of installation detected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InstallType {
    /// No existing configuration found.
    #[default]
    Fresh,
    /// Legacy configuration (settings.json only, no DB).
    Legacy,
    /// Existing installation with DB settings.
    Existing,
}

/// Metadata about the setup wizard run.
#[derive(Debug, Clone)]
pub struct SetupMetadata {
    /// When the wizard started.
    pub started_at: chrono::DateTime<chrono::Utc>,
    /// Gyre version.
    pub version: String,
    /// Stages that were completed.
    pub completed_stages: Vec<String>,
}

impl SetupState {
    /// Create a new state with defaults.
    pub fn new() -> Self {
        let default_agents_dir = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("agents");

        Self {
            settings: Settings::default(),
            stage_status: HashMap::new(),
            detected: DetectedConfig::default(),
            quickstart: false,
            #[cfg(feature = "postgres")]
            db_pool: None,
            #[cfg(feature = "libsql")]
            db_backend: None,
            secrets_crypto: None,
            secrets_store: None,
            llm_api_key: None,
            agents_dir: default_agents_dir,
            metadata: SetupMetadata {
                started_at: chrono::Utc::now(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                completed_stages: Vec::new(),
            },
        }
    }

    /// Mark a stage as completed.
    pub fn mark_completed(&mut self, stage_id: &str) {
        self.stage_status
            .insert(stage_id.to_string(), StageStatus::Completed);
        self.metadata.completed_stages.push(stage_id.to_string());
    }

    /// Mark a stage as skipped.
    pub fn mark_skipped(&mut self, stage_id: &str) {
        self.stage_status
            .insert(stage_id.to_string(), StageStatus::Skipped);
    }

    /// Check if a stage has been completed.
    pub fn is_completed(&self, stage_id: &str) -> bool {
        self.stage_status
            .get(stage_id)
            .is_some_and(|s| *s == StageStatus::Completed)
    }

    /// Initialize a secrets store from the current DB + crypto state.
    ///
    /// Called after both the database and security stages complete.
    pub fn init_secrets_store(&mut self) {
        let Some(ref crypto) = self.secrets_crypto else {
            return;
        };

        #[cfg(feature = "libsql")]
        if let Some(ref backend) = self.db_backend {
            self.secrets_store = Some(Arc::new(crate::secrets::LibSqlSecretsStore::new(
                backend.shared_db(),
                Arc::clone(crypto),
            )));
            return;
        }

        #[cfg(feature = "postgres")]
        if let Some(ref pool) = self.db_pool {
            self.secrets_store = Some(Arc::new(crate::secrets::PostgresSecretsStore::new(
                pool.clone(),
                Arc::clone(crypto),
            )));
        }
    }
}

impl Default for SetupState {
    fn default() -> Self {
        Self::new()
    }
}
