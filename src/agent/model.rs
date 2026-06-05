use std::path::PathBuf;

use crate::app::schema::{AgentCapabilities, AgentManifest, AgentState, ChannelBinding};
use crate::config::GlobalConfig;

#[derive(Debug, Clone)]
pub struct Agent {
    pub id: String,
    pub display_name: String,
    pub workspace_path: PathBuf,
    pub role: String,
    pub config: GlobalConfig,
    pub manifest: AgentManifest,
    pub channel_binding: ChannelBinding,
    pub capabilities: AgentCapabilities,
    pub state: AgentState,
}
