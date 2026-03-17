//! 键盘输入模拟

use anyhow::Result;
use enigo::Enigo;
use tracing;

pub struct Keyboard {
    enigo: Enigo,
}

impl Keyboard {
    /// 创建新的键盘实例
    pub fn new() -> Result<Self> {
        let enigo = Enigo::new(&enigo::Settings::default())?;
        tracing::info!("[Keyboard] 键盘输入已初始化");
        Ok(Self { enigo })
    }

    /// 输入文本
    pub fn type_text(&mut self, text: &str) -> Result<()> {
        tracing::info!("[Keyboard] 准备输入文本，长度: {}", text.len());
        tracing::debug!("[Keyboard] 输入内容: {}", text);
        
        use enigo::Keyboard;
        self.enigo.text(text)?;
        
        tracing::info!("[Keyboard] 文本已输入");
        Ok(())
    }
}

impl Default for Keyboard {
    fn default() -> Self {
        Self::new().unwrap()
    }
}
