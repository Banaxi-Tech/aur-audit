use super::ProviderConfig;
use anyhow::{bail, Context};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: Vec<Message<'a>>,
    temperature: f32,
    stream: bool,
}

#[derive(Debug, Serialize)]
struct Message<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Debug, Deserialize)]
struct ChatResponse {
    choices: Vec<Choice>,
}

#[derive(Debug, Deserialize)]
struct Choice {
    message: ResponseMessage,
}

#[derive(Debug, Deserialize)]
struct ResponseMessage {
    content: String,
}

pub async fn chat(
    http: &reqwest::Client,
    config: &ProviderConfig,
    prompt: &str,
) -> anyhow::Result<String> {
    let api_key = config
        .api_key
        .clone()
        .context("OPENROUTER_API_KEY or config api_key is required for --provider openrouter")?;
    let url = format!("{}/chat/completions", config.base_url);
    let request = ChatRequest {
        model: &config.model,
        messages: vec![Message {
            role: "user",
            content: prompt,
        }],
        temperature: 0.0,
        stream: false,
    };

    let response = http
        .post(url)
        .bearer_auth(api_key)
        .header("HTTP-Referer", "https://github.com/local/aur-audit")
        .header("X-Title", "aur-audit")
        .json(&request)
        .send()
        .await?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        bail!("OpenRouter returned {status}: {body}");
    }

    let parsed: ChatResponse = response.json().await?;
    parsed
        .choices
        .into_iter()
        .next()
        .map(|choice| choice.message.content)
        .filter(|content| !content.trim().is_empty())
        .context("OpenRouter response did not contain text content")
}

pub async fn chat_stream_stdout(
    http: &reqwest::Client,
    config: &ProviderConfig,
    prompt: &str,
) -> anyhow::Result<String> {
    let api_key = config
        .api_key
        .clone()
        .context("OPENROUTER_API_KEY or config api_key is required for --provider openrouter")?;
    let url = format!("{}/chat/completions", config.base_url);
    let request = ChatRequest {
        model: &config.model,
        messages: vec![Message {
            role: "user",
            content: prompt,
        }],
        temperature: 0.0,
        stream: true,
    };

    let mut response = http
        .post(url)
        .bearer_auth(api_key)
        .header("HTTP-Referer", "https://github.com/local/aur-audit")
        .header("X-Title", "aur-audit")
        .json(&request)
        .send()
        .await?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        bail!("OpenRouter returned {status}: {body}");
    }

    super::lmstudio::read_openai_compatible_stream(&mut response).await
}
