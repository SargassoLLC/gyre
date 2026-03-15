//! Stage 7: Agent Creation (Multi-Agent)
//!
//! Absorbs cli/init.rs hermit box creation. Supports creating multiple
//! agents with personality Q&A, LLM overrides, and primary agent marking.

use std::path::PathBuf;

use async_trait::async_trait;

use super::{SetupError, SetupStage, StageOutcome};
use crate::settings::{AgentDefinition, MultiAgentSettings};
use crate::setup::state::SetupState;
use crate::setup::ui::SetupUi;

pub struct AgentsStage;

#[async_trait]
impl SetupStage for AgentsStage {
    fn id(&self) -> &'static str {
        "agents"
    }

    fn name(&self) -> &'static str {
        "Agent Setup"
    }

    async fn run(&self, state: &mut SetupState, ui: &SetupUi) -> Result<StageOutcome, SetupError> {
        let mut agents: Vec<AgentDefinition> = Vec::new();

        // QuickStart: create one agent with defaults.
        if state.quickstart {
            let agent = self.create_quick_agent(state, ui)?;
            agents.push(agent);
        } else {
            // Advanced: interactive multi-agent loop.
            loop {
                let agent = self.create_agent_interactive(state, ui, agents.is_empty())?;
                agents.push(agent);

                let more = ui.confirm("Add another agent?", false)?;
                if !more {
                    break;
                }
            }
        }

        // Ensure exactly one primary agent.
        if !agents.iter().any(|a| a.primary) && !agents.is_empty() {
            agents[0].primary = true;
        }

        let primary_id = agents.iter().find(|a| a.primary).map(|a| a.id.clone());

        state.settings.multi_agent = MultiAgentSettings {
            default_agent: primary_id,
            agents_dir: Some(state.agents_dir.clone()),
            agents,
        };

        let agent_count = state.settings.multi_agent.agents.len();
        ui.success(&format!("{} agent(s) configured.", agent_count));

        Ok(StageOutcome::Completed)
    }
}

impl AgentsStage {
    fn create_quick_agent(
        &self,
        state: &mut SetupState,
        ui: &SetupUi,
    ) -> Result<AgentDefinition, SetupError> {
        ui.info("Creating default agent...");

        let name = "gyre".to_string();
        let id = sanitize_agent_id(&name);

        // Create hermit box directory.
        let box_path = state.agents_dir.join(format!("{}_box", id));
        self.create_hermit_box(&box_path, &name)?;

        ui.success(&format!("Agent '{}' created at {}", id, box_path.display()));

        Ok(AgentDefinition {
            id,
            name: Some(name),
            llm_backend: None,
            model: None,
            channels: Vec::new(),
            primary: true,
        })
    }

    fn create_agent_interactive(
        &self,
        state: &mut SetupState,
        ui: &SetupUi,
        is_first: bool,
    ) -> Result<AgentDefinition, SetupError> {
        let name = ui.input("Agent name")?;
        let id = sanitize_agent_id(&name);

        if id.is_empty() || id.len() > 32 {
            return Err(SetupError::Validation(
                "Agent name must be 1-32 characters (alphanumeric, hyphens, underscores)"
                    .to_string(),
            ));
        }

        let display_name = ui.optional_input("Display name (optional)")?;

        // LLM override.
        let override_llm = ui.confirm("Override global LLM settings for this agent?", false)?;
        let (llm_backend, model) = if override_llm {
            let backend = ui.optional_input("LLM backend (or blank for global)")?;
            let model = ui.optional_input("Model (or blank for global)")?;
            (backend, model)
        } else {
            (None, None)
        };

        // Primary agent.
        let primary = if is_first {
            ui.info("This will be the primary agent (receives unbound messages).");
            true
        } else {
            ui.confirm("Make this the primary agent?", false)?
        };

        // Create hermit box.
        let box_path = state.agents_dir.join(format!("{}_box", id));
        self.create_hermit_box(&box_path, &name)?;

        // Personality Q&A (soul.md content).
        let soul_content = self.personality_qa(ui, &name)?;
        let soul_path = box_path.join("soul.md");
        std::fs::write(&soul_path, soul_content).map_err(|e| SetupError::Io(e))?;

        ui.success(&format!("Agent '{}' created at {}", id, box_path.display()));

        Ok(AgentDefinition {
            id,
            name: display_name.or(Some(name)),
            llm_backend,
            model,
            channels: Vec::new(),
            primary,
        })
    }

    fn create_hermit_box(&self, path: &PathBuf, agent_name: &str) -> Result<(), SetupError> {
        // Create directory structure.
        let dirs = ["memory", "knowledge", "axioms", "TELOS"];
        for dir in &dirs {
            std::fs::create_dir_all(path.join(dir)).map_err(|e| SetupError::Io(e))?;
        }

        // Create seed files.
        let soul = format!(
            "# {}\n\nI am {}, a personal AI assistant built on Gyre.\n\n\
             ## Core Values\n\n- Helpful and accurate\n- Respectful of user privacy\n\
             - Transparent about capabilities and limitations\n",
            agent_name, agent_name
        );

        std::fs::write(path.join("soul.md"), soul).map_err(|e| SetupError::Io(e))?;

        // TELOS seed files.
        std::fs::write(
            path.join("TELOS").join("mission.md"),
            format!(
                "# Mission\n\nAssist the user effectively as {}.\n",
                agent_name
            ),
        )
        .map_err(|e| SetupError::Io(e))?;

        std::fs::write(
            path.join("TELOS").join("goals.md"),
            "# Goals\n\n- Respond accurately to queries\n- Learn from interactions\n",
        )
        .map_err(|e| SetupError::Io(e))?;

        Ok(())
    }

    fn personality_qa(&self, ui: &SetupUi, agent_name: &str) -> Result<String, SetupError> {
        ui.info("Let's define the agent's personality.");
        ui.blank();

        let style = ui.select_one(
            "Communication style",
            &[
                "Professional & precise",
                "Friendly & conversational",
                "Technical & detailed",
                "Concise & direct",
            ],
        )?;

        let style_desc = match style {
            0 => "professional and precise",
            1 => "friendly and conversational",
            2 => "technical and detailed",
            3 => "concise and direct",
            _ => "helpful",
        };

        let domain =
            ui.optional_input("Primary domain/expertise (optional, e.g., 'software engineering')")?;

        let mut soul = format!(
            "# {}\n\nI am {}, a personal AI assistant.\n\n\
             ## Communication Style\n\nI communicate in a {} manner.\n",
            agent_name, agent_name, style_desc,
        );

        if let Some(ref d) = domain {
            soul.push_str(&format!("\n## Expertise\n\nMy primary domain is {}.\n", d));
        }

        soul.push_str(
            "\n## Core Values\n\n- Accuracy and reliability\n\
             - Respect for user privacy\n- Transparency about limitations\n",
        );

        Ok(soul)
    }
}

/// Sanitize a name into a valid agent ID (lowercase alphanumeric + hyphens + underscores).
fn sanitize_agent_id(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c.to_lowercase().next().unwrap_or(c)
            } else if c == ' ' {
                '-'
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim_matches(|c: char| c == '-' || c == '_')
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_agent_id_basic() {
        assert_eq!(sanitize_agent_id("Kimi"), "kimi");
        assert_eq!(sanitize_agent_id("My Agent"), "my-agent");
        assert_eq!(sanitize_agent_id("test_bot"), "test_bot");
        assert_eq!(sanitize_agent_id("  spaces  "), "spaces");
    }
}
