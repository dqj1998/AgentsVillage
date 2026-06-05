// Discord platform adapter: shared Discord-specific helpers used by the
// runtime request handling path.

use chrono::Utc;
use serenity::model::channel::ChannelType;
use serenity::model::id::ChannelId;
use serenity::prelude::Context;
use tracing::warn;

use crate::app::AppResponse;

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

/// Split a message into chunks of max_len characters, preferring newline/sentence boundaries
pub fn split_message(content: &str, max_len: usize) -> Vec<String> {
    if content.len() <= max_len {
        return vec![content.to_string()];
    }

    let mut chunks = Vec::new();
    let mut remaining = content;

    while remaining.len() > max_len {
        let window = &remaining[..max_len];
        let split_pos = window
            .rfind('\n')
            .filter(|&p| p > max_len.saturating_sub(200))
            .or_else(|| {
                window
                    .rfind(['.', '!', '?'])
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

/// Determine if a Discord channel is a thread; returns the channel ID if so
pub async fn get_thread_id(ctx: &Context, channel_id: ChannelId) -> Option<u64> {
    match ctx.http.get_channel(channel_id).await {
        Ok(channel) => {
            if let Some(guild_channel) = channel.guild() {
                if matches!(
                    guild_channel.kind,
                    ChannelType::PublicThread
                        | ChannelType::PrivateThread
                        | ChannelType::NewsThread
                ) {
                    return Some(channel_id.get());
                }
            }
            None
        }
        Err(e) => {
            warn!("Failed to get channel info for {}: {}", channel_id, e);
            None
        }
    }
}

/// Get the display name of a Discord channel
pub async fn get_channel_name(ctx: &Context, channel_id: ChannelId) -> Option<String> {
    match ctx.http.get_channel(channel_id).await {
        Ok(channel) => {
            if let Some(guild_channel) = channel.guild() {
                Some(guild_channel.name.clone())
            } else {
                None
            }
        }
        Err(e) => {
            warn!("Failed to get channel name for {}: {}", channel_id, e);
            None
        }
    }
}

/// Map AppResponse to a Discord-sendable string (for say/typing flows).
pub fn app_response_to_text(response: &AppResponse) -> &str {
    match response {
        AppResponse::Text(s) => s.as_str(),
        AppResponse::Ephemeral(s) => s.as_str(),
        AppResponse::Error(s) => s.as_str(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_message_short_returns_single_chunk() {
        let msg = "Hello, world!";
        let chunks = split_message(msg, 2000);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], msg);
    }

    #[test]
    fn split_message_exact_limit_returns_single_chunk() {
        let msg = "a".repeat(2000);
        let chunks = split_message(&msg, 2000);
        assert_eq!(chunks.len(), 1);
    }

    #[test]
    fn split_message_long_splits_into_multiple_chunks() {
        let msg = "x".repeat(5000);
        let chunks = split_message(&msg, 2000);
        assert!(
            chunks.len() >= 3,
            "expected ≥3 chunks, got {}",
            chunks.len()
        );
        for chunk in &chunks {
            assert!(
                chunk.len() <= 2000,
                "chunk length {} exceeds 2000",
                chunk.len()
            );
        }
    }

    #[test]
    fn split_message_no_empty_chunks() {
        let msg = "line one\nline two\n".repeat(300);
        let chunks = split_message(&msg, 2000);
        for chunk in &chunks {
            assert!(!chunk.is_empty(), "found an empty chunk");
        }
    }

    #[test]
    fn split_message_prefers_newline_boundary() {
        let msg = format!("{}\n{}", "a".repeat(1900), "b".repeat(200));
        let chunks = split_message(&msg, 2000);
        assert_eq!(chunks[0], "a".repeat(1900));
        assert_eq!(chunks[1], "b".repeat(200));
    }

    #[test]
    fn split_message_reconstructs_original_content() {
        let msg = "Hello\nWorld\nFoo\nBar\n".repeat(200);
        let chunks = split_message(&msg, 2000);
        let rejoined = chunks.join("\n");
        assert!(rejoined.contains("Hello"));
        assert!(rejoined.contains("World"));
    }

    #[test]
    fn build_agent_id_without_thread() {
        let id = build_agent_id(123, 456, None);
        assert_eq!(id, "discord-123-456");
    }

    #[test]
    fn build_agent_id_with_thread() {
        let id = build_agent_id(123, 456, Some(789));
        assert_eq!(id, "discord-123-456-789");
    }

    #[test]
    fn build_agent_id_zero_values() {
        let id = build_agent_id(0, 0, None);
        assert_eq!(id, "discord-0-0");
    }

    #[test]
    fn build_agent_id_large_ids() {
        let id = build_agent_id(u64::MAX, u64::MAX, Some(u64::MAX));
        assert_eq!(
            id,
            format!("discord-{}-{}-{}", u64::MAX, u64::MAX, u64::MAX)
        );
    }
}
