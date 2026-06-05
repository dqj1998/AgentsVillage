pub mod compiler;
pub mod executor;
pub mod renderer;
pub mod schema;
pub mod service;

pub use compiler::IntentCompiler;
pub use executor::ExecutionEngine;

use serde::{Deserialize, Serialize};

/// Platform-agnostic inbound request
#[derive(Debug, Clone)]
pub struct AppRequest {
    pub agent_id: String,
    pub platform_user: String, // e.g. Discord username
    pub timestamp: String,
    pub payload: RequestPayload,
}

#[derive(Debug, Clone)]
pub enum RequestPayload {
    Message(String),
    Command { name: String, args: Vec<String> },
}

/// Compiled intent — what the app layer will execute
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Intent {
    Chat { user_text: String, author: String },
    ResetSession,
    Command { name: String, args: Vec<String> },
    Clarify { question: String },
}

/// Platform-agnostic outbound response
#[derive(Debug, Clone)]
pub enum AppResponse {
    Text(String),
    Ephemeral(String),
    Error(String),
}
