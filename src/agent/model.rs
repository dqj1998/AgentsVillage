use std::path::PathBuf;

use crate::config::GlobalConfig;

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct Agent {
    pub id: String,
    pub display_name: String,
    pub workspace_path: PathBuf,
    pub role: String,
    pub config: GlobalConfig,
}
