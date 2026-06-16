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

    let response = http.post(url).json(&request).send().await?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        bail!("LM Studio returned {status}: {body}");
    }

    let parsed: ChatResponse = response.json().await?;
    parsed
        .choices
        .into_iter()
        .next()
        .map(|choice| choice.message.content)
        .filter(|content| !content.trim().is_empty())
        .context("LM Studio response did not contain text content")
}

pub async fn chat_stream_stdout(
    http: &reqwest::Client,
    config: &ProviderConfig,
    prompt: &str,
) -> anyhow::Result<String> {
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

    let mut response = http.post(url).json(&request).send().await?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        bail!("LM Studio returned {status}: {body}");
    }

    read_openai_compatible_stream(&mut response).await
}

pub(crate) async fn read_openai_compatible_stream(
    response: &mut reqwest::Response,
) -> anyhow::Result<String> {
    let mut pending = String::new();
    let mut printer = super::StreamPrinter::new();

    while let Some(chunk) = response.chunk().await? {
        pending.push_str(&String::from_utf8_lossy(&chunk));
        while let Some(newline) = pending.find('\n') {
            let line = pending[..newline].trim().to_string();
            pending = pending[newline + 1..].to_string();

            let Some(data) = line.strip_prefix("data:").map(str::trim) else {
                continue;
            };
            if data == "[DONE]" {
                return printer.finish();
            }

            let value: serde_json::Value = match serde_json::from_str(data) {
                Ok(value) => value,
                Err(_) => continue,
            };
            let Some(content) = value["choices"][0]["delta"]["content"].as_str() else {
                continue;
            };
            printer.push(content)?;
        }
    }

    printer.finish()
}
