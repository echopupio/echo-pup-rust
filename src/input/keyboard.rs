//! 键盘输入模拟

use anyhow::Result;
use enigo::Enigo;

pub struct Keyboard {
    enigo: Enigo,
}

impl Keyboard {
    /// 创建新的键盘实例
    pub fn new() -> Result<Self> {
        let enigo = Enigo::new(&enigo::Settings::default())?;
        Ok(Self { enigo })
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
