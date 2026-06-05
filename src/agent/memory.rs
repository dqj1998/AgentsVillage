use anyhow::{Context, Result};
use chrono::Utc;
use std::path::PathBuf;
use tokio::fs;
use tokio::io::AsyncWriteExt;
use tracing::debug;

pub struct MemoryManager {
    pub workspace_path: PathBuf,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct SessionMessage {
    pub role: String,
    pub author: String,
    pub content: String,
    pub timestamp: String,
}

impl MemoryManager {
    pub fn new(workspace_path: PathBuf) -> Self {
        Self { workspace_path }
    }

    /// Append message to today's session file: workspace/{agent_id}/sessions/YYYY-MM-DD.md
    pub async fn append_session(&self, author: &str, content: &str, timestamp: &str) -> Result<()> {
        let sessions_dir = self.workspace_path.join("sessions");
        fs::create_dir_all(&sessions_dir)
            .await
            .context("Failed to create sessions directory")?;

        let today = Utc::now().format("%Y-%m-%d").to_string();
        let session_file = sessions_dir.join(format!("{}.md", today));

        // Determine role from author
        let role = if author == "assistant" || author == "Bot" {
            "assistant"
        } else {
            "user"
        };

        let entry = format!("## {} | {}: {}\n{}\n\n", timestamp, role, author, content);

        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&session_file)
            .await
            .context("Failed to open session file")?;

        file.write_all(entry.as_bytes())
            .await
            .context("Failed to write to session file")?;

        debug!("Appended message to session file: {:?}", session_file);
        Ok(())
    }

    /// Load last N messages from session files (most recent first, across multiple days if needed)
    pub async fn load_recent_messages(&self, limit: usize) -> Result<Vec<SessionMessage>> {
        let sessions_dir = self.workspace_path.join("sessions");

        if !sessions_dir.exists() {
            return Ok(vec![]);
        }

        // List all session files, sorted descending (most recent first)
        let mut entries = fs::read_dir(&sessions_dir)
            .await
            .context("Failed to read sessions directory")?;

        let mut files: Vec<String> = Vec::new();
        while let Some(entry) = entries.next_entry().await? {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.ends_with(".md") {
                files.push(name);
            }
        }
        files.sort_by(|a, b| b.cmp(a)); // descending order

        let mut messages: Vec<SessionMessage> = Vec::new();

        for file_name in &files {
            if messages.len() >= limit {
                break;
            }

            let file_path = sessions_dir.join(file_name);
            let content = fs::read_to_string(&file_path)
                .await
                .context("Failed to read session file")?;

            let file_messages = parse_session_file(&content);
            // Add in reverse order (most recent first) then we'll reverse at the end
            for msg in file_messages.into_iter().rev() {
                messages.push(msg);
                if messages.len() >= limit {
                    break;
                }
            }
        }

        // Reverse to get chronological order
        messages.reverse();
        Ok(messages)
    }

    /// Read memory.md content
    pub async fn read_memory(&self) -> Result<String> {
        let memory_file = self.workspace_path.join("memory.md");
        if !memory_file.exists() {
            return Ok(String::new());
        }
        let content = fs::read_to_string(&memory_file)
            .await
            .context("Failed to read memory.md")?;
        Ok(content)
    }

    /// Append summary to memory.md
    pub async fn append_memory(&self, summary: &str) -> Result<()> {
        let memory_file = self.workspace_path.join("memory.md");
        let timestamp = Utc::now().format("%Y-%m-%d %H:%M:%S UTC").to_string();
        let entry = format!("\n## Summary ({})\n{}\n", timestamp, summary);

        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&memory_file)
            .await
            .context("Failed to open memory.md")?;

        file.write_all(entry.as_bytes())
            .await
            .context("Failed to write to memory.md")?;

        debug!("Appended summary to memory.md");
        Ok(())
    }

    /// Clear today's session file (truncate to empty), preserving memory.md
    pub async fn clear_today_session(&self) -> Result<()> {
        let sessions_dir = self.workspace_path.join("sessions");
        let today = Utc::now().format("%Y-%m-%d").to_string();
        let session_file = sessions_dir.join(format!("{}.md", today));

        if session_file.exists() {
            fs::write(&session_file, "")
                .await
                .context("Failed to clear today's session file")?;
            debug!("Cleared today's session file: {:?}", session_file);
        }
        Ok(())
    }

    /// Count messages in today's session file only.
    /// Note: This is a pragmatic fix for the O(n) IO debt — full history counting
    /// is being replaced by EventLog-based state tracking.
    pub async fn count_total_messages(&self) -> Result<usize> {
        let sessions_dir = self.workspace_path.join("sessions");

        if !sessions_dir.exists() {
            return Ok(0);
        }

        let today = Utc::now().format("%Y-%m-%d").to_string();
        let session_file = sessions_dir.join(format!("{}.md", today));

        if !session_file.exists() {
            return Ok(0);
        }

        let content = fs::read_to_string(&session_file).await.unwrap_or_default();

        // Count "## " headers as message markers
        let mut total = content.matches("\n## ").count();
        if content.starts_with("## ") {
            total += 1;
        }

        Ok(total)
    }
}

