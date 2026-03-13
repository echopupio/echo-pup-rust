//! 键盘输入模拟

use anyhow::Result;
use enigo::Enigo;

pub struct Keyboard {
    enigo: Enigo,
}

impl Keyboard {
    pub fn new() -> Result<Self> {
        Ok(Self {
            enigo: Enigo::new(&enigo::Settings::default())?,
        })
    }

    pub fn type_text(&mut self, text: &str) -> Result<()> {
        self.enigo.text(text)?;
        Ok(())
    }
}
