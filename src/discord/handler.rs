use std::sync::Arc;

use async_trait::async_trait;
use serenity::model::application::Interaction;
use serenity::model::channel::{ChannelType, GuildChannel, Message};
use serenity::model::gateway::Ready;
use serenity::model::guild::Guild;
use serenity::model::id::ChannelId;
use serenity::prelude::*;
use tokio::sync::Mutex;
use tracing::{error, info, warn};

use crate::agent::{AgentManager, MemoryManager};
use crate::config::GlobalConfig;
use crate::llm::LlmClient;

use super::router::{build_agent_id, build_llm_messages, current_timestamp, split_message};

const CONTEXT_WINDOW: usize = 20;

pub struct DiscordHandler {
    pub agent_manager: Arc<Mutex<AgentManager>>,
    pub llm_client: Arc<LlmClient>,
    #[allow(dead_code)]
    pub config: GlobalConfig,
}

impl DiscordHandler {
    pub fn new(
        agent_manager: Arc<Mutex<AgentManager>>,
        llm_client: Arc<LlmClient>,
        config: GlobalConfig,
    ) -> Self {
        Self {
            agent_manager,
            llm_client,
            config,
        }
    }
}

#[async_trait]
impl EventHandler for DiscordHandler {
    async fn ready(&self, ctx: Context, ready: Ready) {
        info!("Discord bot connected as: {}", ready.user.name);

        // Register slash commands with Discord
        let commands = vec![
            serenity::builder::CreateCommand::new("new")
                .description("Start a fresh conversation (clears today's session, keeps long-term memory)"),
        ];

        match ctx.http.create_global_commands(&commands).await {
            Ok(registered) => {
                info!(
                    "Registered {} global slash command(s): {:?}",
                    registered.len(),
                    registered.iter().map(|c| &c.name).collect::<Vec<_>>()
                );
            }
            Err(e) => {
                warn!("Failed to register global slash commands: {}", e);
            }
        }
    }

    async fn guild_create(&self, _ctx: Context, guild: Guild, _is_new: Option<bool>) {
        info!("Connected to guild: {} ({})", guild.name, guild.id);
    }

    async fn message(&self, ctx: Context, msg: Message) {
        // Ignore bot messages
        if msg.author.bot {
            return;
        }

        let guild_id = match msg.guild_id {
            Some(gid) => gid.get(),
            None => {
                // DM - not supported
                return;
            }
        };

        let channel_id = msg.channel_id.get();

        // Determine if this is a thread message
        let thread_id = get_thread_id(&ctx, msg.channel_id).await;

        // Build agent ID
        let agent_id = build_agent_id(guild_id, channel_id, thread_id);

        // Get channel/thread display name
        let display_name = get_channel_name(&ctx, msg.channel_id).await
            .unwrap_or_else(|| format!("channel-{}", channel_id));

        info!(
            "Message from {} in {} (agent: {}): {}",
            msg.author.name,
            display_name,
            agent_id,
            &msg.content[..msg.content.len().min(50)]
        );

        // Get or create agent
        let agent = {
            let mut manager = self.agent_manager.lock().await;
            match manager.get_or_create_agent(&agent_id, &display_name).await {
                Ok(a) => a.clone(),
                Err(e) => {
                    error!("Failed to get/create agent {}: {}", agent_id, e);
                    return;
                }
            }
        };

        let memory_manager = MemoryManager::new(agent.workspace_path.clone());
        let timestamp = current_timestamp();

        // Append user message to session
        if let Err(e) = memory_manager
            .append_session(&msg.author.name, &msg.content, &timestamp)
            .await
        {
            warn!("Failed to append user message to session: {}", e);
        }

        // Build LLM context
        let llm_messages = match build_llm_messages(
            &agent,
            &memory_manager,
            &self.llm_client,
            CONTEXT_WINDOW,
        )
        .await
        {
            Ok(msgs) => msgs,
            Err(e) => {
                error!("Failed to build LLM messages: {}", e);
                return;
            }
        };

        // Show typing indicator
        let _ = msg.channel_id.broadcast_typing(&ctx.http).await;

        // Call LLM
        let response = match self.llm_client.chat(llm_messages).await {
            Ok(r) => r,
            Err(e) => {
                error!("LLM call failed: {}", e);
                let _ = msg
                    .channel_id
                    .say(&ctx.http, "Sorry, I encountered an error processing your message.")
                    .await;
                return;
            }
        };

        // Append assistant response to session
        let assistant_timestamp = current_timestamp();
        if let Err(e) = memory_manager
            .append_session("assistant", &response, &assistant_timestamp)
            .await
        {
            warn!("Failed to append assistant response to session: {}", e);
        }

        // Send response to Discord (handle 2000 char limit)
        let chunks = split_message(&response, 2000);
        for chunk in chunks {
            if let Err(e) = msg.channel_id.say(&ctx.http, &chunk).await {
                error!("Failed to send message to Discord: {}", e);
            }
        }
    }

