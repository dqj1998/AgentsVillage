use std::sync::Arc;

use anyhow::Result;
use tokio::sync::Mutex;
use tracing::info;

use crate::agent::AgentManager;

use super::{AppRequest, AppResponse, ExecutionEngine, IntentCompiler};

/// Application service — the main entry point for all platform-agnostic request handling
pub struct AppService {
    pub agent_manager: Arc<Mutex<AgentManager>>,
    pub compiler: IntentCompiler,
    pub engine: ExecutionEngine,
}

impl AppService {
    pub fn new(agent_manager: Arc<Mutex<AgentManager>>, engine: ExecutionEngine) -> Self {
        Self {
            agent_manager,
            compiler: IntentCompiler::new(),
            engine,
        }
    }

    pub async fn handle(&self, request: AppRequest) -> Result<AppResponse> {
        // Compile intent
        let intent = self.compiler.compile(&request);
        info!(
            "AppService: compiled intent {:?} for agent {} at {}",
            intent, request.agent_id, request.timestamp
        );

        // Get or create agent
        let agent = {
            let mut manager = self.agent_manager.lock().await;
            manager
                .get_or_create_agent(&request.agent_id, &request.platform_user)
                .await?
                .clone()
        };
        info!(
            "AppService: resolved agent manifest v{} bound to {:?} channel {}",
            agent.manifest.manifest_version,
            agent.channel_binding.channel.kind,
            agent.channel_binding.channel.external_id
        );

        // Execute intent
        let response = self.engine.execute(intent, &agent).await?;
        Ok(response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use tempfile::tempdir;
    use tokio::sync::Mutex;

    use crate::agent::schema_store::SchemaStore;
    use crate::agent::AgentManager;
    use crate::app::schema::{AgentCapabilities, AuditEvent};
    use crate::app::{executor::ExecutionEngine, AppRequest, AppResponse, RequestPayload};
    use crate::config::GlobalConfig;
    use crate::llm::{ChatMessage, LlmBackend};

    /// Mock LLM backend that returns a fixed response without any HTTP calls.
    struct MockLlmBackend {
        response: String,
    }

    impl MockLlmBackend {
        fn new(response: impl Into<String>) -> Self {
            Self {
                response: response.into(),
            }
        }
    }

    #[async_trait::async_trait]
    impl LlmBackend for MockLlmBackend {
        async fn chat(&self, _messages: Vec<ChatMessage>) -> anyhow::Result<String> {
            Ok(self.response.clone())
        }
    }

    /// Failing mock — always returns an error.
    struct FailingLlmBackend;

    #[async_trait::async_trait]
    impl LlmBackend for FailingLlmBackend {
        async fn chat(&self, _messages: Vec<ChatMessage>) -> anyhow::Result<String> {
            anyhow::bail!("mock LLM error: connection refused")
        }
    }

    struct CountingLlmBackend {
        response: String,
        calls: Arc<AtomicUsize>,
    }

    impl CountingLlmBackend {
        fn new(response: impl Into<String>, calls: Arc<AtomicUsize>) -> Self {
            Self {
                response: response.into(),
                calls,
            }
        }
    }

    #[async_trait::async_trait]
    impl LlmBackend for CountingLlmBackend {
        async fn chat(&self, _messages: Vec<ChatMessage>) -> anyhow::Result<String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(self.response.clone())
        }
    }

    fn make_app_service(
        mock: Arc<dyn LlmBackend>,
        workspace_root: std::path::PathBuf,
    ) -> AppService {
        let global_config = GlobalConfig::default();
        let manager = AgentManager::new_with_workspace(global_config, workspace_root);
        let agent_manager = Arc::new(Mutex::new(manager));
        let engine = ExecutionEngine::new(mock, 20);
        AppService::new(agent_manager, engine)
    }

    fn make_chat_request(agent_id: &str, user: &str, text: &str) -> AppRequest {
        AppRequest {
            agent_id: agent_id.to_string(),
            platform_user: user.to_string(),
            timestamp: "2024-01-01 00:00:00 UTC".to_string(),
            payload: RequestPayload::Message(text.to_string()),
        }
    }

    fn make_reset_request(agent_id: &str, user: &str) -> AppRequest {
        AppRequest {
            agent_id: agent_id.to_string(),
            platform_user: user.to_string(),
            timestamp: "2024-01-01 00:00:00 UTC".to_string(),
            payload: RequestPayload::Command {
                name: "new".to_string(),
                args: vec![],
            },
        }
    }

    async fn write_capabilities(
        workspace_root: &std::path::Path,
        agent_id: &str,
        capabilities: &AgentCapabilities,
    ) {
        let workspace_path = workspace_root.join(agent_id);
        tokio::fs::create_dir_all(&workspace_path).await.unwrap();
        let schema_store = SchemaStore::new(workspace_path);
        schema_store
            .initialize(agent_id, "test-channel")
            .await
            .unwrap();
        schema_store.write_capabilities(capabilities).await.unwrap();
    }

    // ── Integration tests ────────────────────────────────────────────────────

    #[tokio::test]
    async fn app_service_chat_returns_text_response() {
        let dir = tempdir().unwrap();
        let mock = Arc::new(MockLlmBackend::new("Hello from mock LLM!")) as Arc<dyn LlmBackend>;
        let svc = make_app_service(mock, dir.path().to_path_buf());

        let req = make_chat_request("discord-1-2", "alice", "Hi there");
        let response = svc.handle(req).await.unwrap();

        assert!(
            matches!(response, AppResponse::Text(ref t) if t == "Hello from mock LLM!"),
            "expected Text(\"Hello from mock LLM!\"), got {:?}",
            response
        );
    }

    #[tokio::test]
    async fn app_service_reset_session_returns_ephemeral() {
        let dir = tempdir().unwrap();
        let mock = Arc::new(MockLlmBackend::new("irrelevant")) as Arc<dyn LlmBackend>;
        let svc = make_app_service(mock, dir.path().to_path_buf());

        // First create the agent by sending a chat
        let chat_req = make_chat_request("discord-1-2", "alice", "Hello");
        let _ = svc.handle(chat_req).await.unwrap();

        // Now reset
        let reset_req = make_reset_request("discord-1-2", "alice");
        let response = svc.handle(reset_req).await.unwrap();

        assert!(
            matches!(response, AppResponse::Ephemeral(_)),
            "expected Ephemeral response for /new, got {:?}",
            response
        );
        if let AppResponse::Ephemeral(msg) = response {
            assert!(
                msg.contains("Session cleared") || msg.contains("fresh"),
                "expected session-cleared message, got: {}",
                msg
            );
        }
    }

    #[tokio::test]
    async fn app_service_llm_failure_returns_error_response() {
        let dir = tempdir().unwrap();
        let mock = Arc::new(FailingLlmBackend) as Arc<dyn LlmBackend>;
        let svc = make_app_service(mock, dir.path().to_path_buf());

        let req = make_chat_request("discord-1-2", "alice", "Will this fail?");
        let response = svc.handle(req).await.unwrap();

        assert!(
            matches!(response, AppResponse::Error(_)),
            "expected Error response when LLM fails, got {:?}",
            response
        );
        if let AppResponse::Error(msg) = response {
            assert!(
                msg.contains("error"),
                "error message should mention error: {}",
                msg
            );
        }
    }

    #[tokio::test]
    async fn app_service_session_persists_across_requests() {
        let dir = tempdir().unwrap();
        let mock = Arc::new(MockLlmBackend::new("pong")) as Arc<dyn LlmBackend>;
        let svc = make_app_service(mock, dir.path().to_path_buf());

        // Send two messages
        let req1 = make_chat_request("discord-1-2", "alice", "ping");
        let _ = svc.handle(req1).await.unwrap();

        let req2 = make_chat_request("discord-1-2", "alice", "ping again");
        let response = svc.handle(req2).await.unwrap();

        // Both should succeed — session file should have entries
        assert!(matches!(response, AppResponse::Text(_)));

        // Verify session file was written
        let session_dir = dir.path().join("discord-1-2").join("sessions");
        assert!(session_dir.exists(), "sessions directory should be created");
        let mut entries = tokio::fs::read_dir(&session_dir).await.unwrap();
        let has_session = entries.next_entry().await.unwrap().is_some();
        assert!(has_session, "at least one session file should exist");
    }

    #[tokio::test]
    async fn app_service_event_log_written_after_chat() {
        let dir = tempdir().unwrap();
        let mock = Arc::new(MockLlmBackend::new("event test response")) as Arc<dyn LlmBackend>;
        let svc = make_app_service(mock, dir.path().to_path_buf());

        let req = make_chat_request("discord-1-2", "alice", "test event logging");
        let _ = svc.handle(req).await.unwrap();

        // Verify event log was written
        let event_log_path = dir
            .path()
            .join("discord-1-2")
            .join("events")
            .join("events.jsonl");
        assert!(
            event_log_path.exists(),
            "events.jsonl should be created after chat"
        );

        let content = tokio::fs::read_to_string(&event_log_path).await.unwrap();
        assert!(
            content.contains("chat_started"),
            "event log should contain chat_started"
        );
        assert!(
            content.contains("chat_completed"),
            "event log should contain chat_completed"
        );
    }

    #[tokio::test]
    async fn app_service_reset_event_log_written() {
        let dir = tempdir().unwrap();
        let mock = Arc::new(MockLlmBackend::new("hi")) as Arc<dyn LlmBackend>;
        let svc = make_app_service(mock, dir.path().to_path_buf());

        // Create agent first
        let chat_req = make_chat_request("discord-1-2", "alice", "hello");
        let _ = svc.handle(chat_req).await.unwrap();

        // Reset
        let reset_req = make_reset_request("discord-1-2", "alice");
        let _ = svc.handle(reset_req).await.unwrap();

        let event_log_path = dir
            .path()
            .join("discord-1-2")
            .join("events")
            .join("events.jsonl");
        let content = tokio::fs::read_to_string(&event_log_path).await.unwrap();
        assert!(
            content.contains("reset_session"),
            "event log should contain reset_session after /new"
        );
    }

    #[tokio::test]
    async fn app_service_unknown_command_returns_ephemeral() {
        let dir = tempdir().unwrap();
        let mock = Arc::new(MockLlmBackend::new("irrelevant")) as Arc<dyn LlmBackend>;
        let svc = make_app_service(mock, dir.path().to_path_buf());

        let req = AppRequest {
            agent_id: "discord-1-2".to_string(),
            platform_user: "alice".to_string(),
            timestamp: "2024-01-01 00:00:00 UTC".to_string(),
            payload: RequestPayload::Command {
                name: "unknown_cmd".to_string(),
                args: vec![],
            },
        };

        let response = svc.handle(req).await.unwrap();
        assert!(
            matches!(response, AppResponse::Ephemeral(_)),
            "unknown command should return Ephemeral, got {:?}",
            response
        );
    }

    #[tokio::test]
    async fn schema_policy_disables_chat_before_llm_call() {
        let dir = tempdir().unwrap();
        let agent_id = "discord-1-2";
        let mut capabilities = AgentCapabilities::initial();
        capabilities.intent_policy.allow_chat = false;
        write_capabilities(dir.path(), agent_id, &capabilities).await;

        let calls = Arc::new(AtomicUsize::new(0));
        let mock = Arc::new(CountingLlmBackend::new(
            "should not be used",
            Arc::clone(&calls),
        )) as Arc<dyn LlmBackend>;
        let svc = make_app_service(mock, dir.path().to_path_buf());

        let req = make_chat_request(agent_id, "alice", "hello");
        let response = svc.handle(req).await.unwrap();

        assert!(
            matches!(response, AppResponse::Ephemeral(ref msg) if msg.contains("Chat is disabled")),
            "expected chat-disabled response, got {:?}",
            response
        );
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "disabled chat must not call LLM"
        );
    }

