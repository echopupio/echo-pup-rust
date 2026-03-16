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

/// API 请求
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

/// API 响应
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

impl LLMRewrite {
    /// 创建新的 LLM 整理器
    pub fn new(provider: &str, api_base: &str, api_key_env: &str, model: &str) -> Result<Self> {
        let api_key = env::var(api_key_env)
            .unwrap_or_else(|_| {
                tracing::warn!("环境变量 {} 未设置", api_key_env);
                String::new()
            });

        let enabled = !api_key.is_empty();

        if !enabled {
            tracing::warn!("LLM API Key 未配置，文本整理功能将不可用");
        } else {
            tracing::info!("LLM 整理已启用，使用模型: {}", model);
        }

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

        let request = ChatRequest {
            model: self.model.clone(),
            messages: vec![Message {
                role: "user".to_string(),
                content: prompt,
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
            tracing::error!("LLM API 错误: {} - {}", status, text);
            return Err(anyhow::anyhow!("LLM API 错误: {}", status));
        }

        let chat_response: ChatResponse = response
            .json()
            .context("解析 LLM 响应失败")?;

        let result = chat_response
            .choices
            .first()
            .map(|c| c.message.content.clone())
            .unwrap_or_else(|| text.to_string());

        tracing::info!("LLM 整理完成");
        Ok(result)
    }

    /// 检查是否启用
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }
}
