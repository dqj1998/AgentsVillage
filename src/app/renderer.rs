use anyhow::Result;

use crate::agent::memory::MemoryManager;
use crate::agent::model::Agent;

/// Renders agent schema, state, and long-term memory into a system prompt string.
/// It currently reads role.md and memory.md as the prompt and memory sources.
pub struct SchemaRenderer;

impl SchemaRenderer {
    pub fn new() -> Self {
        Self
    }

    /// Render the system prompt for an agent.
    /// Currently reads role.md (via agent.role) and memory.md (via MemoryManager).
    /// The capabilities and state fields on Agent are available for future rendering.
    pub async fn render_system_prompt(
        &self,
        agent: &Agent,
        memory_manager: &MemoryManager,
    ) -> Result<String> {
        // Read long-term memory
        let memory_content = memory_manager.read_memory().await?;

        // Build system prompt from role + memory.
        let system_prompt =
            if memory_content.trim().is_empty() || memory_content.trim() == "# Long-term Memory" {
                agent.role.clone()
            } else {
                format!("{}\n\n## Long-term Memory\n{}", agent.role, memory_content)
            };

        // Future: render capabilities, state, event-derived summaries here
        // For now, append a capabilities summary if any non-default caps are set
        let caps = &agent.capabilities;
        let mut extras = Vec::new();
        if !caps.commands {
            // commands disabled — no extra note needed
        }
        if !caps.summarize || !caps.memory_policy.summarize_old_messages {
            extras.push("Note: memory summarization is disabled for this agent.".to_string());
        }

        if extras.is_empty() {
            Ok(system_prompt)
        } else {
            Ok(format!("{}\n\n{}", system_prompt, extras.join("\n")))
        }
    }
}

impl Default for SchemaRenderer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::model::Agent;
    use crate::app::schema::{AgentCapabilities, AgentManifest, AgentState, ChannelBinding};
    use crate::config::GlobalConfig;
    use std::path::PathBuf;
    use tempfile::tempdir;

    fn make_agent(workspace_path: PathBuf) -> Agent {
        Agent {
            id: "test-agent".to_string(),
            display_name: "Test".to_string(),
            workspace_path: workspace_path.clone(),
            role: "You are a test assistant.".to_string(),
            config: GlobalConfig::default(),
            manifest: AgentManifest::initial("test-agent", "Test"),
            channel_binding: ChannelBinding::initial("test-agent", "Test"),
            capabilities: AgentCapabilities::default(),
            state: AgentState::default(),
        }
    }

    #[tokio::test]
    async fn render_system_prompt_no_memory() {
        let dir = tempdir().unwrap();
        let agent = make_agent(dir.path().to_path_buf());
        let memory_manager = MemoryManager::new(dir.path().to_path_buf());
        let renderer = SchemaRenderer::new();

        let prompt = renderer
            .render_system_prompt(&agent, &memory_manager)
            .await
            .unwrap();
        assert!(prompt.contains("You are a test assistant."));
        assert!(!prompt.contains("Long-term Memory"));
    }

    #[tokio::test]
    async fn render_system_prompt_with_memory() {
        let dir = tempdir().unwrap();
        let agent = make_agent(dir.path().to_path_buf());
        let memory_manager = MemoryManager::new(dir.path().to_path_buf());
        memory_manager
            .append_memory("User likes Rust.")
            .await
            .unwrap();

        let renderer = SchemaRenderer::new();
        let prompt = renderer
            .render_system_prompt(&agent, &memory_manager)
            .await
            .unwrap();
        assert!(prompt.contains("You are a test assistant."));
        assert!(prompt.contains("Long-term Memory"));
        assert!(prompt.contains("User likes Rust."));
    }

    #[tokio::test]
    async fn render_system_prompt_summarize_disabled_adds_note() {
        let dir = tempdir().unwrap();
        let mut agent = make_agent(dir.path().to_path_buf());
        agent.capabilities.memory_policy.summarize_old_messages = false;
        let memory_manager = MemoryManager::new(dir.path().to_path_buf());
        let renderer = SchemaRenderer::new();

        let prompt = renderer
            .render_system_prompt(&agent, &memory_manager)
            .await
            .unwrap();
        assert!(prompt.contains("memory summarization is disabled"));
    }

    #[tokio::test]
    async fn render_system_prompt_only_header_memory_treated_as_empty() {
        let dir = tempdir().unwrap();
        let agent = make_agent(dir.path().to_path_buf());
        let memory_manager = MemoryManager::new(dir.path().to_path_buf());

        // Write a memory file with only the header
        tokio::fs::write(dir.path().join("memory.md"), "# Long-term Memory")
            .await
            .unwrap();

        let renderer = SchemaRenderer::new();
        let prompt = renderer
            .render_system_prompt(&agent, &memory_manager)
            .await
            .unwrap();
        // Should not include the Long-term Memory section since content is just the header
        assert_eq!(prompt, "You are a test assistant.");
    }
}