    #[tokio::test]
    async fn schema_policy_disables_reset_without_clearing_session() {
        let dir = tempdir().unwrap();
        let agent_id = "discord-1-2";
        let mut capabilities = AgentCapabilities::initial();
        capabilities.intent_policy.allow_reset_session = false;
        write_capabilities(dir.path(), agent_id, &capabilities).await;

        let mock = Arc::new(MockLlmBackend::new("assistant reply")) as Arc<dyn LlmBackend>;
        let svc = make_app_service(mock, dir.path().to_path_buf());

        let chat_req = make_chat_request(agent_id, "alice", "keep this session");
        let _ = svc.handle(chat_req).await.unwrap();

        let session_dir = dir.path().join(agent_id).join("sessions");
        let mut entries = tokio::fs::read_dir(&session_dir).await.unwrap();
        let session_file = entries
            .next_entry()
            .await
            .unwrap()
            .expect("session file should exist")
            .path();
        let before_reset = tokio::fs::read_to_string(&session_file).await.unwrap();
        assert!(before_reset.contains("keep this session"));

        let reset_req = make_reset_request(agent_id, "alice");
        let response = svc.handle(reset_req).await.unwrap();
        assert!(
            matches!(response, AppResponse::Ephemeral(ref msg) if msg.contains("Reset session is disabled")),
            "expected reset-disabled response, got {:?}",
            response
        );

        let after_reset = tokio::fs::read_to_string(&session_file).await.unwrap();
        assert_eq!(
            before_reset, after_reset,
            "disabled reset must not clear the session"
        );

        let event_log_path = dir
            .path()
            .join(agent_id)
            .join("events")
            .join("events.jsonl");
        let event_log = tokio::fs::read_to_string(&event_log_path).await.unwrap();
        assert!(
            !event_log.contains("reset_session"),
            "disabled reset must not write reset_session event"
        );
    }

