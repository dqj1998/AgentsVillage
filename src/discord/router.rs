use anyhow::Result;
use chrono::Utc;
use tracing::info;

use crate::agent::{Agent, MemoryManager};
use crate::llm::{ChatMessage, LlmClient};

/// Build the LLM messages array from agent context and recent session history
pub async fn build_llm_messages(
    agent: &Agent,
    memory_manager: &MemoryManager,
    llm_client: &LlmClient,
    context_window: usize,
) -> Result<Vec<ChatMessage>> {
    // Read memory.md
    let memory_content = memory_manager.read_memory().await?;

    // Load recent messages
    let recent_messages = memory_manager.load_recent_messages(context_window).await?;

    // Count total messages
    let total_messages = memory_manager.count_total_messages().await?;

    // If total > context_window, summarize the oldest messages beyond the window
    if total_messages > context_window {
        let all_messages = memory_manager.load_recent_messages(total_messages).await?;
        let old_count = total_messages.saturating_sub(context_window);
        if old_count > 0 {
            let old_messages = &all_messages[..old_count.min(all_messages.len())];
            if !old_messages.is_empty() {
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

                match llm_client.chat(summary_messages).await {
                    Ok(summary) => {
                        info!("Generated memory summary for agent {}", agent.id);
                        memory_manager.append_memory(&summary).await?;
                    }
                    Err(e) => {
                        tracing::warn!("Failed to generate memory summary: {}", e);
                    }
                }
            }
        }
    }

    // Build system message
    let system_content = if memory_content.trim().is_empty()
        || memory_content.trim() == "# Long-term Memory"
    {
        agent.role.clone()
    } else {
        format!("{}\n\n## Long-term Memory\n{}", agent.role, memory_content)
    };

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

/// Split a message into chunks of max_len characters, preferring newline/sentence boundaries
pub fn split_message(content: &str, max_len: usize) -> Vec<String> {
    if content.len() <= max_len {
        return vec![content.to_string()];
    }

    let mut chunks = Vec::new();
    let mut remaining = content;

    while remaining.len() > max_len {
        // Try to split at a newline within the last 200 chars of the window
        let window = &remaining[..max_len];
        let split_pos = window.rfind('\n')
            .filter(|&p| p > max_len.saturating_sub(200))
            .or_else(|| {
                // Try to split at a sentence boundary (. ! ?)
                window.rfind(['.', '!', '?'])
                    .filter(|&p| p > max_len.saturating_sub(200))
                    .map(|p| p + 1)
            })
            .unwrap_or(max_len);

        chunks.push(remaining[..split_pos].to_string());
        remaining = remaining[split_pos..].trim_start_matches('\n');
    }

    if !remaining.is_empty() {
        chunks.push(remaining.to_string());
    }

    chunks
}

/// Build agent ID from Discord IDs
pub fn build_agent_id(guild_id: u64, channel_id: u64, thread_id: Option<u64>) -> String {
    match thread_id {
        Some(tid) => format!("discord-{}-{}-{}", guild_id, channel_id, tid),
        None => format!("discord-{}-{}", guild_id, channel_id),
    }
}

/// Get current UTC timestamp string
pub fn current_timestamp() -> String {
    Utc::now().format("%Y-%m-%d %H:%M:%S UTC").to_string()
}
