use anyhow::{Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::debug;

use crate::config::LlmConfig;

pub struct LlmClient {
    config: LlmConfig,
    api_key: Option<String>,
    http: Client,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
}

#[derive(Debug, Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
    message: ChatMessageResponse,
}

#[derive(Debug, Deserialize)]
struct ChatMessageResponse {
    content: String,
}

impl LlmClient {
    pub fn new(config: LlmConfig, api_key: Option<String>) -> Self {
        let timeout = config.timeout_secs.unwrap_or(30);
        let http = Client::builder()
            .timeout(std::time::Duration::from_secs(timeout))
            .build()
            .expect("Failed to build HTTP client");

        Self {
            config,
            api_key,
            http,
        }
    }

    pub async fn chat(&self, messages: Vec<ChatMessage>) -> Result<String> {
        let base_url = self
            .config
            .base_url
            .as_deref()
            .unwrap_or("http://localhost:11434");
        let model = self
            .config
            .model
            .as_deref()
            .unwrap_or("llama3")
            .to_string();

        let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));

        debug!("Calling LLM at {} with model {}", url, model);

        let body = ChatRequest {
            model,
            messages,
        };

        let mut request = self.http.post(&url).json(&body);

        // Add auth header for OpenRouter
        let provider = self.config.provider.as_deref().unwrap_or("ollama");
        if provider == "openrouter" {
            if let Some(key) = &self.api_key {
                request = request.header("Authorization", format!("Bearer {}", key));
            }
        }

        let response = request
            .send()
            .await
            .context("Failed to send request to LLM")?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("LLM API returned error {}: {}", status, body);
        }

        let chat_response: ChatResponse = response
            .json()
            .await
            .context("Failed to parse LLM response")?;

        let content = chat_response
            .choices
            .into_iter()
            .next()
            .map(|c| c.message.content)
            .unwrap_or_default();

        if content.is_empty() {
            anyhow::bail!("LLM returned empty response");
        }

        Ok(content)
    }

    pub async fn test_connection(&self) -> Result<()> {
        let messages = vec![ChatMessage {
            role: "user".to_string(),
            content: "Hello".to_string(),
        }];

        let response = self.chat(messages).await?;
        if response.is_empty() {
            anyhow::bail!("LLM returned empty response during test");
        }

        debug!("LLM test connection successful");
        Ok(())
    }
}
