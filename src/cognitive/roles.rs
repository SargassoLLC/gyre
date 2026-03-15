/// Agent role hierarchy: Chief agents own memory and axioms; Workers get read-only access.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentRole {
    Chief,
    Worker,
}

#[derive(Debug, Clone)]
pub struct AgentIdentity {
    pub id: String,
    pub role: AgentRole,
    pub tribe: String,
    /// Namespace for memory isolation.
    /// Chiefs default to their own id; Workers inherit their parent chief's namespace.
    pub namespace: String,
}

impl AgentIdentity {
    pub fn chief(id: &str, tribe: &str) -> Self {
        Self {
            id: id.to_string(),
            role: AgentRole::Chief,
            tribe: tribe.to_string(),
            namespace: id.to_string(),
        }
    }

    pub fn worker(id: &str, tribe: &str, parent_chief: &str) -> Self {
        Self {
            id: id.to_string(),
            role: AgentRole::Worker,
            tribe: tribe.to_string(),
            namespace: parent_chief.to_string(),
        }
    }

    /// Chiefs can write memory directly; Workers cannot.
    pub fn can_write_memory(&self) -> bool {
        self.role == AgentRole::Chief
    }

    /// Chiefs can write axioms directly; Workers can only propose.
    pub fn can_write_axiom(&self) -> bool {
        self.role == AgentRole::Chief
    }
}
