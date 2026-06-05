use std::sync::Arc;

use anyhow::Result;
use tracing::{info, warn};

use crate::agent::event_log::AgentEvent;
use crate::agent::{Agent, EventLog, MemoryManager};
use crate::app::renderer::SchemaRenderer;
use crate::app::schema::{AuditEvent as SchemaAuditEvent, MemoryPolicy, SystemSchemaCatalog};
use crate::discord::adapter::current_timestamp;
use crate::llm::{ChatMessage, LlmBackend};

use super::{AppResponse, Intent};

/// Trait for intent executors
#[async_trait::async_trait]
pub trait IntentExecutor: Send + Sync {
    async fn execute(&self, intent: Intent, agent: &Agent) -> Result<AppResponse>;
}

/// Execution engine that dispatches intents to executors
pub struct ExecutionEngine {
    pub context_window: usize,
    pub llm_backend: Arc<dyn LlmBackend>,
    system_schema: SystemSchemaCatalog,
}

impl ExecutionEngine {
    pub fn new(llm_backend: Arc<dyn LlmBackend>, context_window: usize) -> Self {
        Self {
            context_window,
            llm_backend,
            system_schema: SystemSchemaCatalog::from_embedded(),
        }
    }

    pub async fn execute(&self, intent: Intent, agent: &Agent) -> Result<AppResponse> {
        if let Some(intent_name) = schema_intent_name(&intent) {
            if let Err(error) = self.system_schema.validate_intent_execution(
                intent_name,
                ChatExecutor::SCHEMA_NAME,
                &agent.capabilities.audit_policy,
            ) {
                warn!(
                    "Schema validation failed for intent {} on agent {}: {}",
                    intent_name, agent.id, error
                );
                return Ok(AppResponse::Error(format!(
                    "Schema validation failed: {}",
                    error
                )));
            }
        }

        // Per-agent schema policy takes precedence over state, agent config, and engine defaults.
        let context_window = agent
            .capabilities
            .memory_policy
            .context_window
            .or(agent.state.context_window)
            .unwrap_or(
                agent
                    .config
                    .core
                    .context_window
                    .unwrap_or(self.context_window),
            );
        let mut memory_policy = agent.capabilities.memory_policy.clone();
        memory_policy.context_window = Some(context_window);
        let executor = ChatExecutor {
            memory_policy,
            llm_backend: Arc::clone(&self.llm_backend),
        };
        executor.execute(intent, agent).await
    }
}

/// Chat executor for the app_service pipeline.
pub struct ChatExecutor {
    pub memory_policy: MemoryPolicy,
    pub llm_backend: Arc<dyn LlmBackend>,
}

impl ChatExecutor {
    pub const SCHEMA_NAME: &'static str = "chat_executor";
}

#[async_trait::async_trait]
impl IntentExecutor for ChatExecutor {
    async fn execute(&self, intent: Intent, agent: &Agent) -> Result<AppResponse> {
        let memory_manager = MemoryManager::new(agent.workspace_path.clone());
        let event_log = EventLog::new(agent.workspace_path.clone());

        match intent {
            Intent::Chat { user_text, author } => {
                if agent.capabilities.safety_policy.deny_disabled_intents
                    && (!agent.capabilities.chat || !agent.capabilities.intent_policy.allow_chat)
                {
                    return Ok(AppResponse::Ephemeral(
                        "Chat is disabled for this agent.".to_string(),
                    ));
                }

                let timestamp = current_timestamp();

                // Append user message to session
                if let Err(e) = memory_manager
                    .append_session(&author, &user_text, &timestamp)
                    .await
                {
                    warn!("Failed to append user message to session: {}", e);
                }

                append_audit_event(
                    agent,
                    &event_log,
                    SchemaAuditEvent::ChatStarted,
                    AgentEvent::chat_started(&author, user_text.len()),
                    "ChatStarted",
                )
                .await;
                info!(
                    "ChatExecutor: ChatStarted for agent {} ({})",
                    agent.id, agent.display_name
                );

                // Build LLM messages with explicit summarization side-effect
                let llm_messages = build_llm_messages_with_summary(
                    agent,
                    &memory_manager,
                    self.llm_backend.as_ref(),
                    &self.memory_policy,
                    &event_log,
                )
                .await?;

                // Call LLM
                match self.llm_backend.chat(llm_messages).await {
                    Ok(response_text) => {
                        // Append assistant response to session
                        let resp_timestamp = current_timestamp();
                        if let Err(e) = memory_manager
                            .append_session("assistant", &response_text, &resp_timestamp)
                            .await
                        {
                            warn!("Failed to append assistant response to session: {}", e);
                        }

                        append_audit_event(
                            agent,
                            &event_log,
                            SchemaAuditEvent::ChatCompleted,
                            AgentEvent::chat_completed(response_text.len()),
                            "ChatCompleted",
                        )
                        .await;
                        info!(
                            "ChatExecutor: ChatCompleted for agent {} ({})",
                            agent.id, agent.display_name
                        );

                        Ok(AppResponse::Text(response_text))
                    }
                    Err(e) => {
                        let error_msg = e.to_string();

                        append_audit_event(
                            agent,
                            &event_log,
                            SchemaAuditEvent::ChatFailed,
                            AgentEvent::chat_failed(&error_msg),
                            "ChatFailed",
                        )
                        .await;
                        info!(
                            "ChatExecutor: ChatFailed for agent {} ({}): {}",
                            agent.id, agent.display_name, error_msg
                        );

                        Ok(AppResponse::Error(format!(
                            "Sorry, I encountered an error: {}",
                            error_msg
                        )))
                    }
                }
            }

            Intent::ResetSession => {
                if agent.capabilities.safety_policy.deny_disabled_intents
                    && (!agent.capabilities.reset_session
                        || !agent.capabilities.intent_policy.allow_reset_session)
                {
                    return Ok(AppResponse::Ephemeral(
                        "Reset session is disabled for this agent.".to_string(),
                    ));
                }

                memory_manager.clear_today_session().await?;

                append_audit_event(
                    agent,
                    &event_log,
                    SchemaAuditEvent::ResetSession,
                    AgentEvent::reset_session("user requested /new"),
                    "ResetSession",
                )
                .await;

                info!(
                    "ChatExecutor: session reset for agent {} ({})",
                    agent.id, agent.display_name
                );
                Ok(AppResponse::Ephemeral(
                    "✅ Session cleared. Starting fresh!".to_string(),
                ))
            }

            Intent::Command { name, .. } => Ok(AppResponse::Ephemeral(format!(
                "Unknown command: /{}",
                name
            ))),

            Intent::Clarify { question } => Ok(AppResponse::Text(question)),
        }
    }
}

