//! `gyre template` CLI subcommands.
//!
//! Commands:
//! * `gyre template list [--installed] [--tag TAG]`
//! * `gyre template install <name>`
//! * `gyre template publish <path>`
//! * `gyre template login <api-key>`
//! * `gyre template uninstall <name>`

use std::path::PathBuf;

use clap::Subcommand;

use crate::template::{self, DEFAULT_REGISTRY_URL};

// ---------------------------------------------------------------------------
// Subcommand enum
// ---------------------------------------------------------------------------

#[derive(Subcommand, Debug, Clone)]
pub enum TemplateCommand {
    /// List available templates from the registry (or installed templates)
    List {
        /// Show only locally installed templates
        #[arg(long)]
        installed: bool,

        /// Filter by tag (e.g. --tag finance)
        #[arg(long)]
        tag: Option<String>,

        /// Registry base URL
        #[arg(long, default_value = DEFAULT_REGISTRY_URL, env = "GYRE_REGISTRY_URL")]
        registry: String,
    },

    /// Install a template from the registry
    Install {
        /// Template name: bare (`kimi`) or namespaced (`author/kimi`)
        name: String,

        /// Registry base URL
        #[arg(long, default_value = DEFAULT_REGISTRY_URL, env = "GYRE_REGISTRY_URL")]
        registry: String,
    },

    /// Package and publish a template to the registry
    Publish {
        /// Path to the agent box directory containing manifest.toml
        path: PathBuf,

        /// Registry base URL
        #[arg(long, default_value = DEFAULT_REGISTRY_URL, env = "GYRE_REGISTRY_URL")]
        registry: String,
    },

    /// Save your registry API key locally (~/.gyre/registry.key)
    Login {
        /// API key obtained from registry.gyre.ai
        api_key: String,
    },

    /// Remove an installed template
    Uninstall {
        /// Template name: bare (`kimi`) or namespaced (`author/kimi`)
        name: String,
    },
}

// ---------------------------------------------------------------------------
// Command runner
// ---------------------------------------------------------------------------

pub async fn run_template_command(cmd: TemplateCommand) -> anyhow::Result<()> {
    match cmd {
        TemplateCommand::List {
            installed,
            tag,
            registry,
        } => cmd_list(installed, tag, &registry).await,

        TemplateCommand::Install { name, registry } => template::install(&name, &registry).await,

        TemplateCommand::Publish { path, registry } => {
            let api_key = template::read_api_key()?;
            template::publish(&path, &registry, &api_key).await
        }

        TemplateCommand::Login { api_key } => cmd_login(&api_key),

        TemplateCommand::Uninstall { name } => cmd_uninstall(&name),
    }
}

// ---------------------------------------------------------------------------
// list
// ---------------------------------------------------------------------------

async fn cmd_list(installed: bool, tag: Option<String>, registry_url: &str) -> anyhow::Result<()> {
    if installed {
        let manifests = template::list_installed()?;

        if manifests.is_empty() {
            println!("No templates installed.");
            println!();
            println!("Browse available templates: gyre template list");
            println!("Install one:                gyre template install <name>");
            return Ok(());
        }

        println!(
            "Installed Templates ({})",
            template::templates_dir().display()
        );
        println!("{}", "─".repeat(60));

        for m in &manifests {
            let t = &m.template;
            let tags = if t.tags.is_empty() {
                String::new()
            } else {
                format!("  [{}]", t.tags.join(", "))
            };
            println!(
                "  {}/{:<30} v{}  {}{}",
                t.author, t.name, t.version, t.kind, tags
            );
        }

        println!("{}", "─".repeat(60));
        println!("Run: gyre template uninstall <name>");
    } else {
        let templates: Vec<template::TemplateMeta> =
            template::list(registry_url, tag.as_deref()).await?;

        if templates.is_empty() {
            println!("No templates found.");
            return Ok(());
        }

        println!("Available Templates ({})", registry_url);
        println!("{}", "─".repeat(60));

        for t in &templates {
            let rating_str = match t.rating {
                Some(r) => format!("★{:.1}", r),
                None => "    ".to_string(),
            };
            let installs = format_installs(t.downloads);
            println!(
                "  {:<40} v{}  {}  {} installs",
                t.full_name(),
                t.version,
                rating_str,
                installs,
            );
        }

        println!("{}", "─".repeat(60));
        println!("Run: gyre template install <name>");
    }

    Ok(())
}

fn format_installs(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else {
        format!("{}", n)
    }
}

// ---------------------------------------------------------------------------
// login
// ---------------------------------------------------------------------------

fn cmd_login(api_key: &str) -> anyhow::Result<()> {
    if api_key.is_empty() {
        anyhow::bail!("API key cannot be empty");
    }

    template::write_api_key(api_key)?;

    let key_path = template::registry_key_path();
    println!("✅ API key saved to {}", key_path.display());
    println!();
    println!("You can now publish templates with:");
    println!("  gyre template publish <path>");

    Ok(())
}

// ---------------------------------------------------------------------------
// uninstall
// ---------------------------------------------------------------------------

fn cmd_uninstall(name: &str) -> anyhow::Result<()> {
    let (author, template_name) = template::resolve_name(name);
    let install_dir = template::templates_dir().join(format!("{}-{}", author, template_name));

    if !install_dir.exists() {
        anyhow::bail!(
            "Template '{}/{}' is not installed (looked in {}).",
            author,
            template_name,
            install_dir.display()
        );
    }

    std::fs::remove_dir_all(&install_dir)?;
    println!("✅ Template '{}/{}' removed.", author, template_name);

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_installs() {
        assert_eq!(format_installs(0), "0");
        assert_eq!(format_installs(500), "500");
        assert_eq!(format_installs(1500), "1.5K");
        assert_eq!(format_installs(2300), "2.3K");
        assert_eq!(format_installs(1_200_000), "1.2M");
    }
}
