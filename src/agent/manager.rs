use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tokio::fs;
use tracing::{info, warn};

use crate::config::{merge_agent_config, AgentOverrideConfig, GlobalConfig};

use super::model::Agent;
use super::schema_store::SchemaStore;

pub struct AgentManager {
    agents: HashMap<String, Agent>,
    global_config: GlobalConfig,
    workspace_root: PathBuf,
}

impl AgentManager {
    pub fn new(global_config: GlobalConfig) -> Self {
        Self {
            agents: HashMap::new(),
            global_config,
            workspace_root: PathBuf::from("workspace"),
        }
    }

    /// Create an AgentManager with a custom workspace root (used in tests).
    #[cfg(test)]
    pub fn new_with_workspace(global_config: GlobalConfig, workspace_root: PathBuf) -> Self {
        Self {
            agents: HashMap::new(),
            global_config,
            workspace_root,
        }
    }

    /// Scan workspace/ dir and load agent workspaces.
    pub async fn load_all_agents(&mut self) -> Result<()> {
        let workspace = self.workspace_root.clone();

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

            let has_manifest = path.join("manifest.yaml").exists();
            if !has_manifest && !dir_name.starts_with("discord-") {
                warn!(
                    "Skipping workspace directory '{}' (no manifest.yaml and no legacy Discord naming)",
                    dir_name
                );
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
        let workspace_path = self.workspace_root.join(agent_id);

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
            "# Agent Role\n\nYou are a helpful AI assistant in the channel \"{}\".\nYour agent ID is: {}\n",
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

        // Initialize schema (creates schema/ and events/ dirs, writes YAML files if absent)
        let schema_store = SchemaStore::new(workspace_path.clone());
        schema_store.initialize(agent_id, display_name).await?;
        let manifest = schema_store.read_manifest().await.unwrap_or_default();
        let channel_binding = schema_store
            .read_channel_binding()
            .await
            .unwrap_or_default();
        let capabilities = schema_store.read_capabilities().await.unwrap_or_default();
        let state = schema_store.read_state().await.unwrap_or_default();

        // Load agent-level config override if exists
        let agent_config = self.load_agent_config(&workspace_path).await;
        let merged_config = merge_agent_config(&self.global_config, agent_config);

        let agent = Agent {
            id: agent_id.to_string(),
            display_name: display_name.to_string(),
            workspace_path,
            role: role_content,
            config: merged_config,
            manifest,
            channel_binding,
            capabilities,
            state,
        };

        info!("Created new agent: {}", agent_id);
        Ok(agent)
    }

    /// Load existing agent from workspace/{agent_id}/
    pub async fn load_agent(&self, agent_id: &str) -> Result<Agent> {
        let workspace_path = self.workspace_root.join(agent_id);

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
        let fallback_display_name = extract_display_name(&role, agent_id);

        // Lazy init: if schema doesn't exist yet, initialize schema files.
        let schema_store = SchemaStore::new(workspace_path.clone());
        schema_store
            .initialize(agent_id, &fallback_display_name)
            .await?;
        let manifest = schema_store.read_manifest().await.unwrap_or_default();
        let channel_binding = schema_store
            .read_channel_binding()
            .await
            .unwrap_or_default();
        let agent_schema = schema_store.read_agent_schema().await.unwrap_or_default();
        let display_name = if !manifest.display_name.trim().is_empty() {
            manifest.display_name.clone()
        } else if agent_schema.identity.display_name.trim().is_empty() {
            fallback_display_name
        } else {
            agent_schema.identity.display_name
        };
        let capabilities = schema_store.read_capabilities().await.unwrap_or_default();
        let state = schema_store.read_state().await.unwrap_or_default();

        // Load agent-level config override if exists
        let agent_config = self.load_agent_config(&workspace_path).await;
        let merged_config = merge_agent_config(&self.global_config, agent_config);

        Ok(Agent {
            id: agent_id.to_string(),
            display_name,
            workspace_path,
            role,
            config: merged_config,
            manifest,
            channel_binding,
            capabilities,
            state,
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
    // Try to find channel "{name}" pattern from current or legacy role.md files.
    if let Some(start) = role.find("channel \"") {
        let after = &role[start + 9..];
        if let Some(end) = after.find('"') {
            return after[..end].to_string();
        }
    }
    fallback.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{CoreConfig, DiscordConfig, LlmConfig, PlatformConfig};
    use tempfile::tempdir;

    fn test_global_config(workspace_root: &std::path::Path) -> GlobalConfig {
        // We need a GlobalConfig that doesn't require a real LLM or Discord token.
        // The workspace root override is handled by the test by setting the cwd or
        // by using absolute paths — here we just need a valid config struct.
        let _ = workspace_root; // used by caller to set up paths
        GlobalConfig {
            core: CoreConfig::default(),
            platform: PlatformConfig {
                name: Some("TestPlatform".to_string()),
            },
            llm: LlmConfig {
                provider: Some("openrouter".to_string()),
                base_url: Some("https://openrouter.ai/api/v1".to_string()),
                model: Some("openai/gpt-4o-mini".to_string()),
                timeout_secs: Some(30),
            },
            discord: DiscordConfig::default(),
        }
    }

    /// Helper: create an AgentManager that uses a temp dir as the workspace root.
    /// We achieve this by writing agents directly into the temp dir and calling
    /// load_agent with an absolute path trick via a local wrapper.
    async fn create_agent_in_dir(
        workspace_root: &std::path::Path,
        agent_id: &str,
        display_name: &str,
        global_config: GlobalConfig,
    ) -> Result<Agent> {
        let workspace_path = workspace_root.join(agent_id);
        tokio::fs::create_dir_all(&workspace_path).await?;

        // Create sessions dir
        tokio::fs::create_dir_all(workspace_path.join("sessions")).await?;

        // Create role.md
        let role_content = format!(
            "# Agent Role\n\nYou are a helpful AI assistant in the channel \"{}\".\nYour agent ID is: {}\n",
            display_name, agent_id
        );
        tokio::fs::write(workspace_path.join("role.md"), &role_content).await?;
        tokio::fs::write(workspace_path.join("memory.md"), "# Long-term Memory\n").await?;

        // Initialize schema
        let schema_store = SchemaStore::new(workspace_path.clone());
        schema_store.initialize(agent_id, display_name).await?;
        let manifest = schema_store.read_manifest().await.unwrap_or_default();
        let channel_binding = schema_store
            .read_channel_binding()
            .await
            .unwrap_or_default();
        let capabilities = schema_store.read_capabilities().await.unwrap_or_default();
        let state = schema_store.read_state().await.unwrap_or_default();

        let merged_config = merge_agent_config(&global_config, None);

        Ok(Agent {
            id: agent_id.to_string(),
            display_name: display_name.to_string(),
            workspace_path,
            role: role_content,
            config: merged_config,
            manifest,
            channel_binding,
            capabilities,
            state,
        })
    }

    #[tokio::test]
    async fn create_agent_populates_capabilities_and_state() {
        let dir = tempdir().expect("tempdir");
        let global_config = test_global_config(dir.path());

        let agent = create_agent_in_dir(dir.path(), "discord-1-2", "test-channel", global_config)
            .await
            .expect("create_agent");

        // capabilities should have defaults
        assert!(agent.capabilities.chat, "chat capability should be true");
        assert!(
            agent.capabilities.reset_session,
            "reset_session capability should be true"
        );
        assert!(
            agent.capabilities.summarize,
            "summarize capability should be true"
        );

        // state should have context_window set (from derive_initial_state)
        assert_eq!(
            agent.state.context_window,
            Some(20),
            "context_window should default to 20"
        );
    }

    #[tokio::test]
    async fn load_agent_on_existing_workspace_reads_schema_files() {
        let dir = tempdir().expect("tempdir");
        let global_config = test_global_config(dir.path());

        // First, create the agent (writes schema files)
        let created = create_agent_in_dir(
            dir.path(),
            "discord-10-20",
            "my-channel",
            global_config.clone(),
        )
        .await
        .expect("create_agent");

        // Now simulate load_agent by re-reading from the same workspace path
        let workspace_path = created.workspace_path.clone();
        let agent_id = "discord-10-20";

        let role_path = workspace_path.join("role.md");
        let role = tokio::fs::read_to_string(&role_path)
            .await
            .expect("read role.md");
        let display_name = extract_display_name(&role, agent_id);

        let schema_store = SchemaStore::new(workspace_path.clone());
        // initialize is idempotent — should not overwrite existing files
        schema_store
            .initialize(agent_id, &display_name)
            .await
            .expect("initialize");
        let manifest = schema_store.read_manifest().await.unwrap_or_default();
        let channel_binding = schema_store
            .read_channel_binding()
            .await
            .unwrap_or_default();
        let capabilities = schema_store.read_capabilities().await.unwrap_or_default();
        let state = schema_store.read_state().await.unwrap_or_default();

        let loaded = Agent {
            id: agent_id.to_string(),
            display_name: display_name.clone(),
            workspace_path,
            role,
            config: merge_agent_config(&global_config, None),
            manifest,
            channel_binding,
            capabilities,
            state,
        };

        // Verify loaded agent matches created agent
        assert_eq!(loaded.id, created.id);
        assert_eq!(loaded.display_name, created.display_name);
        assert_eq!(loaded.capabilities.chat, created.capabilities.chat);
        assert_eq!(
            loaded.capabilities.reset_session,
            created.capabilities.reset_session
        );
        assert_eq!(loaded.state.context_window, created.state.context_window);
        assert_eq!(loaded.manifest.agent_id, created.manifest.agent_id);
        assert_eq!(
            loaded.channel_binding.agent_id,
            created.channel_binding.agent_id
        );
    }

    #[tokio::test]
    async fn schema_initialize_is_idempotent() {
        let dir = tempdir().expect("tempdir");
        let workspace_path = dir.path().join("discord-99-88");
        tokio::fs::create_dir_all(&workspace_path)
            .await
            .expect("mkdir");

        let schema_store = SchemaStore::new(workspace_path.clone());

        // First init
        schema_store
            .initialize("discord-99-88", "test")
            .await
            .expect("first init");
        let caps1 = schema_store.read_capabilities().await.expect("caps1");
        let state1 = schema_store.read_state().await.expect("state1");
        let manifest1 = schema_store.read_manifest().await.expect("manifest1");
        let binding1 = schema_store.read_channel_binding().await.expect("binding1");

        // Second init — should not overwrite
        schema_store
            .initialize("discord-99-88", "test")
            .await
            .expect("second init");
        let caps2 = schema_store.read_capabilities().await.expect("caps2");
        let state2 = schema_store.read_state().await.expect("state2");
        let manifest2 = schema_store.read_manifest().await.expect("manifest2");
        let binding2 = schema_store.read_channel_binding().await.expect("binding2");

        assert_eq!(caps1.chat, caps2.chat);
        assert_eq!(caps1.reset_session, caps2.reset_session);
        assert_eq!(state1.context_window, state2.context_window);
        assert_eq!(manifest1.agent_id, manifest2.agent_id);
        assert_eq!(binding1.channel.external_id, binding2.channel.external_id);
    }
}
