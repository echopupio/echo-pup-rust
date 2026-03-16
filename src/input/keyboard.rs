//! 键盘输入模拟

use anyhow::Result;
use enigo::{Enigo, Keyboard, Settings, Direction, Key};

pub struct Keyboard {
    enigo: Enigo,
}

impl Keyboard {
    /// 创建新的键盘实例
    pub fn new() -> Result<Self> {
        let enigo = Enigo::new(&Settings::default())?;
        Ok(Self { enigo })
    }

    /// 输入文本
    pub fn type_text(&mut self, text: &str) -> Result<()> {
        self.enigo.text(text)?;
        Ok(())
    }

    /// 按下单个键
    pub fn press_key(&mut self, key: Key) -> Result<()> {
        self.enigo.key(key, Direction::Click)?;
        Ok(())
    }

    /// 按下组合键
    pub fn press_hotkey(&mut self, keys: &[Key]) -> Result<()> {
        for key in keys {
            self.enigo.key(*key, Direction::Press)?;
        }
        for key in keys.iter().rev() {
            self.enigo.key(*key, Direction::Release)?;
        }
        Ok(())
    }

    /// 模拟 Ctrl+V 粘贴
    pub fn paste(&mut self) -> Result<()> {
        self.press_hotkey(&[Key::Control, Key::Unicode('v')])?;
        Ok(())
    }
}

impl Default for Keyboard {
    fn default() -> Self {
        Self::new().unwrap()
    }
}
