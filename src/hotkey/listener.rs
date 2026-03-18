//! 热键监听器 - 默认使用 global-hotkey，右 Ctrl 使用低层键盘事件监听

use anyhow::{anyhow, Result};
use global_hotkey::hotkey::HotKey;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyEventReceiver, GlobalHotKeyManager};
use parking_lot::Mutex;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use rdev::{EventType, Key};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use tracing::{error, info};

/// 热键事件类型
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HotkeyEvent {
    Pressed,
    Released,
}

/// 热键回调类型
pub type HotkeyCallback = Arc<dyn Fn(HotkeyEvent) + Send + Sync>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ListenerMode {
    GlobalHotkey,
    RightCtrl,
}

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
    mode: ListenerMode,
}

impl HotkeyListener {
    /// 创建新的热键监听器
    pub fn new() -> Result<Self> {
        let manager =
            GlobalHotKeyManager::new().map_err(|e| anyhow!("无法创建热键管理器: {:?}", e))?;

        Ok(Self {
            hotkey: None,
            manager: Some(manager),
            callback: None,
            press_callback: None,
            release_callback: None,
            is_pressed: Arc::new(Mutex::new(false)),
            stop_sender: None,
            event_receiver: None,
            mode: ListenerMode::GlobalHotkey,
        })
    }

    /// 设置热键 (如 "F12", "Control+Shift+A", "right_ctrl")
    pub fn set_hotkey(&mut self, key: &str) -> Result<()> {
        self.stop_listener_thread();

        if is_right_ctrl_alias(key) {
            self.unregister_global_hotkey();
            self.hotkey = None;
            self.event_receiver = None;
            self.mode = ListenerMode::RightCtrl;

            #[cfg(not(any(target_os = "linux", target_os = "macos")))]
            {
                return Err(anyhow!(
                    "当前系统不支持 right_ctrl 单键监听，请使用组合键（例如 ctrl+space）"
                ));
            }

            return Ok(());
        }

        // 使用 global-hotkey 的解析功能
        let hotkey: HotKey = key.parse().map_err(|e| anyhow!("无法解析热键: {:?}", e))?;

        if let Some(ref mut manager) = self.manager {
            // 取消之前可能存在的热键
            if let Some(ref old_hotkey) = self.hotkey {
                let _ = manager.unregister(*old_hotkey);
            }

            // 注册新热键
            manager.register(hotkey)?;
        }

        self.hotkey = Some(hotkey);
        self.mode = ListenerMode::GlobalHotkey;

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
        self.stop_listener_thread();

        match self.mode {
            ListenerMode::GlobalHotkey => self.start_global_listener(),
            ListenerMode::RightCtrl => self.start_right_ctrl_listener(),
        }
    }

