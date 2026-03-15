//! `gyre health` — lightweight health check for Docker and monitoring.
//!
//! Exits 0 if the gateway is reachable, 1 otherwise.
//! Designed to be fast and side-effect-free for use in Docker HEALTHCHECK
//! and orchestration probes (liveness/readiness).

/// Run the health check.
///
/// Tries to reach the local gateway at `GATEWAY_HOST:GATEWAY_PORT`.
/// Falls back to `127.0.0.1:3000` if not configured.
pub async fn run_health_command() -> anyhow::Result<()> {
    let host = std::env::var("GATEWAY_HOST").unwrap_or_else(|_| "127.0.0.1".into());
    let port = std::env::var("GATEWAY_PORT").unwrap_or_else(|_| "3000".into());

    let url = format!("http://{}:{}/api/health", host, port);

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()?;

    match client.get(&url).send().await {
        Ok(resp) if resp.status().is_success() => {
            println!("healthy");
            Ok(())
        }
        Ok(resp) => {
            eprintln!("unhealthy: HTTP {}", resp.status());
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("unhealthy: {e}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    // Health check hits a live server, so we just verify the module compiles
    // and the function signature is correct. Integration tests cover the rest.

    #[test]
    fn health_command_exists() {
        // Compile-time check that run_health_command is async and returns anyhow::Result
        let _: fn() -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<()>>>> =
            || Box::pin(super::run_health_command());
    }
}