    #[tokio::test]
    async fn schema_policy_disables_summarization_even_over_context_window() {
        let dir = tempdir().unwrap();
        let agent_id = "discord-1-2";
        let mut capabilities = AgentCapabilities::initial();
        capabilities.memory_policy.context_window = Some(1);
        capabilities.memory_policy.summarize_old_messages = false;
        write_capabilities(dir.path(), agent_id, &capabilities).await;

        let mock = Arc::new(MockLlmBackend::new("assistant reply")) as Arc<dyn LlmBackend>;
        let svc = make_app_service(mock, dir.path().to_path_buf());

        let req1 = make_chat_request(agent_id, "alice", "first message");
        let _ = svc.handle(req1).await.unwrap();
        let req2 = make_chat_request(agent_id, "alice", "second message");
        let _ = svc.handle(req2).await.unwrap();

        let event_log_path = dir
            .path()
            .join(agent_id)
            .join("events")
            .join("events.jsonl");
        let event_log = tokio::fs::read_to_string(&event_log_path).await.unwrap();
        assert!(
            !event_log.contains("memory_summarized"),
            "summarization-disabled policy must suppress memory_summarized events"
        );

        let memory_path = dir.path().join(agent_id).join("memory.md");
        let memory = tokio::fs::read_to_string(&memory_path).await.unwrap();
        assert!(
            !memory.contains("## Summary"),
            "summarization-disabled policy must not append memory summaries"
        );
    }

    #[tokio::test]
    async fn required_audit_policy_is_enforced_before_execution() {
        let dir = tempdir().unwrap();
        let agent_id = "discord-1-2";
        let mut capabilities = AgentCapabilities::initial();
        capabilities.audit_policy.emit_events = vec![AuditEvent::ChatStarted];
        write_capabilities(dir.path(), agent_id, &capabilities).await;

        let calls = Arc::new(AtomicUsize::new(0));
        let mock = Arc::new(CountingLlmBackend::new(
            "should not be used",
            Arc::clone(&calls),
        )) as Arc<dyn LlmBackend>;
        let svc = make_app_service(mock, dir.path().to_path_buf());

        let req = make_chat_request(agent_id, "alice", "audit this");
        let response = svc.handle(req).await.unwrap();

        assert!(
            matches!(response, AppResponse::Error(ref msg) if msg.contains("requires audit event chat_completed")),
            "expected schema validation error, got {:?}",
            response
        );
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "schema validation failure must not call LLM"
        );

        let event_log_path = dir
            .path()
            .join(agent_id)
            .join("events")
            .join("events.jsonl");
        assert!(
            !event_log_path.exists(),
            "schema validation failure must not write event log entries"
        );
    }
}
