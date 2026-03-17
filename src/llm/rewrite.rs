//! LLM 文本整理

use anyhow::{Context, Result};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use std::env;

/// LLM 文本整理
pub struct LLMRewrite {
    client: Client,
    provider: String,
    model: String,
    api_base: String,
    api_key: String,
    enabled: bool,
}

/// OpenAI API 请求
#[derive(Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<Message>,
    temperature: f32,
}

/// 消息
#[derive(Serialize)]
struct Message {
    role: String,
    content: String,
}

/// OpenAI API 响应
#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<Choice>,
}

/// 选择
#[derive(Deserialize)]
struct Choice {
    message: ResponseMessage,
}

/// 响应消息
#[derive(Deserialize)]
struct ResponseMessage {
    content: String,
}

/// Ollama API 请求
#[derive(Serialize)]
struct OllamaRequest {
    model: String,
    prompt: String,
    stream: bool,
}

/// Ollama 流式响应
#[derive(Deserialize)]
struct OllamaResponse {
    response: String,
}

impl LLMRewrite {
    /// 创建新的 LLM 整理器
    pub fn new(provider: &str, api_base: &str, api_key_env: &str, model: &str) -> Result<Self> {
        let api_key = env::var(api_key_env)
            .unwrap_or_else(|_| String::new());

        let enabled = !api_key.is_empty();

        Ok(Self {
            client: Client::new(),
            provider: provider.to_string(),
            model: model.to_string(),
            api_base: api_base.to_string(),
            api_key,
            enabled,
        })
    }

    /// 整理/润色文本
    pub fn rewrite(&self, text: &str) -> Result<String> {
        if !self.enabled || text.is_empty() {
            return Ok(text.to_string());
        }

        let prompt = format!(
            "请将以下语音转写的文本进行整理和润色：\n1. 修正明显的识别错误\n2. 添加适当的标点符号\n3. 使语句更通顺自然\n\n原始文本：\n{}",
            text
        );

        match self.provider.as_str() {
            "ollama" => self.rewrite_ollama(&prompt),
            _ => self.rewrite_openai(&prompt),
        }
    }

    /// 使用 OpenAI API 整理文本
    fn rewrite_openai(&self, prompt: &str) -> Result<String> {
        let request = ChatRequest {
            model: self.model.clone(),
            messages: vec![Message {
                role: "user".to_string(),
                content: prompt.to_string(),
            }],
            temperature: 0.7,
        };

        let url = format!("{}/chat/completions", self.api_base);

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .context("LLM API 请求失败")?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().unwrap_or_default();
            return Err(anyhow::anyhow!("LLM API 错误: {} - {}", status, text));
        }

        let chat_response: ChatResponse = response
            .json()
            .context("解析 LLM 响应失败")?;

        let result = chat_response
            .choices
            .first()
            .map(|c| c.message.content.clone())
            .unwrap_or_else(|| prompt.to_string());

        Ok(result)
    }

    /// 使用 Ollama API 整理文本（流式）
    fn rewrite_ollama(&self, prompt: &str) -> Result<String> {
        let request = OllamaRequest {
            model: self.model.clone(),
            prompt: prompt.to_string(),
            stream: false,
        };

        let url = format!("{}/api/generate", self.api_base);

        let response = self
            .client
            .post(&url)
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .context("Ollama API 请求失败")?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().unwrap_or_default();
            return Err(anyhow::anyhow!("Ollama API 错误: {} - {}", status, text));
        }

        let ollama_response: OllamaResponse = response
            .json()
            .context("解析 Ollama 响应失败")?;

        Ok(ollama_response.response)
    }

    /// 检查是否启用
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }
}
