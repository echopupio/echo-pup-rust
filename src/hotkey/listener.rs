//! 热键监听器

use anyhow::Result;
use global_hotkey::{GlobalHotKey, HotKeyState, Manager};

pub struct HotkeyListener {
    hotkey: GlobalHotKey,
}

impl HotkeyListener {
    pub fn new(key: &str) -> Result<Self> {
        // TODO: 解析按键字符串
        todo!()
    }

    pub fn start<F>(&self, on_press: F, on_release: F) -> Result<()>
    where
        F: Fn() + Send + 'static,
    {
        todo!()
    }

    pub fn stop(&self) -> Result<()> {
        todo!()
    }
}
