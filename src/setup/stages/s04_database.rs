//! Stage 4: Database Setup
//!
//! Choose between PostgreSQL and libSQL, configure connection,
//! run migrations, and write bootstrap vars to ~/.gyre/.env.

use async_trait::async_trait;

use super::{SetupError, SetupStage, StageOutcome};
use crate::setup::state::SetupState;
use crate::setup::ui::SetupUi;

pub struct DatabaseStage;

#[async_trait]
impl SetupStage for DatabaseStage {
    fn id(&self) -> &'static str {
        "database"
    }

    fn name(&self) -> &'static str {
        "Database Setup"
    }

    async fn run(&self, state: &mut SetupState, ui: &SetupUi) -> Result<StageOutcome, SetupError> {
        if state.quickstart {
            // QuickStart auto-picks libSQL at default path.
            ui.info("QuickStart: using embedded libSQL database.");

            let default_path = dirs::home_dir()
                .unwrap_or_else(|| std::path::PathBuf::from("."))
                .join(".gyre")
                .join("gyre.db");

            state.settings.database_backend = Some("libsql".to_string());
            state.settings.libsql_path = Some(default_path.to_string_lossy().to_string());

            #[cfg(feature = "libsql")]
            {
                self.init_libsql(state, &default_path).await?;
            }

            ui.success(&format!("Database: libSQL at {}", default_path.display()));
            return Ok(StageOutcome::Completed);
        }

        // Advanced mode: choose backend.
        let backends = self.available_backends();
        let choice = ui.select_one("Select database backend", &backends)?;
        let backend = backends[choice];

        match backend {
            "libSQL (embedded, zero-config)" => {
                self.setup_libsql(state, ui).await?;
            }
            "PostgreSQL (production)" => {
                self.setup_postgres(state, ui).await?;
            }
            _ => unreachable!(),
        }

        Ok(StageOutcome::Completed)
    }
}

impl DatabaseStage {
    fn available_backends(&self) -> Vec<&'static str> {
        let mut backends = Vec::new();

        #[cfg(feature = "libsql")]
        backends.push("libSQL (embedded, zero-config)");

        #[cfg(feature = "postgres")]
        backends.push("PostgreSQL (production)");

        if backends.is_empty() {
            backends.push("libSQL (embedded, zero-config)");
        }

        backends
    }

    #[cfg(feature = "libsql")]
    async fn init_libsql(
        &self,
        state: &mut SetupState,
        path: &std::path::Path,
    ) -> Result<(), SetupError> {
        use crate::db::Database;
        use crate::db::libsql_backend::LibSqlBackend;

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| SetupError::Database(format!("Failed to create directory: {}", e)))?;
        }

        let backend = LibSqlBackend::new_local(path)
            .await
            .map_err(|e| SetupError::Database(format!("Failed to open libSQL: {}", e)))?;

        backend
            .run_migrations()
            .await
            .map_err(|e| SetupError::Database(format!("Migration failed: {}", e)))?;

        state.db_backend = Some(backend);
        Ok(())
    }

    async fn setup_libsql(&self, state: &mut SetupState, ui: &SetupUi) -> Result<(), SetupError> {
        let default_path = dirs::home_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join(".gyre")
            .join("gyre.db");

        let path_str =
            ui.input_with_default("libSQL database path", &default_path.to_string_lossy())?;

        state.settings.database_backend = Some("libsql".to_string());
        state.settings.libsql_path = Some(path_str.clone());

        #[cfg(feature = "libsql")]
        {
            let path = std::path::PathBuf::from(&path_str);
            self.init_libsql(state, &path).await?;
        }

        // Optional Turso sync.
        let use_turso = ui.confirm("Enable Turso cloud sync?", false)?;
        if use_turso {
            let turso_url = ui.input("Turso database URL (libsql://...)")?;
            state.settings.libsql_url = Some(turso_url);
        }

        ui.success(&format!("Database: libSQL at {}", path_str));
        Ok(())
    }

    async fn setup_postgres(&self, state: &mut SetupState, ui: &SetupUi) -> Result<(), SetupError> {
        let default_url = "postgres://gyre:gyre@localhost:5432/gyre";

        loop {
            let url = ui.input_with_default("PostgreSQL connection URL", default_url)?;

            state.settings.database_backend = Some("postgres".to_string());
            state.settings.database_url = Some(url.clone());

            let pool_size = ui.input_with_default("Connection pool size", "10")?;
            if let Ok(size) = pool_size.parse() {
                state.settings.database_pool_size = Some(size);
            }

            ui.info("Testing connection...");

            #[cfg(feature = "postgres")]
            {
                match self.test_postgres_connection(&url).await {
                    Ok(pool) => {
                        ui.success("PostgreSQL connection successful.");
                        state.db_pool = Some(pool);
                    }
                    Err(e) => {
                        ui.error(&format!("Connection failed: {}", e));
                        let retry = ui.confirm("Retry with different URL?", true)?;
                        if retry {
                            continue;
                        }
                        return Err(SetupError::Database(e));
                    }
                }
            }

            #[cfg(not(feature = "postgres"))]
            {
                ui.info("PostgreSQL support not compiled in. Skipping connection test.");
            }

            ui.success(&format!("Database: PostgreSQL at {}", url));
            return Ok(());
        }
    }

    #[cfg(feature = "postgres")]
    async fn test_postgres_connection(&self, url: &str) -> Result<deadpool_postgres::Pool, String> {
        use deadpool_postgres::{Config, Runtime};

        let mut config = Config::new();
        config.url = Some(url.to_string());

        let pool = config
            .create_pool(Some(Runtime::Tokio1), tokio_postgres::NoTls)
            .map_err(|e| format!("Pool creation failed: {}", e))?;

        let client = pool
            .get()
            .await
            .map_err(|e| format!("Connection failed: {}", e))?;

        client
            .simple_query("SELECT 1")
            .await
            .map_err(|e| format!("Query failed: {}", e))?;

        Ok(pool)
    }
}
