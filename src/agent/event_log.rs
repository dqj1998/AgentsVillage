use anyhow::{Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tokio::fs;
use tokio::io::AsyncWriteExt;
use tracing::debug;

/// Events recorded in the append-only JSONL event log
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentEvent {
    ChatStarted {
        timestamp: String,
        author: String,
        user_text_len: usize,
    },
    ChatCompleted {
        timestamp: String,
        response_len: usize,
    },
    ChatFailed {
        timestamp: String,
        error: String,
    },
    MemorySummarized {
        timestamp: String,
        messages_summarized: usize,
    },
    ResetSession {
        timestamp: String,
        reason: String,
    },
}

impl AgentEvent {
    /// Convenience constructor: ChatStarted with current UTC timestamp
    pub fn chat_started(author: impl Into<String>, user_text_len: usize) -> Self {
        AgentEvent::ChatStarted {
            timestamp: Utc::now().format("%Y-%m-%d %H:%M:%S UTC").to_string(),
            author: author.into(),
            user_text_len,
        }
    }

    /// Convenience constructor: ChatCompleted with current UTC timestamp
    pub fn chat_completed(response_len: usize) -> Self {
        AgentEvent::ChatCompleted {
            timestamp: Utc::now().format("%Y-%m-%d %H:%M:%S UTC").to_string(),
            response_len,
        }
    }

    /// Convenience constructor: ChatFailed with current UTC timestamp
    pub fn chat_failed(error: impl Into<String>) -> Self {
        AgentEvent::ChatFailed {
            timestamp: Utc::now().format("%Y-%m-%d %H:%M:%S UTC").to_string(),
            error: error.into(),
        }
    }

    /// Convenience constructor: MemorySummarized with current UTC timestamp
    pub fn memory_summarized(messages_summarized: usize) -> Self {
        AgentEvent::MemorySummarized {
            timestamp: Utc::now().format("%Y-%m-%d %H:%M:%S UTC").to_string(),
            messages_summarized,
        }
    }

    /// Convenience constructor: ResetSession with current UTC timestamp
    pub fn reset_session(reason: impl Into<String>) -> Self {
        AgentEvent::ResetSession {
            timestamp: Utc::now().format("%Y-%m-%d %H:%M:%S UTC").to_string(),
            reason: reason.into(),
        }
    }
}

/// Append-only JSONL event log stored at workspace/{agent_id}/events/events.jsonl
pub struct EventLog {
    events_dir: PathBuf,
}

impl EventLog {
    pub fn new(workspace_path: PathBuf) -> Self {
        Self {
            events_dir: workspace_path.join("events"),
        }
    }

    fn log_path(&self) -> PathBuf {
        self.events_dir.join("events.jsonl")
    }

    /// Append a single event to the JSONL log
    pub async fn append(&self, event: AgentEvent) -> Result<()> {
        fs::create_dir_all(&self.events_dir)
            .await
            .context("Failed to create events directory")?;

        let line = serde_json::to_string(&event).context("Failed to serialize event")?;
        let line = format!("{}\n", line);

        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.log_path())
            .await
            .context("Failed to open events.jsonl")?;

        file.write_all(line.as_bytes())
            .await
            .context("Failed to write event")?;

        debug!("Appended event to {:?}", self.log_path());
        Ok(())
    }

    /// Read all events from the log
    #[cfg(test)]
    pub async fn read_all(&self) -> Result<Vec<AgentEvent>> {
        let path = self.log_path();
        if !path.exists() {
            return Ok(vec![]);
        }

        let content = fs::read_to_string(&path)
            .await
            .context("Failed to read events.jsonl")?;

        let mut events = Vec::new();
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            match serde_json::from_str::<AgentEvent>(line) {
                Ok(event) => events.push(event),
                Err(e) => {
                    tracing::warn!("Failed to parse event line: {} — {}", line, e);
                }
            }
        }

        Ok(events)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn event_log_append_and_read_all_round_trip() {
        let dir = tempdir().unwrap();
        let log = EventLog::new(dir.path().to_path_buf());

        // Initially empty
        let events = log.read_all().await.unwrap();
        assert!(events.is_empty(), "new log should be empty");

        // Append a ChatStarted event
        let event = AgentEvent::chat_started("Alice", 42);
        log.append(event).await.unwrap();

        let events = log.read_all().await.unwrap();
        assert_eq!(events.len(), 1);
        match &events[0] {
            AgentEvent::ChatStarted {
                author,
                user_text_len,
                ..
            } => {
                assert_eq!(author, "Alice");
                assert_eq!(*user_text_len, 42);
            }
            other => panic!("Expected ChatStarted, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn event_log_multiple_events_round_trip() {
        let dir = tempdir().unwrap();
        let log = EventLog::new(dir.path().to_path_buf());

        log.append(AgentEvent::chat_started("Bob", 10))
            .await
            .unwrap();
        log.append(AgentEvent::chat_completed(200)).await.unwrap();
        log.append(AgentEvent::chat_failed("timeout"))
            .await
            .unwrap();

        let events = log.read_all().await.unwrap();
        assert_eq!(events.len(), 3);

        assert!(matches!(events[0], AgentEvent::ChatStarted { .. }));
        assert!(matches!(events[1], AgentEvent::ChatCompleted { .. }));
        assert!(matches!(events[2], AgentEvent::ChatFailed { .. }));
    }

    #[tokio::test]
    async fn event_log_reset_session_round_trip() {
        let dir = tempdir().unwrap();
        let log = EventLog::new(dir.path().to_path_buf());

        let event = AgentEvent::reset_session("user requested /new");
        log.append(event).await.unwrap();

        let events = log.read_all().await.unwrap();
        assert_eq!(events.len(), 1);
        match &events[0] {
            AgentEvent::ResetSession { reason, .. } => {
                assert_eq!(reason, "user requested /new");
            }
            other => panic!("Expected ResetSession, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn event_log_skips_malformed_lines_gracefully() {
        let dir = tempdir().unwrap();
        let events_dir = dir.path().join("events");
        std::fs::create_dir_all(&events_dir).unwrap();
        let log_path = events_dir.join("events.jsonl");

        // Write a valid line, a malformed line, and another valid line
        let valid1 = serde_json::to_string(&AgentEvent::chat_started("Alice", 5)).unwrap();
        let valid2 = serde_json::to_string(&AgentEvent::chat_completed(100)).unwrap();
        let content = format!("{}\n{{not valid json}}\n{}\n", valid1, valid2);
        std::fs::write(&log_path, content).unwrap();

        let log = EventLog::new(dir.path().to_path_buf());
        let events = log.read_all().await.unwrap();

        // Malformed line should be skipped; valid lines should be parsed
        assert_eq!(events.len(), 2, "malformed line should be skipped");
        assert!(matches!(events[0], AgentEvent::ChatStarted { .. }));
        assert!(matches!(events[1], AgentEvent::ChatCompleted { .. }));
    }

    #[tokio::test]
    async fn event_log_memory_summarized_round_trip() {
        let dir = tempdir().unwrap();
        let log = EventLog::new(dir.path().to_path_buf());

        let event = AgentEvent::memory_summarized(15);
        log.append(event).await.unwrap();

        let events = log.read_all().await.unwrap();
        assert_eq!(events.len(), 1);
        match &events[0] {
            AgentEvent::MemorySummarized {
                messages_summarized,
                ..
            } => {
                assert_eq!(*messages_summarized, 15);
            }
            other => panic!("Expected MemorySummarized, got {:?}", other),
        }
    }

    #[test]
    fn agent_event_serialization_has_type_tag() {
        // Verify the serde tag is present in serialized form
        let event = AgentEvent::reset_session("test");
        let json = serde_json::to_string(&event).unwrap();
        assert!(
            json.contains("\"type\":\"reset_session\""),
            "serialized JSON should contain type tag: {}",
            json
        );
    }
}