/// Parse session file content into SessionMessage list.
/// Known limitation: user messages containing "\n## " on a line by itself
/// will cause incorrect splitting. This is tracked as technical debt.
/// The timestamp guard (must start with a digit) reduces false positives.
pub(crate) fn parse_session_file(content: &str) -> Vec<SessionMessage> {
    let mut messages = Vec::new();

    // Split on "## " headers
    let sections: Vec<&str> = content.split("\n## ").collect();

    for (i, section) in sections.iter().enumerate() {
        let section = if i == 0 && section.starts_with("## ") {
            &section[3..]
        } else if i == 0 {
            continue; // skip empty first section
        } else {
            section
        };

        // Parse header line: "YYYY-MM-DD HH:MM:SS UTC | role: Author"
        let lines: Vec<&str> = section.splitn(2, '\n').collect();
        if lines.is_empty() {
            continue;
        }

        let header = lines[0].trim();
        let body = if lines.len() > 1 {
            lines[1].trim_end().to_string()
        } else {
            String::new()
        };

        // Parse "timestamp | role: author"
        let parts: Vec<&str> = header.splitn(2, " | ").collect();
        if parts.len() != 2 {
            continue;
        }

        let timestamp = parts[0].trim().to_string();

        // Timestamp guard: must start with a digit (e.g. "2024-...") to reduce
        // false positives from "## " appearing inside message bodies.
        if !timestamp
            .chars()
            .next()
            .map(|c| c.is_ascii_digit())
            .unwrap_or(false)
        {
            continue;
        }

        let role_author = parts[1].trim();

        // Parse "role: author"
        let ra_parts: Vec<&str> = role_author.splitn(2, ": ").collect();
        if ra_parts.len() != 2 {
            continue;
        }

        let role = ra_parts[0].trim().to_string();
        let author = ra_parts[1].trim().to_string();

        messages.push(SessionMessage {
            role,
            author,
            content: body,
            timestamp,
        });
    }

    messages
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    // ── parse_session_file unit tests ────────────────────────────────────────

    #[test]
    fn parse_session_file_empty_returns_empty_vec() {
        let result = parse_session_file("");
        assert!(result.is_empty());
    }

    #[test]
    fn parse_session_file_single_entry_parses_correctly() {
        let content = "## 2024-01-15 10:30:00 UTC | user: Alice\nHello there!\n\n";
        let messages = parse_session_file(content);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].role, "user");
        assert_eq!(messages[0].author, "Alice");
        assert_eq!(messages[0].content, "Hello there!");
        assert_eq!(messages[0].timestamp, "2024-01-15 10:30:00 UTC");
    }

    #[test]
    fn parse_session_file_multiple_entries_parse_correctly() {
        let content = concat!(
            "## 2024-01-15 10:30:00 UTC | user: Alice\nHello!\n\n",
            "## 2024-01-15 10:31:00 UTC | assistant: Bot\nHi Alice!\n\n",
            "## 2024-01-15 10:32:00 UTC | user: Alice\nHow are you?\n\n",
        );
        let messages = parse_session_file(content);
        assert_eq!(messages.len(), 3);

        assert_eq!(messages[0].role, "user");
        assert_eq!(messages[0].author, "Alice");
        assert_eq!(messages[0].content, "Hello!");

        assert_eq!(messages[1].role, "assistant");
        assert_eq!(messages[1].author, "Bot");
        assert_eq!(messages[1].content, "Hi Alice!");

        assert_eq!(messages[2].role, "user");
        assert_eq!(messages[2].author, "Alice");
        assert_eq!(messages[2].content, "How are you?");
    }

    /// Documents the known fragile parsing behavior: a "## " inside the message
    /// body causes the parser to split on it, treating the body fragment as a
    /// new (malformed) header and silently dropping it.  This test captures the
    /// *current* (wrong) behavior so we know when it changes.
    #[test]
    fn parse_session_file_double_hash_in_body_documents_current_behavior() {
        let content = concat!(
            "## 2024-01-15 10:30:00 UTC | user: Alice\n",
            "Here is a heading:\n## Sub-section\nMore text\n\n",
        );
        let messages = parse_session_file(content);
        // Current behavior: the "## Sub-section" inside the body causes a split.
        // The first entry's content is truncated to just "Here is a heading:".
        // The fragment "Sub-section\nMore text" fails header parsing and is dropped.
        // So we end up with 1 message whose content does NOT include "Sub-section".
        assert_eq!(
            messages.len(),
            1,
            "current behavior: body ## causes split → 1 msg"
        );
        assert_eq!(messages[0].author, "Alice");
        assert!(
            !messages[0].content.contains("Sub-section"),
            "current behavior: Sub-section is lost due to fragile parsing"
        );
    }

    // ── MemoryManager integration tests ──────────────────────────────────────

    #[tokio::test]
    async fn memory_manager_append_and_load_round_trip() {
        let dir = tempdir().unwrap();
        let mm = MemoryManager::new(dir.path().to_path_buf());

        mm.append_session("Alice", "First message", "2024-01-15 10:00:00 UTC")
            .await
            .unwrap();
        mm.append_session("Bot", "Second message", "2024-01-15 10:01:00 UTC")
            .await
            .unwrap();
        mm.append_session("Alice", "Third message", "2024-01-15 10:02:00 UTC")
            .await
            .unwrap();

        let messages = mm.load_recent_messages(10).await.unwrap();
        assert_eq!(messages.len(), 3, "expected 3 messages back");

        // Chronological order
        assert_eq!(messages[0].content, "First message");
        assert_eq!(messages[1].content, "Second message");
        assert_eq!(messages[2].content, "Third message");
    }

    #[tokio::test]
    async fn memory_manager_load_recent_messages_respects_limit() {
        let dir = tempdir().unwrap();
        let mm = MemoryManager::new(dir.path().to_path_buf());

        for i in 0..5 {
            mm.append_session(
                "Alice",
                &format!("Message {}", i),
                "2024-01-15 10:00:00 UTC",
            )
            .await
            .unwrap();
        }

        let messages = mm.load_recent_messages(2).await.unwrap();
        assert_eq!(messages.len(), 2, "limit=2 should return only 2 messages");
        // Should be the 2 most recent (messages 3 and 4)
        assert_eq!(messages[0].content, "Message 3");
        assert_eq!(messages[1].content, "Message 4");
    }

    #[tokio::test]
    async fn memory_manager_clear_today_session_truncates_file() {
        let dir = tempdir().unwrap();
        let mm = MemoryManager::new(dir.path().to_path_buf());

        mm.append_session("Alice", "Hello", "2024-01-15 10:00:00 UTC")
            .await
            .unwrap();

        // Verify there's something there
        let before = mm.load_recent_messages(10).await.unwrap();
        assert!(!before.is_empty());

        mm.clear_today_session().await.unwrap();

        let after = mm.load_recent_messages(10).await.unwrap();
        assert!(
            after.is_empty(),
            "after clear, no messages should be returned"
        );
    }

    #[tokio::test]
    async fn memory_manager_count_total_messages_correct() {
        let dir = tempdir().unwrap();
        let mm = MemoryManager::new(dir.path().to_path_buf());

        let count_before = mm.count_total_messages().await.unwrap();
        assert_eq!(count_before, 0);

        for i in 0..4 {
            mm.append_session("Alice", &format!("Msg {}", i), "2024-01-15 10:00:00 UTC")
                .await
                .unwrap();
        }

        let count_after = mm.count_total_messages().await.unwrap();
        assert_eq!(count_after, 4);
    }

    #[tokio::test]
    async fn memory_manager_read_and_append_memory_round_trip() {
        let dir = tempdir().unwrap();
        let mm = MemoryManager::new(dir.path().to_path_buf());

        // Initially empty
        let initial = mm.read_memory().await.unwrap();
        assert!(initial.is_empty());

        mm.append_memory("Key fact: the sky is blue.")
            .await
            .unwrap();
        mm.append_memory("Key fact: water is wet.").await.unwrap();

        let content = mm.read_memory().await.unwrap();
        assert!(
            content.contains("Key fact: the sky is blue."),
            "first summary should be present"
        );
        assert!(
            content.contains("Key fact: water is wet."),
            "second summary should be present"
        );
    }
}
