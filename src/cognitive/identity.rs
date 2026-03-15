use crate::cognitive::HermitBox;

/// Identity files loaded from a HermitBox folder-world.
#[derive(Clone)]
pub struct AgentIdentityFiles {
    pub soul: String,
    pub user_context: String,
    pub memory_summary: String,
    // Telos files
    pub mission: String,
    pub goals: String,
    pub beliefs: String,
    pub experiences: String,
    pub boundaries: String,
}

impl AgentIdentityFiles {
    /// Load identity files from a HermitBox.
    pub fn load(hermit_box: &HermitBox) -> Self {
        Self {
            soul: hermit_box.read_soul(),
            user_context: hermit_box.read_user(),
            memory_summary: hermit_box.read_memory_summary(),
            mission: hermit_box.read_telos_file("MISSION.md"),
            goals: hermit_box.read_telos_file("GOALS.md"),
            beliefs: hermit_box.read_telos_file("BELIEFS.md"),
            experiences: hermit_box.read_telos_file("EXPERIENCES.md"),
            boundaries: hermit_box.read_telos_file("BOUNDARIES.md"),
        }
    }

    /// Format as a system prompt block.
    ///
    /// Returns an empty string if all identity files are empty.
    pub fn system_prompt_block(&self) -> String {
        let mut parts = Vec::new();

        if !self.soul.is_empty() {
            parts.push(format!("### Soul\n{}\n", self.soul));
        }
        if !self.user_context.is_empty() {
            parts.push(format!("### User Context\n{}\n", self.user_context));
        }
        if !self.memory_summary.is_empty() {
            parts.push(format!("### Memory Summary\n{}\n", self.memory_summary));
        }

        // Telos section (non-empty files only)
        let mut telos_parts = Vec::new();
        if !self.mission.is_empty() {
            telos_parts.push(format!("### Mission\n{}\n", self.mission));
        }
        if !self.beliefs.is_empty() {
            telos_parts.push(format!("### Beliefs\n{}\n", self.beliefs));
        }
        if !self.experiences.is_empty() {
            telos_parts.push(format!("### Experiences\n{}\n", self.experiences));
        }
        if !self.boundaries.is_empty() {
            telos_parts.push(format!("### Boundaries\n{}\n", self.boundaries));
        }
        if !self.goals.is_empty() {
            telos_parts.push(format!("### Goals\n{}\n", self.goals));
        }

        if !telos_parts.is_empty() {
            parts.push(format!("## Agent Telos\n\n{}", telos_parts.join("\n")));
        }

        if parts.is_empty() {
            return String::new();
        }

        format!("## Agent Identity\n\n{}", parts.join("\n"))
    }

    /// Persist the memory summary back to the HermitBox.
    pub fn save_memory_summary(&self, hermit_box: &HermitBox, summary: &str) -> Result<(), String> {
        hermit_box.write_memory_summary(summary)
    }
}
