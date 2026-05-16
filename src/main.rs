use std::sync::Arc;

use anyhow::Result;
use tokio::sync::Mutex;
use tracing::{error, info};

mod agent;
mod config;
mod discord;
mod error;
mod llm;

use agent::AgentManager;
use config::{is_config_complete, load_global_config, run_init_wizard};
use discord::{build_discord_client, default_intents, DiscordHandler};
use llm::LlmClient;

#[tokio::main]
async fn main() -> Result<()> {
    // 1. Initialize tracing subscriber
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    info!("Starting AgentsVillage...");

    // 2. Load .env file
    match dotenvy::dotenv() {
        Ok(path) => info!("Loaded .env from {:?}", path),
        Err(_) => info!("No .env file found, using environment variables"),
    }

    // 3. Load global config.toml
    let mut global_config = load_global_config()?;

    // 4. If config incomplete, run init wizard
    if !is_config_complete(&global_config) {
        info!("Config is incomplete, running initialization wizard...");
        global_config = run_init_wizard().await?;
    }

    // 5. Create LlmClient from config + env
    let api_key = std::env::var("LLM_API_KEY").ok();
    let llm_client = Arc::new(LlmClient::new(global_config.llm.clone(), api_key));

    // 6. Create AgentManager, load all existing agents from workspace/
    let mut agent_manager = AgentManager::new(global_config.clone());
    agent_manager.load_all_agents().await?;
    let agent_manager = Arc::new(Mutex::new(agent_manager));

    // 7. Build Discord handler
    let handler = DiscordHandler::new(
        Arc::clone(&agent_manager),
        Arc::clone(&llm_client),
        global_config.clone(),
    );

    // 8. Get Discord token from environment
    let discord_token = std::env::var("DISCORD_TOKEN")
        .map_err(|_| anyhow::anyhow!("DISCORD_TOKEN environment variable not set. Please add it to your .env file."))?;

    // 9. Build and start Discord client
    let intents = default_intents();
    let mut discord_client = build_discord_client(&discord_token, handler, intents).await?;

    info!("Discord client built, connecting to gateway...");

    // 10. Handle Ctrl+C gracefully
    let shard_manager = discord_client.shard_manager.clone();

    tokio::spawn(async move {
        tokio::signal::ctrl_c()
            .await
            .expect("Failed to listen for Ctrl+C");
        info!("Received Ctrl+C, shutting down...");
        shard_manager.shutdown_all().await;
    });

    // Start the client
    if let Err(e) = discord_client.start().await {
        error!("Discord client error: {}", e);
        return Err(anyhow::anyhow!("Discord client error: {}", e));
    }

    info!("AgentsVillage shut down gracefully.");
    Ok(())
}
