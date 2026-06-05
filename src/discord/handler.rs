// This handler delegates message and command execution to AppService.

use std::sync::Arc;

use async_trait::async_trait;
use serenity::model::application::Interaction;
use serenity::model::channel::{GuildChannel, Message};
use serenity::model::gateway::Ready;
use serenity::model::guild::Guild;
use serenity::prelude::*;
use tokio::sync::Mutex;
use tracing::{error, info, warn};

use crate::agent::AgentManager;
use crate::app::{service::AppService, AppRequest, AppResponse, RequestPayload};

use super::adapter::{
    app_response_to_text, build_agent_id, current_timestamp, get_channel_name, get_thread_id,
    split_message,
};

pub struct DiscordHandler {
    pub agent_manager: Arc<Mutex<AgentManager>>,
    pub app_service: Arc<AppService>,
}

impl DiscordHandler {
    pub fn new(agent_manager: Arc<Mutex<AgentManager>>, app_service: Arc<AppService>) -> Self {
        Self {
            agent_manager,
            app_service,
        }
    }
}

#[async_trait]
impl EventHandler for DiscordHandler {
    async fn ready(&self, ctx: Context, ready: Ready) {
        info!("Discord bot connected as: {}", ready.user.name);

        // Register slash commands with Discord
        let commands = vec![serenity::builder::CreateCommand::new("new").description(
            "Start a fresh conversation (clears today's session, keeps long-term memory)",
        )];

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
        let display_name = get_channel_name(&ctx, msg.channel_id)
            .await
            .unwrap_or_else(|| format!("channel-{}", channel_id));

        info!(
            "Message from {} in {} (agent: {}): {}",
            msg.author.name,
            display_name,
            agent_id,
            &msg.content[..msg.content.len().min(50)]
        );

        let request = AppRequest {
            agent_id: agent_id.clone(),
            platform_user: msg.author.name.clone(),
            timestamp: current_timestamp(),
            payload: RequestPayload::Message(msg.content.clone()),
        };

        let _ = msg.channel_id.broadcast_typing(&ctx.http).await;

        match self.app_service.handle(request).await {
            Ok(response) => {
                if let AppResponse::Error(text) = &response {
                    error!("AppService error response: {}", text);
                }

                let chunks = split_message(app_response_to_text(&response), 2000);
                for chunk in chunks {
                    if let Err(e) = msg.channel_id.say(&ctx.http, &chunk).await {
                        error!("Failed to send message to Discord: {}", e);
                    }
                }
            }
            Err(e) => {
                error!("AppService handle error: {}", e);
                let _ = msg
                    .channel_id
                    .say(
                        &ctx.http,
                        "Sorry, I encountered an error processing your message.",
                    )
                    .await;
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

        // Pre-create the agent instance for the new thread.
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

                    let request = AppRequest {
                        agent_id: agent_id.clone(),
                        platform_user: command.user.name.clone(),
                        timestamp: current_timestamp(),
                        payload: RequestPayload::Command {
                            name: "new".to_string(),
                            args: vec![],
                        },
                    };

                    match self.app_service.handle(request).await {
                        Ok(AppResponse::Ephemeral(text)) | Ok(AppResponse::Text(text)) => {
                            let _ = command
                                .create_response(
                                    &ctx.http,
                                    serenity::builder::CreateInteractionResponse::Message(
                                        serenity::builder::CreateInteractionResponseMessage::new()
                                            .content(text)
                                            .ephemeral(true),
                                    ),
                                )
                                .await;
                        }
                        Ok(AppResponse::Error(text)) => {
                            error!("AppService /new error: {}", text);
                            let _ = command
                                .create_response(
                                    &ctx.http,
                                    serenity::builder::CreateInteractionResponse::Message(
                                        serenity::builder::CreateInteractionResponseMessage::new()
                                            .content(format!("❌ {}", text))
                                            .ephemeral(true),
                                    ),
                                )
                                .await;
                        }
                        Err(e) => {
                            error!("AppService /new handle error: {}", e);
                            let _ = command
                                .create_response(
                                    &ctx.http,
                                    serenity::builder::CreateInteractionResponse::Message(
                                        serenity::builder::CreateInteractionResponseMessage::new()
                                            .content(
                                                "❌ Failed to clear session. Please try again.",
                                            )
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