/// Build LLM messages from agent context.
/// Side effect: if total_messages > context_window, calls LLM to summarize old messages
/// and writes the summary to memory.md. This side effect is now explicit.
async fn build_llm_messages_with_summary(
    agent: &Agent,
    memory_manager: &MemoryManager,
    llm_backend: &dyn LlmBackend,
    memory_policy: &MemoryPolicy,
    event_log: &EventLog,
) -> Result<Vec<ChatMessage>> {
    let context_window = memory_policy.context_window.unwrap_or(20);

    // Load recent messages
    let recent_messages = memory_manager.load_recent_messages(context_window).await?;

    // Count total messages (today's session only — see count_total_messages doc)
    let total_messages = memory_manager.count_total_messages().await?;

    info!(
        "ChatExecutor: checking if summarization needed (total={}, window={})",
        total_messages, context_window
    );

    // If total > context_window, summarize the oldest messages beyond the window
    if agent.capabilities.summarize
        && memory_policy.summarize_old_messages
        && total_messages > context_window
    {
        let all_messages = memory_manager.load_recent_messages(total_messages).await?;
        let old_count = total_messages.saturating_sub(context_window);
        if old_count > 0 {
            let old_messages = &all_messages[..old_count.min(all_messages.len())];
            if !old_messages.is_empty() {
                info!(
                    "ChatExecutor: summarizing {} old messages",
                    old_messages.len()
                );

                let old_text: String = old_messages
                    .iter()
                    .map(|m| format!("{}: {}", m.author, m.content))
                    .collect::<Vec<_>>()
                    .join("\n");

                let summarization_prompt = format!(
                    "Summarize the following conversation history concisely for long-term memory. \
                     Focus on key facts, decisions, and context that would be useful for future conversations:\n\n{}",
                    old_text
                );

                let summary_messages = vec![ChatMessage {
                    role: "user".to_string(),
                    content: summarization_prompt,
                }];

                match llm_backend.chat(summary_messages).await {
                    Ok(summary) => {
                        info!("Generated memory summary for agent {}", agent.id);
                        if memory_policy.write_summary_to_memory {
                            memory_manager.append_memory(&summary).await?;
                        }

                        append_audit_event(
                            agent,
                            event_log,
                            SchemaAuditEvent::MemorySummarized,
                            AgentEvent::memory_summarized(old_messages.len()),
                            "MemorySummarized",
                        )
                        .await;
                    }
                    Err(e) => {
                        warn!("Failed to generate memory summary: {}", e);
                    }
                }
            }
        }
    }

    // Build system message via SchemaRenderer (single assembly point)
    let renderer = SchemaRenderer::new();
    let system_content = renderer.render_system_prompt(agent, memory_manager).await?;

    let mut messages = vec![ChatMessage {
        role: "system".to_string(),
        content: system_content,
    }];

    // Add recent messages as user/assistant turns
    for session_msg in &recent_messages {
        messages.push(ChatMessage {
            role: session_msg.role.clone(),
            content: session_msg.content.clone(),
        });
    }

    Ok(messages)
}

async fn append_audit_event(
    agent: &Agent,
    event_log: &EventLog,
    audit_event: SchemaAuditEvent,
    event: AgentEvent,
    label: &str,
) {
    let schema_event_name = audit_event.schema_name();

    if !agent.capabilities.audit_policy.emits(&audit_event) {
        info!(
            "Audit event {} suppressed by schema for agent {} ({})",
            schema_event_name, agent.id, agent.display_name
        );
        return;
    }

    if let Err(e) = event_log.append(event).await {
        warn!("Failed to write {} event: {}", label, e);
    }
}

fn schema_intent_name(intent: &Intent) -> Option<&'static str> {
    match intent {
        Intent::Chat { .. } => Some("chat"),
        Intent::ResetSession => Some("reset_session"),
        Intent::Command { .. } | Intent::Clarify { .. } => None,
    }
}
