//! 键盘输入模拟

use anyhow::Result;
use enigo::Enigo;
use tracing::{error, info, warn};

pub struct Keyboard {
    enigo: Enigo,
}

impl Keyboard {
    /// 创建新的键盘实例（带重试机制）
    pub fn new() -> Result<Self> {
        let max_retries = 3;
        let mut last_error = None;

        for attempt in 1..=max_retries {
            match Enigo::new(&enigo::Settings::default()) {
                Ok(enigo) => {
                    info!("键盘输入初始化成功 (尝试 {}/{})", attempt, max_retries);
                    return Ok(Self { enigo });
                }
                Err(e) => {
                    last_error = Some(e);
                    warn!(
                        "键盘输入初始化失败 (尝试 {}/{}): {}",
                        attempt, max_retries, e
                    );
                    if attempt < max_retries {
                        std::thread::sleep(std::time::Duration::from_millis(500));
                    }
                }
            }
        }

        error!("键盘输入初始化最终失败: {:?}", last_error);
        Err(anyhow::anyhow!("键盘初始化失败: {:?}", last_error))
    }

    /// 输入文本
    pub fn type_text(&mut self, text: &str) -> Result<()> {
        use enigo::Keyboard;
        self.enigo.text(text)?;
        Ok(())
    }
}

impl Default for Keyboard {
    fn default() -> Self {
        Self::new().unwrap()
    }
}
