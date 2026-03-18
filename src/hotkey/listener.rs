//! 热键监听器 - 使用 global-hotkey 实现

use anyhow::Result;
use global_hotkey::hotkey::HotKey;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyEventReceiver, GlobalHotKeyManager};
use parking_lot::Mutex;
use std::sync::mpsc::{channel, Sender};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

/// 热键事件类型
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HotkeyEvent {
    Pressed,
    Released,
}

/// 热键回调类型
pub type HotkeyCallback = Arc<dyn Fn(HotkeyEvent) + Send + Sync>;

/// 热键监听器
pub struct HotkeyListener {
    hotkey: Option<HotKey>,
    manager: Option<GlobalHotKeyManager>,
    callback: Option<HotkeyCallback>,
    press_callback: Option<Arc<dyn Fn() + Send + Sync>>,
    release_callback: Option<Arc<dyn Fn() + Send + Sync>>,
    is_pressed: Arc<Mutex<bool>>,
    stop_sender: Option<Sender<()>>,
    event_receiver: Option<GlobalHotKeyEventReceiver>,
}

impl HotkeyListener {
    /// 创建新的热键监听器
    pub fn new() -> Result<Self> {
        let manager = GlobalHotKeyManager::new()
            .map_err(|e| anyhow::anyhow!("无法创建热键管理器: {:?}", e))?;

        Ok(Self {
            hotkey: None,
            manager: Some(manager),
            callback: None,
            press_callback: None,
            release_callback: None,
            is_pressed: Arc::new(Mutex::new(false)),
            stop_sender: None,
            event_receiver: None,
        })
    }

    /// 设置热键 (如 "F12", "Control+Shift+A")
    pub fn set_hotkey(&mut self, key: &str) -> Result<()> {
        // 使用 global-hotkey 的解析功能
        let hotkey: HotKey = key
            .parse()
            .map_err(|e| anyhow::anyhow!("无法解析热键: {:?}", e))?;

        if let Some(ref mut manager) = self.manager {
            // 取消之前可能存在的热键
            if let Some(ref old_hotkey) = self.hotkey {
                let _ = manager.unregister(*old_hotkey);
            }

            // 注册新热键
            manager.register(hotkey)?;
        }

        self.hotkey = Some(hotkey);

        // 获取事件接收器 (需要克隆)
        let receiver = GlobalHotKeyEvent::receiver();
        self.event_receiver = Some(receiver.clone());

        Ok(())
    }

    /// 设置回调函数
    pub fn on_event(&mut self, callback: HotkeyCallback) {
        self.callback = Some(callback);
    }

    /// 设置按下回调 (兼容旧接口)
    pub fn on_press(&mut self, callback: Arc<dyn Fn() + Send + Sync>) {
        self.press_callback = Some(callback);
    }

    /// 设置松开回调 (兼容旧接口)
    pub fn on_release(&mut self, callback: Arc<dyn Fn() + Send + Sync>) {
        self.release_callback = Some(callback);
    }

    /// 开始监听热键事件
    pub fn start(&mut self) -> Result<()> {
        // 启动事件处理线程
        let is_pressed = self.is_pressed.clone();
        let press_callback = self.press_callback.clone();
        let release_callback = self.release_callback.clone();

        if self.event_receiver.is_none() {
            return Ok(());
        }

        let receiver = self.event_receiver.take().unwrap();
        let (stop_tx, stop_rx) = channel();
        self.stop_sender = Some(stop_tx);

        let hotkey_id = self.hotkey.map(|h| h.id).unwrap_or(0);

        thread::spawn(move || {
            loop {
                // 检查是否需要停止
                if stop_rx.try_recv().is_ok() {
                    break;
                }

                // 接收热键事件
                if let Ok(event) = receiver.recv_timeout(Duration::from_millis(100)) {
                    if event.id == hotkey_id {
                        match event.state {
                            global_hotkey::HotKeyState::Pressed => {
                                *is_pressed.lock() = true;
                                if let Some(ref cb) = press_callback {
                                    cb();
                                }
                            }
                            global_hotkey::HotKeyState::Released => {
                                *is_pressed.lock() = false;
                                if let Some(ref cb) = release_callback {
                                    cb();
                                }
                            }
                        }
                    }
                }
            }
        });

        Ok(())
    }

    /// 检查热键是否被按下
    pub fn is_pressed(&self) -> bool {
        *self.is_pressed.lock()
    }

    /// 停止监听
    pub fn stop(&mut self) -> Result<()> {
        if let Some(ref mut manager) = self.manager {
            if let Some(ref hotkey) = self.hotkey {
                let _ = manager.unregister(*hotkey);
            }
        }

        if let Some(sender) = self.stop_sender.take() {
            let _ = sender.send(());
        }

        Ok(())
    }
}

impl Default for HotkeyListener {
    fn default() -> Self {
        Self::new().unwrap()
    }
}

impl Drop for HotkeyListener {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}
