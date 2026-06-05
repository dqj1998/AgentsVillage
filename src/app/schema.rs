use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Per-agent schema loaded from workspace/{agent_id}/schema/agent.yaml
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AgentSchema {
    pub identity: AgentIdentity,
    pub schema_version: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AgentIdentity {
    pub id: String,
    pub display_name: String,
    pub schema_version: Option<u32>,
}

/// Workspace-level manifest for one runtime agent instance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentManifest {
    pub manifest_version: u32,
    pub agent_id: String,
    pub display_name: String,
    pub schema_refs: SchemaRefs,
    pub workspace_layout: WorkspaceLayout,
}

impl AgentManifest {
    pub fn initial(agent_id: &str, display_name: &str) -> Self {
        Self {
            manifest_version: 1,
            agent_id: agent_id.to_string(),
            display_name: display_name.to_string(),
            schema_refs: SchemaRefs::default(),
            workspace_layout: WorkspaceLayout::default(),
        }
    }
}

impl Default for AgentManifest {
    fn default() -> Self {
        Self::initial("", "")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchemaRefs {
    #[serde(default = "default_system_schema_ref")]
    pub system: String,
}

impl Default for SchemaRefs {
    fn default() -> Self {
        Self {
            system: default_system_schema_ref(),
        }
    }
}

fn default_system_schema_ref() -> String {
    "schemas/system.yaml".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceLayout {
    pub role: String,
    pub memory: String,
    pub sessions: String,
    pub events: String,
    pub legacy_instance_schema: String,
}

impl Default for WorkspaceLayout {
    fn default() -> Self {
        Self {
            role: "role.md".to_string(),
            memory: "memory.md".to_string(),
            sessions: "sessions/".to_string(),
            events: "events/events.jsonl".to_string(),
            legacy_instance_schema: "schema/".to_string(),
        }
    }
}

/// Workspace-level binding from an external channel location to one agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelBinding {
    pub binding_version: u32,
    pub agent_id: String,
    pub workspace: String,
    pub channel: ChannelRef,
    pub routing_policy: RoutingPolicy,
    pub delivery_policy: DeliveryPolicy,
}

impl ChannelBinding {
    pub fn initial(agent_id: &str, display_name: &str) -> Self {
        Self {
            binding_version: 1,
            agent_id: agent_id.to_string(),
            workspace: format!("workspace/{}", agent_id),
            channel: ChannelRef::from_agent_id(agent_id, display_name),
            routing_policy: RoutingPolicy::default(),
            delivery_policy: DeliveryPolicy::default(),
        }
    }
}

impl Default for ChannelBinding {
    fn default() -> Self {
        Self::initial("", "")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelRef {
    pub kind: ChannelKind,
    pub external_id: String,
    pub display_name: String,
    pub fields: BTreeMap<String, String>,
}

impl ChannelRef {
    pub fn from_agent_id(agent_id: &str, display_name: &str) -> Self {
        let mut fields = BTreeMap::new();
        let parts: Vec<&str> = agent_id.split('-').collect();
        let kind = if parts.first() == Some(&"discord") && parts.len() >= 3 {
            fields.insert("guild_id".to_string(), parts[1].to_string());
            fields.insert("channel_id".to_string(), parts[2].to_string());
            if let Some(thread_id) = parts.get(3) {
                fields.insert("thread_id".to_string(), (*thread_id).to_string());
            }
            ChannelKind::Discord
        } else {
            fields.insert("source_agent_id".to_string(), agent_id.to_string());
            ChannelKind::Unknown
        };

        Self {
            kind,
            external_id: agent_id.to_string(),
            display_name: display_name.to_string(),
            fields,
        }
    }
}

impl Default for ChannelRef {
    fn default() -> Self {
        Self::from_agent_id("", "")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChannelKind {
    Discord,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingPolicy {
    pub create_agent_on_first_message: bool,
    pub route_mentions_only: bool,
    pub allow_thread_agents: bool,
}

impl Default for RoutingPolicy {
    fn default() -> Self {
        Self {
            create_agent_on_first_message: true,
            route_mentions_only: false,
            allow_thread_agents: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeliveryPolicy {
    pub split_long_messages: bool,
    pub max_message_length: usize,
    pub supports_ephemeral: bool,
    pub typing_indicator: bool,
}

impl Default for DeliveryPolicy {
    fn default() -> Self {
        Self {
            split_long_messages: true,
            max_message_length: 2000,
            supports_ephemeral: true,
            typing_indicator: true,
        }
    }
}

/// Capability flags loaded from workspace/{agent_id}/schema/capabilities.yaml
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentCapabilities {
    #[serde(default)]
    pub intent_policy: IntentPolicy,
    #[serde(default)]
    pub memory_policy: MemoryPolicy,
    #[serde(default)]
    pub audit_policy: AuditPolicy,
    #[serde(default)]
    pub safety_policy: SafetyPolicy,

    // Compatibility fields for older capabilities.yaml files.
    #[serde(default = "default_true")]
    pub chat: bool,
    #[serde(default = "default_true")]
    pub reset_session: bool,
    #[serde(default = "default_true")]
    pub summarize: bool,
    #[serde(default)]
    pub commands: bool,
}

impl Default for AgentCapabilities {
    fn default() -> Self {
        Self {
            intent_policy: IntentPolicy::default(),
            memory_policy: MemoryPolicy::default(),
            audit_policy: AuditPolicy::default(),
            safety_policy: SafetyPolicy::default(),
            chat: true,
            reset_session: true,
            summarize: true,
            commands: false,
        }
    }
}

impl AgentCapabilities {
    /// Defaults written for newly initialized agents.
    pub fn initial() -> Self {
        Self {
            memory_policy: MemoryPolicy {
                context_window: Some(20),
                ..MemoryPolicy::default()
            },
            ..Self::default()
        }
    }
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntentPolicy {
    pub allow_chat: bool,
    pub allow_reset_session: bool,
    pub command_mode: CommandMode,
}

impl Default for IntentPolicy {
    fn default() -> Self {
        Self {
            allow_chat: true,
            allow_reset_session: true,
            command_mode: CommandMode::RejectUnknown,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CommandMode {
    RejectUnknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryPolicy {
    pub context_window: Option<usize>,
    pub summarize_old_messages: bool,
    pub write_summary_to_memory: bool,
}

impl Default for MemoryPolicy {
    fn default() -> Self {
        Self {
            context_window: None,
            summarize_old_messages: true,
            write_summary_to_memory: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditPolicy {
    pub emit_events: Vec<AuditEvent>,
    pub persist_session_transcript: bool,
}

impl Default for AuditPolicy {
    fn default() -> Self {
        Self {
            emit_events: vec![
                AuditEvent::ChatStarted,
                AuditEvent::ChatCompleted,
                AuditEvent::ChatFailed,
                AuditEvent::ResetSession,
                AuditEvent::MemorySummarized,
            ],
            persist_session_transcript: true,
        }
    }
}

impl AuditPolicy {
    pub fn emits(&self, event: &AuditEvent) -> bool {
        self.emit_events.contains(event)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AuditEvent {
    ChatStarted,
    ChatCompleted,
    ChatFailed,
    ResetSession,
    MemorySummarized,
}

impl AuditEvent {
    pub fn schema_name(&self) -> &'static str {
        match self {
            Self::ChatStarted => "chat_started",
            Self::ChatCompleted => "chat_completed",
            Self::ChatFailed => "chat_failed",
            Self::ResetSession => "reset_session",
            Self::MemorySummarized => "memory_summarized",
        }
    }

    pub fn from_schema_name(name: &str) -> Option<Self> {
        match name {
            "chat_started" => Some(Self::ChatStarted),
            "chat_completed" => Some(Self::ChatCompleted),
            "chat_failed" => Some(Self::ChatFailed),
            "reset_session" => Some(Self::ResetSession),
            "memory_summarized" => Some(Self::MemorySummarized),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SafetyPolicy {
    pub deny_disabled_intents: bool,
    pub require_explicit_user_command_for_reset: bool,
}

impl Default for SafetyPolicy {
    fn default() -> Self {
        Self {
            deny_disabled_intents: true,
            require_explicit_user_command_for_reset: true,
        }
    }
}

/// Runtime state loaded from workspace/{agent_id}/schema/state.yaml
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AgentState {
    pub context_window: Option<usize>,
    pub event_cursor: Option<u64>,
    pub last_reset_at: Option<String>,
    pub last_summary_at: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformSchema {
    pub schema_version: u32,
    pub platform: String,
    pub pipeline: Vec<String>,
    pub schema_layers: Vec<String>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntentCatalog {
    pub schema_version: u32,
    pub intents: Vec<IntentDefinition>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntentDefinition {
    pub name: String,
    pub enabled_by_default: bool,
    pub trigger: String,
    pub input_fields: Vec<String>,
    pub execution: IntentExecutionDefinition,
    pub failure: IntentFailureDefinition,
    pub audit: IntentAuditDefinition,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntentExecutionDefinition {
    pub executor: String,
    pub steps: Vec<ExecutionStepDefinition>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionStepDefinition {
    pub name: String,
    pub effect: String,
    pub audit: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntentFailureDefinition {
    pub audit: String,
    pub response: String,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntentAuditDefinition {
    pub required_events: Vec<String>,
    pub include_fields: Vec<String>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutorCatalog {
    pub schema_version: u32,
    pub executors: Vec<ExecutorDefinition>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutorDefinition {
    pub name: String,
    pub allowed_effects: Vec<String>,
    pub required_audit_events: Vec<String>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditCatalog {
    pub schema_version: u32,
    pub required_fields: Vec<String>,
    pub events: Vec<AuditEventDefinition>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEventDefinition {
    pub name: String,
    pub required_fields: Vec<String>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemorySchema {
    pub schema_version: u32,
    pub effects: Vec<MemoryEffectDefinition>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEffectDefinition {
    pub name: String,
    pub path: String,
    pub audit: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelSchema {
    pub schema_version: u32,
    pub kind: ChannelKind,
    pub capabilities: DeliveryPolicy,
    pub routing_policy: RoutingPolicy,
    pub external_ref_fields: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemSchema {
    pub schema_version: u32,
    pub platform: String,
    pub pipeline: Vec<String>,
    pub schema_layers: Vec<String>,
    pub intents: IntentCatalog,
    pub executors: ExecutorCatalog,
    pub audit: AuditCatalog,
    pub memory: MemorySchema,
    pub channels: Vec<ChannelSchema>,
}

#[derive(Debug, Clone)]
pub struct SystemSchemaCatalog {
    pub intents: IntentCatalog,
    pub executors: ExecutorCatalog,
    pub audit: AuditCatalog,
}

impl SystemSchemaCatalog {
    pub fn from_embedded() -> Self {
        let schema: SystemSchema = serde_yaml::from_str(include_str!("../../schemas/system.yaml"))
            .expect("embedded system schema must parse");
        Self {
            intents: schema.intents,
            executors: schema.executors,
            audit: schema.audit,
        }
    }

    pub fn validate_intent_execution(
        &self,
        intent_name: &str,
        executor_name: &str,
        audit_policy: &AuditPolicy,
    ) -> Result<(), SchemaValidationError> {
        let intent = self
            .intents
            .intents
            .iter()
            .find(|intent| intent.name == intent_name)
            .ok_or_else(|| SchemaValidationError::MissingIntent(intent_name.to_string()))?;

        if !intent.enabled_by_default {
            return Err(SchemaValidationError::DisabledIntent(
                intent_name.to_string(),
            ));
        }

        if intent.execution.executor != executor_name {
            return Err(SchemaValidationError::ExecutorMismatch {
                intent: intent_name.to_string(),
                expected: intent.execution.executor.clone(),
                actual: executor_name.to_string(),
            });
        }

        let executor = self
            .executors
            .executors
            .iter()
            .find(|executor| executor.name == executor_name)
            .ok_or_else(|| SchemaValidationError::MissingExecutor(executor_name.to_string()))?;

        for step in &intent.execution.steps {
            if !executor.allowed_effects.contains(&step.effect) {
                return Err(SchemaValidationError::EffectNotAllowed {
                    executor: executor_name.to_string(),
                    effect: step.effect.clone(),
                });
            }

            if let Some(audit_event) = &step.audit {
                self.validate_audit_event_declared(audit_event)?;
            }
        }

        self.validate_audit_event_declared(&intent.failure.audit)?;

        for required_event in &intent.audit.required_events {
            self.validate_audit_event_declared(required_event)?;
            let Some(audit_event) = AuditEvent::from_schema_name(required_event) else {
                return Err(SchemaValidationError::UnknownRuntimeAuditEvent(
                    required_event.clone(),
                ));
            };

            if !audit_policy.emits(&audit_event) {
                return Err(SchemaValidationError::RequiredAuditSuppressed {
                    intent: intent_name.to_string(),
                    event: required_event.clone(),
                });
            }
        }

        Ok(())
    }

    fn validate_audit_event_declared(&self, event_name: &str) -> Result<(), SchemaValidationError> {
        if self
            .audit
            .events
            .iter()
            .any(|event| event.name == event_name)
        {
            Ok(())
        } else {
            Err(SchemaValidationError::MissingAuditEvent(
                event_name.to_string(),
            ))
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchemaValidationError {
    MissingIntent(String),
    DisabledIntent(String),
    MissingExecutor(String),
    ExecutorMismatch {
        intent: String,
        expected: String,
        actual: String,
    },
    EffectNotAllowed {
        executor: String,
        effect: String,
    },
    MissingAuditEvent(String),
    UnknownRuntimeAuditEvent(String),
    RequiredAuditSuppressed {
        intent: String,
        event: String,
    },
}

impl std::fmt::Display for SchemaValidationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingIntent(intent) => write!(formatter, "missing intent schema: {}", intent),
            Self::DisabledIntent(intent) => {
                write!(formatter, "intent disabled by schema: {}", intent)
            }
            Self::MissingExecutor(executor) => {
                write!(formatter, "missing executor schema: {}", executor)
            }
            Self::ExecutorMismatch {
                intent,
                expected,
                actual,
            } => write!(
                formatter,
                "intent {} requires executor {}, got {}",
                intent, expected, actual
            ),
            Self::EffectNotAllowed { executor, effect } => write!(
                formatter,
                "executor {} is not allowed to perform effect {}",
                executor, effect
            ),
            Self::MissingAuditEvent(event) => {
                write!(formatter, "missing audit event schema: {}", event)
            }
            Self::UnknownRuntimeAuditEvent(event) => {
                write!(formatter, "unknown runtime audit event: {}", event)
            }
            Self::RequiredAuditSuppressed { intent, event } => write!(
                formatter,
                "intent {} requires audit event {}, but instance policy suppresses it",
                intent, event
            ),
        }
    }
}

impl std::error::Error for SchemaValidationError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_binding_extracts_discord_fields_from_agent_id() {
        let binding = ChannelBinding::initial("discord-1-2-3", "thread");

        assert_eq!(binding.channel.kind, ChannelKind::Discord);
        assert_eq!(
            binding.channel.fields.get("guild_id"),
            Some(&"1".to_string())
        );
        assert_eq!(
            binding.channel.fields.get("channel_id"),
            Some(&"2".to_string())
        );
        assert_eq!(
            binding.channel.fields.get("thread_id"),
            Some(&"3".to_string())
        );
    }

    #[test]
    fn repo_level_system_schema_file_parses() {
        let schema: SystemSchema =
            serde_yaml::from_str(include_str!("../../schemas/system.yaml")).unwrap();

        assert_eq!(schema.platform, "agents_village");
        assert!(schema
            .intents
            .intents
            .iter()
            .any(|intent| intent.name == "chat"));
        assert!(schema
            .audit
            .events
            .iter()
            .any(|event| event.name == "chat_started"));
        assert!(schema
            .memory
            .effects
            .iter()
            .any(|effect| effect.name == "session.append"));
        assert!(schema
            .executors
            .executors
            .iter()
            .any(|executor| executor.name == "chat_executor"));
        assert!(schema
            .channels
            .iter()
            .any(|channel| channel.kind == ChannelKind::Discord));
    }

    #[test]
    fn system_schema_validates_chat_contract() {
        let catalog = SystemSchemaCatalog::from_embedded();
        let audit_policy = AuditPolicy::default();

        catalog
            .validate_intent_execution("chat", "chat_executor", &audit_policy)
            .unwrap();
    }

    #[test]
    fn system_schema_rejects_suppressed_required_audit() {
        let catalog = SystemSchemaCatalog::from_embedded();
        let audit_policy = AuditPolicy {
            emit_events: vec![AuditEvent::ChatStarted],
            persist_session_transcript: true,
        };

        let error = catalog
            .validate_intent_execution("chat", "chat_executor", &audit_policy)
            .unwrap_err();

        assert_eq!(
            error,
            SchemaValidationError::RequiredAuditSuppressed {
                intent: "chat".to_string(),
                event: "chat_completed".to_string(),
            }
        );
    }
}