    fn start_global_listener(&mut self) -> Result<()> {
        info!("热键监听模式: global-hotkey");

        let is_pressed = self.is_pressed.clone();
        let event_callback = self.callback.clone();
        let press_callback = self.press_callback.clone();
        let release_callback = self.release_callback.clone();

        if self.event_receiver.is_none() {
            return Ok(());
        }

        let receiver = self.event_receiver.take().unwrap();
        let (stop_tx, stop_rx) = channel();
        self.stop_sender = Some(stop_tx);

        let hotkey_id = self.hotkey.map(|h| h.id).unwrap_or(0);

        thread::spawn(move || loop {
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
                            if let Some(ref cb) = event_callback {
                                cb(HotkeyEvent::Pressed);
                            }
                            if let Some(ref cb) = press_callback {
                                cb();
                            }
                        }
                        global_hotkey::HotKeyState::Released => {
                            *is_pressed.lock() = false;
                            if let Some(ref cb) = event_callback {
                                cb(HotkeyEvent::Released);
                            }
                            if let Some(ref cb) = release_callback {
                                cb();
                            }
                        }
                    }
                }
            }
        });

        Ok(())
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn start_right_ctrl_listener(&mut self) -> Result<()> {
        info!("热键监听模式: right_ctrl (rdev)");

        let is_pressed = self.is_pressed.clone();
        let event_callback = self.callback.clone();
        let press_callback = self.press_callback.clone();
        let release_callback = self.release_callback.clone();

        let (stop_tx, stop_rx) = channel();
        self.stop_sender = Some(stop_tx);

        let should_stop = Arc::new(AtomicBool::new(false));

        {
            let should_stop = should_stop.clone();
            thread::spawn(move || {
                let _ = stop_rx.recv();
                should_stop.store(true, Ordering::SeqCst);
            });
        }

        thread::spawn(move || {
            let result = rdev::listen(move |event| {
                if should_stop.load(Ordering::SeqCst) {
                    return;
                }

                match event.event_type {
                    EventType::KeyPress(key) if is_right_ctrl_event_key(key) => {
                        let mut pressed = is_pressed.lock();
                        if !*pressed {
                            *pressed = true;
                            drop(pressed);
                            if let Some(ref cb) = event_callback {
                                cb(HotkeyEvent::Pressed);
                            }
                            if let Some(ref cb) = press_callback {
                                cb();
                            }
                        }
                    }
                    EventType::KeyRelease(key) if is_right_ctrl_event_key(key) => {
                        let mut pressed = is_pressed.lock();
                        if *pressed {
                            *pressed = false;
                            drop(pressed);
                            if let Some(ref cb) = event_callback {
                                cb(HotkeyEvent::Released);
                            }
                            if let Some(ref cb) = release_callback {
                                cb();
                            }
                        }
                    }
                    _ => {}
                }
            });

            if let Err(err) = result {
                error!("right_ctrl 热键监听失败: {:?}", err);
            }
        });

        Ok(())
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    fn start_right_ctrl_listener(&mut self) -> Result<()> {
        Err(anyhow!(
            "当前系统不支持 right_ctrl 单键监听，请使用组合键（例如 ctrl+space）"
        ))
    }

    /// 检查热键是否被按下
    pub fn is_pressed(&self) -> bool {
        *self.is_pressed.lock()
    }

    /// 停止监听
    pub fn stop(&mut self) -> Result<()> {
        self.stop_listener_thread();
        self.unregister_global_hotkey();
        Ok(())
    }

    fn stop_listener_thread(&mut self) {
        if let Some(sender) = self.stop_sender.take() {
            let _ = sender.send(());
        }
        *self.is_pressed.lock() = false;
    }

    fn unregister_global_hotkey(&mut self) {
        if let Some(ref mut manager) = self.manager {
            if let Some(ref hotkey) = self.hotkey {
                let _ = manager.unregister(*hotkey);
            }
        }
    }
}

fn normalize_key_name(key: &str) -> String {
    key.to_ascii_lowercase()
        .chars()
        .filter(|c| *c != ' ' && *c != '_' && *c != '-')
        .collect()
}

fn is_right_ctrl_alias(key: &str) -> bool {
    matches!(
        normalize_key_name(key).as_str(),
        "rightctrl" | "rctrl" | "ctrlright" | "controlright" | "rightcontrol"
    )
}

#[cfg(all(any(target_os = "linux", target_os = "macos"), target_os = "macos"))]
fn is_right_ctrl_event_key(key: Key) -> bool {
    matches!(key, Key::ControlRight | Key::ControlLeft)
}

#[cfg(all(
    any(target_os = "linux", target_os = "macos"),
    not(target_os = "macos")
))]
fn is_right_ctrl_event_key(key: Key) -> bool {
    matches!(key, Key::ControlRight)
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

#[cfg(test)]
mod tests {
    use super::is_right_ctrl_alias;

    #[test]
    fn test_right_ctrl_aliases() {
        assert!(is_right_ctrl_alias("right_ctrl"));
        assert!(is_right_ctrl_alias("right-ctrl"));
        assert!(is_right_ctrl_alias("RightCtrl"));
        assert!(is_right_ctrl_alias("control_right"));
        assert!(is_right_ctrl_alias("rctrl"));
        assert!(!is_right_ctrl_alias("ctrl+space"));
        assert!(!is_right_ctrl_alias("f12"));
    }
}
