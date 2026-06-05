use anyhow::{Context, Result};
use dialoguer::{Input, Select};
use serde::{Deserialize, Serialize};
use std::path::Path;
use tracing::info;

use crate::llm::LlmClient;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CoreConfig {
    pub context_window: Option<usize>, // number of recent messages to include, default 20
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GlobalConfig {
    pub core: CoreConfig,
    pub platform: PlatformConfig,
    pub llm: LlmConfig,
    pub discord: DiscordConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PlatformConfig {
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LlmConfig {
    pub provider: Option<String>, // "openrouter" or "ollama"
    pub base_url: Option<String>,
    pub model: Option<String>,
    pub timeout_secs: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DiscordConfig {
    pub application_id: Option<String>,
    pub guild_id: Option<String>,
    pub channel_id: Option<String>,
    pub channel_name: Option<String>,
    pub intents: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AgentOverrideConfig {
    pub llm: Option<LlmConfig>,
    pub core: Option<CoreConfig>,
}

pub fn load_global_config() -> Result<GlobalConfig> {
    let config_path = Path::new("config.toml");
    if !config_path.exists() {
        return Ok(GlobalConfig::default());
    }

    let content = std::fs::read_to_string(config_path).context("Failed to read config.toml")?;

    if content.trim().is_empty() {
        return Ok(GlobalConfig::default());
    }

    let config: GlobalConfig = toml::from_str(&content).context("Failed to parse config.toml")?;

    Ok(config)
}

pub fn is_config_complete(config: &GlobalConfig) -> bool {
    config.platform.name.is_some()
        && config.llm.provider.is_some()
        && config.llm.base_url.is_some()
        && config.llm.model.is_some()
        && config.discord.application_id.is_some()
        && config.discord.guild_id.is_some()
        && config.discord.channel_id.is_some()
}

pub fn save_global_config(config: &GlobalConfig) -> Result<()> {
    let content = toml::to_string_pretty(config).context("Failed to serialize config")?;
    std::fs::write("config.toml", content).context("Failed to write config.toml")?;
    info!("Config saved to config.toml");
    Ok(())
}

pub fn merge_agent_config(
    global: &GlobalConfig,
    agent_override: Option<AgentOverrideConfig>,
) -> GlobalConfig {
    let mut merged = global.clone();
    if let Some(override_cfg) = agent_override {
        if let Some(llm_override) = override_cfg.llm {
            if llm_override.provider.is_some() {
                merged.llm.provider = llm_override.provider;
            }
            if llm_override.base_url.is_some() {
                merged.llm.base_url = llm_override.base_url;
            }
            if llm_override.model.is_some() {
                merged.llm.model = llm_override.model;
            }
            if llm_override.timeout_secs.is_some() {
                merged.llm.timeout_secs = llm_override.timeout_secs;
            }
        }
        if let Some(core_override) = override_cfg.core {
            if core_override.context_window.is_some() {
                merged.core.context_window = core_override.context_window;
            }
        }
    }
    merged
}

/// Return the configured context window size, defaulting to 20 if not set.
pub fn effective_context_window(config: &GlobalConfig) -> usize {
    config.core.context_window.unwrap_or(20)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_global_config() -> GlobalConfig {
        GlobalConfig {
            core: CoreConfig::default(),
            platform: PlatformConfig {
                name: Some("TestPlatform".to_string()),
            },
            llm: LlmConfig {
                provider: Some("openrouter".to_string()),
                base_url: Some("https://openrouter.ai/api/v1".to_string()),
                model: Some("openai/gpt-4o-mini".to_string()),
                timeout_secs: Some(30),
            },
            discord: DiscordConfig {
                application_id: Some("app-123".to_string()),
                guild_id: Some("guild-456".to_string()),
                channel_id: Some("chan-789".to_string()),
                channel_name: Some("general".to_string()),
                intents: None,
            },
        }
    }

    #[test]
    fn merge_agent_config_no_override_returns_global_unchanged() {
        let global = base_global_config();
        let merged = merge_agent_config(&global, None);

        assert_eq!(merged.llm.provider, global.llm.provider);
        assert_eq!(merged.llm.base_url, global.llm.base_url);
        assert_eq!(merged.llm.model, global.llm.model);
        assert_eq!(merged.llm.timeout_secs, global.llm.timeout_secs);
        assert_eq!(merged.platform.name, global.platform.name);
        assert_eq!(merged.discord.guild_id, global.discord.guild_id);
    }

    #[test]
    fn merge_agent_config_model_override_only_changes_model() {
        let global = base_global_config();
        let override_cfg = AgentOverrideConfig {
            llm: Some(LlmConfig {
                model: Some("anthropic/claude-3-haiku".to_string()),
                provider: None,
                base_url: None,
                timeout_secs: None,
            }),
            core: None,
        };

        let merged = merge_agent_config(&global, Some(override_cfg));

        // Only model should change
        assert_eq!(
            merged.llm.model,
            Some("anthropic/claude-3-haiku".to_string())
        );
        // Everything else preserved
        assert_eq!(merged.llm.provider, Some("openrouter".to_string()));
        assert_eq!(
            merged.llm.base_url,
            Some("https://openrouter.ai/api/v1".to_string())
        );
        assert_eq!(merged.llm.timeout_secs, Some(30));
        assert_eq!(merged.platform.name, Some("TestPlatform".to_string()));
    }

    #[test]
    fn merge_agent_config_partial_override_only_base_url_changes() {
        let global = base_global_config();
        let override_cfg = AgentOverrideConfig {
            llm: Some(LlmConfig {
                base_url: Some("http://localhost:11434".to_string()),
                provider: None,
                model: None,
                timeout_secs: None,
            }),
            core: None,
        };

        let merged = merge_agent_config(&global, Some(override_cfg));

        // Only base_url should change
        assert_eq!(
            merged.llm.base_url,
            Some("http://localhost:11434".to_string())
        );
        // Everything else preserved
        assert_eq!(merged.llm.provider, Some("openrouter".to_string()));
        assert_eq!(merged.llm.model, Some("openai/gpt-4o-mini".to_string()));
        assert_eq!(merged.llm.timeout_secs, Some(30));
    }

    #[test]
    fn merge_agent_config_full_llm_override_replaces_all_llm_fields() {
        let global = base_global_config();
        let override_cfg = AgentOverrideConfig {
            llm: Some(LlmConfig {
                provider: Some("ollama".to_string()),
                base_url: Some("http://localhost:11434".to_string()),
                model: Some("llama3".to_string()),
                timeout_secs: Some(60),
            }),
            core: None,
        };

        let merged = merge_agent_config(&global, Some(override_cfg));

        assert_eq!(merged.llm.provider, Some("ollama".to_string()));
        assert_eq!(
            merged.llm.base_url,
            Some("http://localhost:11434".to_string())
        );
        assert_eq!(merged.llm.model, Some("llama3".to_string()));
        assert_eq!(merged.llm.timeout_secs, Some(60));
        // Non-LLM fields still from global
        assert_eq!(merged.platform.name, Some("TestPlatform".to_string()));
        assert_eq!(merged.discord.guild_id, Some("guild-456".to_string()));
    }
}

pub async fn run_init_wizard() -> Result<GlobalConfig> {
    println!("\n=== AgentsVillage Initialization Wizard ===\n");

    // Step 3.1: Platform name
    let platform_name: String = Input::new()
        .with_prompt("Enter a name for this platform")
        .default("AgentsVillage".to_string())
        .interact_text()?;

    // Step 3.2: LLM provider
    let provider_options = vec!["OpenRouter", "Ollama"];
    let provider_idx = Select::new()
        .with_prompt("Select LLM provider")
        .items(&provider_options)
        .default(0)
        .interact()?;

    let provider = provider_options[provider_idx].to_lowercase();

    // Step 3.3: Provider-specific config
    let (base_url, model, timeout_secs) = match provider.as_str() {
        "openrouter" => {
            let base_url: String = Input::new()
                .with_prompt("OpenRouter base URL")
                .default("https://openrouter.ai/api/v1".to_string())
                .interact_text()?;
            let model: String = Input::new()
                .with_prompt("Model name")
                .default("openai/gpt-4o-mini".to_string())
                .interact_text()?;
            let timeout: u64 = Input::new()
                .with_prompt("Timeout (seconds)")
                .default(30u64)
                .interact_text()?;
            println!("\n📝 Please add the following to your .env file:");
            println!("   LLM_API_KEY=sk-or-your-openrouter-api-key\n");
            (base_url, model, timeout)
        }
        "ollama" => {
            let base_url: String = Input::new()
                .with_prompt("Ollama base URL")
                .default("http://localhost:11434".to_string())
                .interact_text()?;
            let model: String = Input::new()
                .with_prompt("Model name")
                .default("llama3".to_string())
                .interact_text()?;
            let timeout: u64 = Input::new()
                .with_prompt("Timeout (seconds)")
                .default(60u64)
                .interact_text()?;
            (base_url, model, timeout)
        }
        _ => unreachable!(),
    };

    let llm_config = LlmConfig {
        provider: Some(provider.clone()),
        base_url: Some(base_url),
        model: Some(model),
        timeout_secs: Some(timeout_secs),
    };

    // Step 3.4-3.5: Test LLM connection
    loop {
        println!("\n🔌 Testing LLM connection...");
        let _ = dotenvy::dotenv(); // re-load .env on each retry to pick up newly added keys
        let api_key = std::env::var("LLM_API_KEY").ok();
        let client = LlmClient::new(llm_config.clone(), api_key);
        match client.test_connection().await {
            Ok(()) => {
                println!("✅ LLM connection successful!");
                break;
            }
            Err(e) => {
                println!("❌ LLM connection failed: {}", e);
                let retry_options = vec!["Retry", "Skip (configure later)", "Exit"];
                let choice = Select::new()
                    .with_prompt("What would you like to do?")
                    .items(&retry_options)
                    .default(0)
                    .interact()?;
                match choice {
                    0 => continue,
                    1 => break,
                    _ => anyhow::bail!("Setup aborted by user"),
                }
            }
        }
    }

    let mut config = GlobalConfig {
        core: CoreConfig::default(),
        platform: PlatformConfig {
            name: Some(platform_name),
        },
        llm: llm_config,
        discord: DiscordConfig::default(),
    };

    // Save partial config before Discord setup
    save_global_config(&config)?;

    // Step 4: Discord setup
    println!("\n=== Discord Bot Setup ===\n");
    println!("📋 Instructions:");
    println!("  1. Go to https://discord.com/developers/applications");
    println!("  2. Click 'New Application' and give it a name");
    println!("  3. Go to the 'Bot' section");
    println!("  4. Click 'Add Bot' if not already done");
    println!("  5. Under 'Token', click 'Reset Token' and copy it\n");

    let _bot_token: String = Input::new()
        .with_prompt("Enter your Bot Token (will NOT be saved to config)")
        .interact_text()?;

    println!("\n📝 Please add the following to your .env file:");
    println!("   DISCORD_TOKEN=your-bot-token-here\n");

    println!("⚙️  Enable MESSAGE_CONTENT Privileged Gateway Intent:");
    println!("  1. In your Bot settings, scroll to 'Privileged Gateway Intents'");
    println!("  2. Enable 'MESSAGE CONTENT INTENT'");
    println!("  3. Save changes\n");

    let app_id: String = Input::new()
        .with_prompt("Enter your Application/Client ID (from 'General Information' tab)")
        .interact_text()?;

    // Generate OAuth2 URL
    // VIEW_CHANNEL=1024, SEND_MESSAGES=2048, READ_MESSAGE_HISTORY=65536,
    // CREATE_PUBLIC_THREADS=34359738368, SEND_MESSAGES_IN_THREADS=274877906944
    let permissions: u64 = 1024 + 2048 + 65536 + 34359738368 + 274877906944;
    let oauth_url = format!(
        "https://discord.com/api/oauth2/authorize?client_id={}&permissions={}&scope=bot%20applications.commands",
        app_id, permissions
    );

    println!("\n🔗 OAuth2 Bot Invite URL:");
    println!("   {}\n", oauth_url);
    println!("📋 Open the above URL in your browser to add the bot to your server.\n");

    let _confirm: String = Input::new()
        .with_prompt("Press Enter once the bot has been added to your server")
        .default("".to_string())
        .allow_empty(true)
        .interact_text()?;

    // Step 5: Channel binding
    println!("\n=== Channel Binding ===\n");

    // Load token from env
    let discord_token = std::env::var("DISCORD_TOKEN").unwrap_or_else(|_| _bot_token.clone());

    let (guild_id, channel_id, channel_name) =
        run_channel_binding_wizard(&discord_token, &app_id).await?;

    config.discord = DiscordConfig {
        application_id: Some(app_id),
        guild_id: Some(guild_id),
        channel_id: Some(channel_id),
        channel_name: Some(channel_name),
        intents: None,
    };

    save_global_config(&config)?;
    println!("\n✅ Configuration complete! You can now start the bot.\n");

    Ok(config)
}

async fn run_channel_binding_wizard(
    token: &str,
    _app_id: &str,
) -> Result<(String, String, String)> {
    let http = reqwest::Client::new();
    let base = "https://discord.com/api/v10";
    let auth = format!("Bot {}", token);

    // List guilds
    println!("🔍 Fetching your guilds...");
    let guilds_resp = http
        .get(format!("{}/users/@me/guilds", base))
        .header("Authorization", &auth)
        .send()
        .await
        .context("Failed to fetch guilds")?;

    let guilds: Vec<serde_json::Value> = guilds_resp
        .json()
        .await
        .context("Failed to parse guilds response")?;

    let guild_id = if guilds.is_empty() {
        println!("No guilds found. Please enter Guild ID manually.");
        Input::new().with_prompt("Enter Guild ID").interact_text()?
    } else {
        let guild_names: Vec<String> = guilds
            .iter()
            .map(|g| {
                format!(
                    "{} ({})",
                    g["name"].as_str().unwrap_or("Unknown"),
                    g["id"].as_str().unwrap_or("?")
                )
            })
            .collect();

        let mut options = guild_names.clone();
        options.push("Enter Guild ID manually".to_string());

        let choice = Select::new()
            .with_prompt("Select a guild")
            .items(&options)
            .default(0)
            .interact()?;

        if choice == guilds.len() {
            Input::new().with_prompt("Enter Guild ID").interact_text()?
        } else {
            guilds[choice]["id"].as_str().unwrap_or("").to_string()
        }
    };

    // List channels
    println!("\n🔍 Fetching channels for guild {}...", guild_id);
    let channels_resp = http
        .get(format!("{}/guilds/{}/channels", base, guild_id))
        .header("Authorization", &auth)
        .send()
        .await
        .context("Failed to fetch channels")?;

    let channels: Vec<serde_json::Value> = channels_resp
        .json()
        .await
        .context("Failed to parse channels response")?;

    // Filter to text channels (type 0)
    let text_channels: Vec<&serde_json::Value> = channels
        .iter()
        .filter(|c| c["type"].as_u64() == Some(0))
        .collect();

    let (channel_id, channel_name) = if text_channels.is_empty() {
        println!("No text channels found. Please enter Channel ID manually.");
        let cid: String = Input::new()
            .with_prompt("Enter Channel ID")
            .interact_text()?;
        (cid, "unknown".to_string())
    } else {
        let channel_names: Vec<String> = text_channels
            .iter()
            .map(|c| {
                format!(
                    "#{} ({})",
                    c["name"].as_str().unwrap_or("Unknown"),
                    c["id"].as_str().unwrap_or("?")
                )
            })
            .collect();

        let mut options = channel_names.clone();
        options.push("Enter Channel ID manually".to_string());

        let choice = Select::new()
            .with_prompt("Select a channel")
            .items(&options)
            .default(0)
            .interact()?;

        if choice == text_channels.len() {
            let cid: String = Input::new()
                .with_prompt("Enter Channel ID")
                .interact_text()?;
            (cid, "unknown".to_string())
        } else {
            let ch = text_channels[choice];
            (
                ch["id"].as_str().unwrap_or("").to_string(),
                ch["name"].as_str().unwrap_or("unknown").to_string(),
            )
        }
    };

    // Verify channel exists
    println!("\n🔍 Verifying channel {}...", channel_id);
    let channel_resp = http
        .get(format!("{}/channels/{}", base, channel_id))
        .header("Authorization", &auth)
        .send()
        .await
        .context("Failed to verify channel")?;

    if !channel_resp.status().is_success() {
        anyhow::bail!("Channel {} not found or not accessible", channel_id);
    }

    // Try sending typing indicator to verify write permission
    println!("🔍 Verifying write permissions...");
    let typing_resp = http
        .post(format!("{}/channels/{}/typing", base, channel_id))
        .header("Authorization", &auth)
        .header("Content-Length", "0")
        .send()
        .await
        .context("Failed to send typing indicator")?;

    if typing_resp.status().is_success() || typing_resp.status().as_u16() == 204 {
        println!("✅ Write permissions verified for #{}", channel_name);
    } else {
        println!(
            "⚠️  Could not verify write permissions (status: {}). Continuing anyway.",
            typing_resp.status()
        );
    }

    Ok((guild_id, channel_id, channel_name))
}
