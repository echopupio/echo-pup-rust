//! LLM 文本整理

use anyhow::{Context, Result};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use tracing::{info, warn};

/// 请求超时（秒）
const REQUEST_TIMEOUT_SECS: u64 = 4;

/// 连续失败多少次后暂停 LLM 调用
const PAUSE_AFTER_FAILURES: u32 = 3;

/// 暂停后每隔多少秒探测一次（尝试恢复）
const PROBE_INTERVAL_SECS: u64 = 60;

/// 系统提示词
const SYSTEM_PROMPT: &str = "你是语音转写文本的校对助手。请仅修正以下语音识别文本中的错误，不要改变原意、不要添加内容、不要解释。\n\n规则：\n1. 修正同音字和近音字错误\n2. 补充必要的标点符号\n3. 只输出修正后的文本";

/// LLM 文本整理
pub struct LLMRewrite {
    client: Client,
    provider: String,
    model: String,
    api_base: String,
    api_key: String,
    enabled: bool,
    consecutive_failures: u32,
    last_probe_time: Option<Instant>,
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

/// Anthropic Messages API 响应
#[derive(Deserialize)]
struct AnthropicResponse {
    content: Vec<AnthropicContent>,
}

/// Anthropic 内容块
#[derive(Deserialize)]
struct AnthropicContent {
    text: String,
}

impl LLMRewrite {
    /// 创建新的 LLM 整理器
    pub fn new(provider: &str, api_base: &str, api_key: &str, model: &str) -> Result<Self> {
        let api_key = api_key.to_string();
        let provider = provider.to_string();

        // Ollama 是本地服务，不需要 API Key 也算启用
        let enabled = provider == "ollama" || !api_key.is_empty();

        let client = Client::builder()
            .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
            .build()
            .context("创建 HTTP 客户端失败")?;

        Ok(Self {
            client,
            provider,
            model: model.to_string(),
            api_base: api_base.to_string(),
            api_key,
            enabled,
            consecutive_failures: 0,
            last_probe_time: None,
        })
    }

    /// 整理/润色文本
    pub fn rewrite(&mut self, text: &str) -> Result<String> {
        if !self.enabled || text.is_empty() {
            return Ok(text.to_string());
        }

        // 连续失败达到阈值后暂停，定期探测恢复
        if self.consecutive_failures >= PAUSE_AFTER_FAILURES {
            let should_probe = match self.last_probe_time {
                Some(t) => t.elapsed() >= Duration::from_secs(PROBE_INTERVAL_SECS),
                None => true,
            };
            if !should_probe {
                return Ok(text.to_string());
            }
            info!("LLM 已暂停（连续失败 {} 次），尝试探测恢复...", self.consecutive_failures);
            self.last_probe_time = Some(Instant::now());
        }

        let start = Instant::now();
        let result = match self.provider.as_str() {
            "ollama" => self.rewrite_ollama(text),
            "anthropic" => self.rewrite_anthropic(text),
            _ => self.rewrite_openai(text),
        };

        match result {
            Ok(rewritten) => {
                if self.consecutive_failures > 0 {
                    info!("LLM 已恢复（之前连续失败 {} 次）", self.consecutive_failures);
                }
                self.consecutive_failures = 0;
                self.last_probe_time = None;
                info!("LLM 整理耗时: {}ms", start.elapsed().as_millis());
                Ok(rewritten)
            }
            Err(e) => {
                self.consecutive_failures += 1;
                let elapsed = start.elapsed();
                if elapsed >= Duration::from_secs(REQUEST_TIMEOUT_SECS) {
                    warn!(
                        "LLM 整理超时 ({}ms)，降级到原始文本 (连续失败 {}/{})",
                        elapsed.as_millis(), self.consecutive_failures, PAUSE_AFTER_FAILURES
                    );
                } else {
                    warn!(
                        "LLM 整理失败: {}，降级到原始文本 (连续失败 {}/{})",
                        e, self.consecutive_failures, PAUSE_AFTER_FAILURES
                    );
                }
                if self.consecutive_failures == PAUSE_AFTER_FAILURES {
                    warn!(
                        "LLM 连续失败 {} 次，暂停调用（每 {}s 探测一次恢复）",
                        PAUSE_AFTER_FAILURES, PROBE_INTERVAL_SECS
                    );
                }
                Ok(text.to_string())
            }
        }
    }

    /// 使用 OpenAI API 整理文本
    fn rewrite_openai(&self, text: &str) -> Result<String> {
        let request = ChatRequest {
            model: self.model.clone(),
            messages: vec![
                Message {
                    role: "system".to_string(),
                    content: SYSTEM_PROMPT.to_string(),
                },
                Message {
                    role: "user".to_string(),
                    content: text.to_string(),
                },
            ],
            temperature: 0.3,
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

        let chat_response: ChatResponse = response.json().context("解析 LLM 响应失败")?;

        let result = chat_response
            .choices
            .first()
            .map(|c| c.message.content.clone())
            .unwrap_or_else(|| text.to_string());

        Ok(result)
    }

    /// 使用 Ollama API 整理文本
    fn rewrite_ollama(&self, text: &str) -> Result<String> {
        let prompt = format!("{}\n\n{}", SYSTEM_PROMPT, text);
        let request = OllamaRequest {
            model: self.model.clone(),
            prompt,
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

        let ollama_response: OllamaResponse = response.json().context("解析 Ollama 响应失败")?;

        Ok(ollama_response.response)
    }

    /// 使用 Anthropic Messages API 整理文本
    fn rewrite_anthropic(&self, text: &str) -> Result<String> {
        let url = format!("{}/messages", self.api_base);

        let body = serde_json::json!({
            "model": self.model,
            "max_tokens": 2048,
            "system": SYSTEM_PROMPT,
            "messages": [{"role": "user", "content": text}]
        });

        let response = self
            .client
            .post(&url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .context("Anthropic API 请求失败")?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().unwrap_or_default();
            return Err(anyhow::anyhow!("Anthropic API 错误: {} - {}", status, text));
        }

        let resp: AnthropicResponse = response.json().context("解析 Anthropic 响应失败")?;

        let result = resp
            .content
            .first()
            .map(|c| c.text.clone())
            .unwrap_or_else(|| text.to_string());

        Ok(result)
    }

    /// 检查是否启用
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }
}
