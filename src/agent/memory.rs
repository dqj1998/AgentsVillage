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
    pub async fn append_session(
        &self,
        author: &str,
        content: &str,
        timestamp: &str,
    ) -> Result<()> {
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

        let entry = format!(
            "## {} | {}: {}\n{}\n\n",
            timestamp, role, author, content
        );

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

    /// Count total messages across all session files
    pub async fn count_total_messages(&self) -> Result<usize> {
        let sessions_dir = self.workspace_path.join("sessions");

        if !sessions_dir.exists() {
            return Ok(0);
        }

        let mut entries = fs::read_dir(&sessions_dir)
            .await
            .context("Failed to read sessions directory")?;

        let mut total = 0usize;

        while let Some(entry) = entries.next_entry().await? {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.ends_with(".md") {
                let file_path = sessions_dir.join(&name);
                let content = fs::read_to_string(&file_path)
                    .await
                    .unwrap_or_default();
                // Count "## " headers as message markers
                total += content.matches("\n## ").count();
                if content.starts_with("## ") {
                    total += 1;
                }
            }
        }

        Ok(total)
    }
}

/// Parse session file content into SessionMessage list
fn parse_session_file(content: &str) -> Vec<SessionMessage> {
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
