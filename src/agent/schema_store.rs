use anyhow::{Context, Result};
use std::path::PathBuf;
use tokio::fs;
use tracing::{debug, info};

use crate::app::schema::{
    AgentCapabilities, AgentIdentity, AgentManifest, AgentSchema, AgentState, ChannelBinding,
};

/// Manages reading and writing of per-agent YAML schema files
/// under workspace/{agent_id}/schema/
pub struct SchemaStore {
    workspace_path: PathBuf,
}

impl SchemaStore {
    pub fn new(workspace_path: PathBuf) -> Self {
        Self { workspace_path }
    }

    fn schema_dir(&self) -> PathBuf {
        self.workspace_path.join("schema")
    }

    fn events_dir(&self) -> PathBuf {
        self.workspace_path.join("events")
    }

    fn manifest_path(&self) -> PathBuf {
        self.workspace_path.join("manifest.yaml")
    }

    fn channel_binding_path(&self) -> PathBuf {
        self.workspace_path.join("channel-binding.yaml")
    }

    /// Initialize schema directory structure.
    pub async fn initialize(&self, agent_id: &str, display_name: &str) -> Result<()> {
        let schema_dir = self.schema_dir();
        let events_dir = self.events_dir();

        fs::create_dir_all(&self.workspace_path)
            .await
            .context("Failed to create agent workspace directory")?;

        fs::create_dir_all(&schema_dir)
            .await
            .context("Failed to create schema directory")?;
        fs::create_dir_all(&events_dir)
            .await
            .context("Failed to create events directory")?;

        let manifest_path = self.manifest_path();
        if !manifest_path.exists() {
            let manifest = AgentManifest::initial(agent_id, display_name);
            self.write_manifest(&manifest).await?;
            info!("Initialized manifest.yaml for {}", agent_id);
        }

        let channel_binding_path = self.channel_binding_path();
        if !channel_binding_path.exists() {
            let binding = ChannelBinding::initial(agent_id, display_name);
            self.write_channel_binding(&binding).await?;
            info!("Initialized channel-binding.yaml for {}", agent_id);
        }

        // Write agent.yaml if not exists
        let agent_yaml_path = schema_dir.join("agent.yaml");
        if !agent_yaml_path.exists() {
            let schema = AgentSchema {
                identity: AgentIdentity {
                    id: agent_id.to_string(),
                    display_name: display_name.to_string(),
                    schema_version: Some(1),
                },
                schema_version: 1,
            };
            self.write_agent_schema(&schema).await?;
            info!("Initialized agent.yaml for {}", agent_id);
        }

        // Write capabilities.yaml if not exists
        let caps_path = schema_dir.join("capabilities.yaml");
        if !caps_path.exists() {
            let caps = AgentCapabilities::initial();
            self.write_capabilities(&caps).await?;
            info!("Initialized capabilities.yaml for {}", agent_id);
        }

        // Write state.yaml if not exists
        let state_path = schema_dir.join("state.yaml");
        if !state_path.exists() {
            let state = self.derive_initial_state().await;
            self.write_state(&state).await?;
            info!("Initialized state.yaml for {}", agent_id);
        }

        Ok(())
    }

    pub async fn read_manifest(&self) -> Result<AgentManifest> {
        let path = self.manifest_path();
        if !path.exists() {
            return Ok(AgentManifest::default());
        }
        let content = fs::read_to_string(&path)
            .await
            .context("Failed to read manifest.yaml")?;
        let manifest: AgentManifest =
            serde_yaml::from_str(&content).context("Failed to parse manifest.yaml")?;
        Ok(manifest)
    }

    pub async fn write_manifest(&self, manifest: &AgentManifest) -> Result<()> {
        let path = self.manifest_path();
        let content = serde_yaml::to_string(manifest).context("Failed to serialize manifest")?;
        fs::write(&path, content)
            .await
            .context("Failed to write manifest.yaml")?;
        Ok(())
    }

    pub async fn read_channel_binding(&self) -> Result<ChannelBinding> {
        let path = self.channel_binding_path();
        if !path.exists() {
            return Ok(ChannelBinding::default());
        }
        let content = fs::read_to_string(&path)
            .await
            .context("Failed to read channel-binding.yaml")?;
        let binding: ChannelBinding =
            serde_yaml::from_str(&content).context("Failed to parse channel-binding.yaml")?;
        Ok(binding)
    }

    pub async fn write_channel_binding(&self, binding: &ChannelBinding) -> Result<()> {
        let path = self.channel_binding_path();
        let content =
            serde_yaml::to_string(binding).context("Failed to serialize channel binding")?;
        fs::write(&path, content)
            .await
            .context("Failed to write channel-binding.yaml")?;
        Ok(())
    }

    /// Derive initial runtime state for a new agent.
    async fn derive_initial_state(&self) -> AgentState {
        AgentState {
            context_window: Some(20),
            event_cursor: Some(0),
            last_reset_at: None,
            last_summary_at: None,
        }
    }

    /// Read agent schema from agent.yaml
    pub async fn read_agent_schema(&self) -> Result<AgentSchema> {
        let path = self.schema_dir().join("agent.yaml");
        if !path.exists() {
            return Ok(AgentSchema::default());
        }
        let content = fs::read_to_string(&path)
            .await
            .context("Failed to read agent.yaml")?;
        let schema: AgentSchema =
            serde_yaml::from_str(&content).context("Failed to parse agent.yaml")?;
        debug!("Read agent schema from {:?}", path);
        Ok(schema)
    }

    /// Write agent schema to agent.yaml
    pub async fn write_agent_schema(&self, schema: &AgentSchema) -> Result<()> {
        let path = self.schema_dir().join("agent.yaml");
        let content = serde_yaml::to_string(schema).context("Failed to serialize agent schema")?;
        fs::write(&path, content)
            .await
            .context("Failed to write agent.yaml")?;
        debug!("Wrote agent schema to {:?}", path);
        Ok(())
    }

    /// Read capabilities from capabilities.yaml
    pub async fn read_capabilities(&self) -> Result<AgentCapabilities> {
        let path = self.schema_dir().join("capabilities.yaml");
        if !path.exists() {
            return Ok(AgentCapabilities::default());
        }
        let content = fs::read_to_string(&path)
            .await
            .context("Failed to read capabilities.yaml")?;
        let caps: AgentCapabilities =
            serde_yaml::from_str(&content).context("Failed to parse capabilities.yaml")?;
        Ok(caps)
    }

    /// Write capabilities to capabilities.yaml
    pub async fn write_capabilities(&self, caps: &AgentCapabilities) -> Result<()> {
        let path = self.schema_dir().join("capabilities.yaml");
        let content = serde_yaml::to_string(caps).context("Failed to serialize capabilities")?;
        fs::write(&path, content)
            .await
            .context("Failed to write capabilities.yaml")?;
        Ok(())
    }

    /// Read state from state.yaml
    pub async fn read_state(&self) -> Result<AgentState> {
        let path = self.schema_dir().join("state.yaml");
        if !path.exists() {
            return Ok(AgentState::default());
        }
        let content = fs::read_to_string(&path)
            .await
            .context("Failed to read state.yaml")?;
        let state: AgentState =
            serde_yaml::from_str(&content).context("Failed to parse state.yaml")?;
        Ok(state)
    }

    /// Write state to state.yaml
    pub async fn write_state(&self, state: &AgentState) -> Result<()> {
        let path = self.schema_dir().join("state.yaml");
        let content = serde_yaml::to_string(state).context("Failed to serialize state")?;
        fs::write(&path, content)
            .await
            .context("Failed to write state.yaml")?;
        Ok(())
    }
}
