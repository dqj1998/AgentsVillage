use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tokio::fs;
use tracing::{info, warn};

use crate::config::{merge_agent_config, AgentOverrideConfig, GlobalConfig};

use super::model::Agent;

pub struct AgentManager {
    agents: HashMap<String, Agent>,
    global_config: GlobalConfig,
}

impl AgentManager {
    pub fn new(global_config: GlobalConfig) -> Self {
        Self {
            agents: HashMap::new(),
            global_config,
        }
    }

    /// Scan workspace/ dir, load all agents matching naming convention
    pub async fn load_all_agents(&mut self) -> Result<()> {
        let workspace = PathBuf::from("workspace");

        if !workspace.exists() {
            fs::create_dir_all(&workspace)
                .await
                .context("Failed to create workspace directory")?;
            return Ok(());
        }

        let mut entries = fs::read_dir(&workspace)
            .await
            .context("Failed to read workspace directory")?;

        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }

            let dir_name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();

            // Only load agents matching the naming convention: discord-{guild_id}-{channel_id}[- {thread_id}]
            if !dir_name.starts_with("discord-") {
                warn!("Skipping workspace directory '{}' (doesn't match naming convention)", dir_name);
                continue;
            }

            match self.load_agent(&dir_name).await {
                Ok(agent) => {
                    info!("Loaded agent: {}", dir_name);
                    self.agents.insert(dir_name, agent);
                }
                Err(e) => {
                    warn!("Failed to load agent '{}': {}", dir_name, e);
                }
            }
        }

        info!("Loaded {} agents from workspace", self.agents.len());
        Ok(())
    }

    /// Get agent by ID, create if not exists
    pub async fn get_or_create_agent(
        &mut self,
        agent_id: &str,
        display_name: &str,
    ) -> Result<&Agent> {
        if !self.agents.contains_key(agent_id) {
            let agent = self.create_agent(agent_id, display_name).await?;
            self.agents.insert(agent_id.to_string(), agent);
        }
        Ok(self.agents.get(agent_id).unwrap())
    }

    /// Create new agent: mkdir workspace/{agent_id}/, create role.md, memory.md, sessions/
    pub async fn create_agent(&mut self, agent_id: &str, display_name: &str) -> Result<Agent> {
        let workspace_path = PathBuf::from("workspace").join(agent_id);

        fs::create_dir_all(&workspace_path)
            .await
            .context("Failed to create agent workspace directory")?;

        // Create sessions directory
        let sessions_dir = workspace_path.join("sessions");
        fs::create_dir_all(&sessions_dir)
            .await
            .context("Failed to create sessions directory")?;

        // Create role.md
        let role_content = format!(
            "# Agent Role\n\nYou are a helpful AI assistant in the Discord channel \"{}\".\nYour agent ID is: {}\n",
            display_name, agent_id
        );
        let role_path = workspace_path.join("role.md");
        if !role_path.exists() {
            fs::write(&role_path, &role_content)
                .await
                .context("Failed to create role.md")?;
        }

        // Create memory.md
        let memory_path = workspace_path.join("memory.md");
        if !memory_path.exists() {
            fs::write(&memory_path, "# Long-term Memory\n")
                .await
                .context("Failed to create memory.md")?;
        }

        // Load agent-level config override if exists
        let agent_config = self.load_agent_config(&workspace_path).await;
        let merged_config = merge_agent_config(&self.global_config, agent_config);

        let agent = Agent {
            id: agent_id.to_string(),
            display_name: display_name.to_string(),
            workspace_path,
            role: role_content,
            config: merged_config,
        };

        info!("Created new agent: {}", agent_id);
        Ok(agent)
    }

    /// Load existing agent from workspace/{agent_id}/
    pub async fn load_agent(&self, agent_id: &str) -> Result<Agent> {
        let workspace_path = PathBuf::from("workspace").join(agent_id);

        if !workspace_path.exists() {
            anyhow::bail!("Agent workspace not found: {:?}", workspace_path);
        }

        // Read role.md
        let role_path = workspace_path.join("role.md");
        let role = if role_path.exists() {
            fs::read_to_string(&role_path)
                .await
                .context("Failed to read role.md")?
        } else {
            format!(
                "# Agent Role\n\nYou are a helpful AI assistant.\nYour agent ID is: {}\n",
                agent_id
            )
        };

        // Extract display name from role.md or use agent_id
        let display_name = extract_display_name(&role, agent_id);

        // Load agent-level config override if exists
        let agent_config = self.load_agent_config(&workspace_path).await;
        let merged_config = merge_agent_config(&self.global_config, agent_config);

        Ok(Agent {
            id: agent_id.to_string(),
            display_name,
            workspace_path,
            role,
            config: merged_config,
        })
    }

    /// Load agent-level config override from workspace/{agent_id}/config.toml
    async fn load_agent_config(&self, workspace_path: &Path) -> Option<AgentOverrideConfig> {
        let config_path = workspace_path.join("config.toml");
        if !config_path.exists() {
            return None;
        }

        let content = fs::read_to_string(&config_path).await.ok()?;
        toml::from_str(&content).ok()
    }

    /// Get a reference to an agent by ID
    #[allow(dead_code)]
    pub fn get_agent(&self, agent_id: &str) -> Option<&Agent> {
        self.agents.get(agent_id)
    }
}

/// Extract display name from role.md content
fn extract_display_name(role: &str, fallback: &str) -> String {
    // Try to find "Discord channel "{name}"" pattern
    if let Some(start) = role.find("Discord channel \"") {
        let after = &role[start + 17..];
        if let Some(end) = after.find('"') {
            return after[..end].to_string();
        }
    }
    fallback.to_string()
}
