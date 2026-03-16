//! 热键监听器 - 简化实现

use anyhow::Result;
use std::sync::Arc;
use parking_lot::Mutex;

/// 热键回调类型
pub type HotkeyCallback = Arc<dyn Fn() + Send + Sync>;

/// 热键监听器
pub struct HotkeyListener {
    press_callback: Option<HotkeyCallback>,
    release_callback: Option<HotkeyCallback>,
    is_pressed: Arc<Mutex<bool>>,
}

impl HotkeyListener {
    /// 创建新的热键监听器
    pub fn new() -> Result<Self> {
        Ok(Self {
            press_callback: None,
            release_callback: None,
            is_pressed: Arc::new(Mutex::new(false)),
        })
    }

    /// 设置热键
    pub fn set_hotkey(&mut self, key: &str) -> Result<()> {
        tracing::info!("热键已设置为: {} (模拟模式)", key);
        Ok(())
    }

    /// 设置按下回调
    pub fn on_press(&mut self, callback: HotkeyCallback) {
        self.press_callback = Some(callback);
    }

    /// 设置松开回调
    pub fn on_release(&mut self, callback: HotkeyCallback) {
        self.release_callback = Some(callback);
    }

    /// 开始监听
    pub fn start(&mut self) -> Result<()> {
        tracing::info!("热键监听已启动 (模拟模式)");
        
        // 触发按下回调
        if let Some(ref cb) = self.press_callback {
            cb();
        }
        
        Ok(())
    }

    /// 检查热键是否被按下
    pub fn is_pressed(&self) -> bool {
        *self.is_pressed.lock()
    }

    /// 停止监听
    pub fn stop(&mut self) -> Result<()> {
        tracing::info!("热键监听已停止");
        Ok(())
    }
}

impl Default for HotkeyListener {
    fn default() -> Self {
        Self::new().unwrap()
    }
}