    async fn thread_create(&self, _ctx: Context, thread: GuildChannel) {
        let guild_id = thread.guild_id.get();
        let parent_channel_id = thread.parent_id.map(|p| p.get()).unwrap_or(0);
        let thread_id = thread.id.get();

        let agent_id = build_agent_id(guild_id, parent_channel_id, Some(thread_id));
        let display_name = thread.name.clone();

        info!("New thread created: {} (agent: {})", display_name, agent_id);

        // Pre-create agent instance for the new thread
        let mut manager = self.agent_manager.lock().await;
        match manager.get_or_create_agent(&agent_id, &display_name).await {
            Ok(_) => {
                info!("Pre-created agent for thread: {}", agent_id);
            }
            Err(e) => {
                warn!("Failed to pre-create agent for thread {}: {}", agent_id, e);
            }
        }
    }

    async fn interaction_create(&self, ctx: Context, interaction: Interaction) {
        if let Interaction::Command(command) = interaction {
            info!(
                "Slash command /{} from {} in channel {}",
                command.data.name, command.user.name, command.channel_id
            );

            match command.data.name.as_str() {
                "new" => {
                    let guild_id = match command.guild_id {
                        Some(gid) => gid.get(),
                        None => {
                            let _ = command
                                .create_response(
                                    &ctx.http,
                                    serenity::builder::CreateInteractionResponse::Message(
                                        serenity::builder::CreateInteractionResponseMessage::new()
                                            .content("❌ This command can only be used in a server channel.")
                                            .ephemeral(true),
                                    ),
                                )
                                .await;
                            return;
                        }
                    };

                    let channel_id = command.channel_id.get();
                    let thread_id = get_thread_id(&ctx, command.channel_id).await;
                    let agent_id = build_agent_id(guild_id, channel_id, thread_id);

                    // Get agent workspace path
                    let workspace_path = {
                        let mut manager = self.agent_manager.lock().await;
                        match manager.get_or_create_agent(&agent_id, &format!("channel-{}", channel_id)).await {
                            Ok(agent) => agent.workspace_path.clone(),
                            Err(e) => {
                                error!("Failed to get agent for /new: {}", e);
                                let _ = command
                                    .create_response(
                                        &ctx.http,
                                        serenity::builder::CreateInteractionResponse::Message(
                                            serenity::builder::CreateInteractionResponseMessage::new()
                                                .content("❌ Failed to find agent for this channel.")
                                                .ephemeral(true),
                                        ),
                                    )
                                    .await;
                                return;
                            }
                        }
                    };

                    let memory_manager = MemoryManager::new(workspace_path);
                    match memory_manager.clear_today_session().await {
                        Ok(()) => {
                            info!("Cleared today's session for agent {}", agent_id);
                            let _ = command
                                .create_response(
                                    &ctx.http,
                                    serenity::builder::CreateInteractionResponse::Message(
                                        serenity::builder::CreateInteractionResponseMessage::new()
                                            .content("🆕 Started a new conversation! Today's session has been cleared. Long-term memory is preserved.")
                                            .ephemeral(true),
                                    ),
                                )
                                .await;
                        }
                        Err(e) => {
                            error!("Failed to clear session for /new: {}", e);
                            let _ = command
                                .create_response(
                                    &ctx.http,
                                    serenity::builder::CreateInteractionResponse::Message(
                                        serenity::builder::CreateInteractionResponseMessage::new()
                                            .content("❌ Failed to clear session. Please try again.")
                                            .ephemeral(true),
                                    ),
                                )
                                .await;
                        }
                    }
                }
                unknown => {
                    warn!("Unknown slash command: /{}", unknown);
                    let _ = command
                        .create_response(
                            &ctx.http,
                            serenity::builder::CreateInteractionResponse::Message(
                                serenity::builder::CreateInteractionResponseMessage::new()
                                    .content(format!("❓ Unknown command: `/{}`", unknown))
                                    .ephemeral(true),
                            ),
                        )
                        .await;
                }
            }
        }
    }
}

/// Get thread ID if the channel is a thread
async fn get_thread_id(ctx: &Context, channel_id: ChannelId) -> Option<u64> {
    // Try REST API to get channel info
    match ctx.http.get_channel(channel_id).await {
        Ok(channel) => {
            // In serenity 0.12, Channel is an enum: Guild(GuildChannel) or Private(PrivateChannel)
            if let Some(guild_channel) = channel.guild() {
                match guild_channel.kind {
                    ChannelType::PublicThread
                    | ChannelType::PrivateThread
                    | ChannelType::NewsThread => Some(channel_id.get()),
                    _ => None,
                }
            } else {
                None
            }
        }
        Err(e) => {
            warn!("Failed to get channel info for {}: {}", channel_id, e);
            None
        }
    }
}

/// Get channel display name
async fn get_channel_name(ctx: &Context, channel_id: ChannelId) -> Option<String> {
    // Try cache first using guild channels
    // Fall back to REST API
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
