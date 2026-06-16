use super::ProviderConfig;
use anyhow::bail;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: Vec<Message<'a>>,
    stream: bool,
    options: Options,
}

#[derive(Debug, Serialize)]
struct Message<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Debug, Serialize)]
struct Options {
    temperature: f32,
}

#[derive(Debug, Deserialize)]
struct ChatResponse {
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
    let url = format!("{}/api/chat", config.base_url);
    let request = ChatRequest {
        model: &config.model,
        messages: vec![Message {
            role: "user",
            content: prompt,
        }],
        stream: false,
        options: Options { temperature: 0.0 },
    };

    let response = http.post(url).json(&request).send().await?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        bail!("Ollama returned {status}: {body}");
    }

    let parsed: ChatResponse = response.json().await?;
    if parsed.message.content.trim().is_empty() {
        bail!("Ollama response did not contain text content");
    }
    Ok(parsed.message.content)
}

pub async fn chat_stream_stdout(
    http: &reqwest::Client,
    config: &ProviderConfig,
    prompt: &str,
) -> anyhow::Result<String> {
    let url = format!("{}/api/chat", config.base_url);
    let request = ChatRequest {
        model: &config.model,
        messages: vec![Message {
            role: "user",
            content: prompt,
        }],
        stream: true,
        options: Options { temperature: 0.0 },
    };

    let mut response = http.post(url).json(&request).send().await?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        bail!("Ollama returned {status}: {body}");
    }

    let mut pending = String::new();
    let mut printer = super::StreamPrinter::new();
    while let Some(chunk) = response.chunk().await? {
        pending.push_str(&String::from_utf8_lossy(&chunk));
        while let Some(newline) = pending.find('\n') {
            let line = pending[..newline].trim().to_string();
            pending = pending[newline + 1..].to_string();
            if line.is_empty() {
                continue;
            }
            let value: serde_json::Value = match serde_json::from_str(&line) {
                Ok(value) => value,
                Err(_) => continue,
            };
            if let Some(content) = value["message"]["content"].as_str() {
                printer.push(content)?;
            }
            if value["done"].as_bool() == Some(true) {
                return printer.finish();
            }
        }
    }

    printer.finish()
}
