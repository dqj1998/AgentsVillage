use anyhow::Result;
use serenity::prelude::GatewayIntents;

use super::handler::DiscordHandler;

pub async fn build_discord_client(
    token: &str,
    handler: DiscordHandler,
    intents: GatewayIntents,
) -> Result<serenity::Client> {
    let client = serenity::Client::builder(token, intents)
        .event_handler(handler)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to build Discord client: {}", e))?;

    Ok(client)
}

pub fn default_intents() -> GatewayIntents {
    GatewayIntents::GUILDS
        | GatewayIntents::GUILD_MESSAGES
        | GatewayIntents::MESSAGE_CONTENT
        | GatewayIntents::GUILD_MESSAGE_TYPING
        | GatewayIntents::GUILD_MESSAGE_REACTIONS
}
