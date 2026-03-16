//! 热键监听器

use anyhow::{Context, Result};
use global_hotkey::{GlobalHotKey, HotKeyManager, HotKeyState, Modifier, PressedHotKey};
use std::sync::Arc;
use std::time::Duration;
use parking_lot::Mutex;

/// 热键回调类型
pub type HotkeyCallback = Arc<dyn Fn() + Send + Sync>;

/// 热键监听器
pub struct HotkeyListener {
    manager: HotKeyManager,
    current_hotkey: Option<GlobalHotKey>,
    press_callback: Option<HotkeyCallback>,
    release_callback: Option<HotkeyCallback>,
    is_pressed: Arc<Mutex<bool>>,
}

impl HotkeyListener {
    /// 创建新的热键监听器
    pub fn new() -> Result<Self> {
        Ok(Self {
            manager: HotKeyManager::new()?,
            current_hotkey: None,
            press_callback: None,
            release_callback: None,
            is_pressed: Arc::new(Mutex::new(false)),
        })
    }

    /// 设置热键
    pub fn set_hotkey(&mut self, key: &str) -> Result<()> {
        let hotkey = parse_key(key).context("无法解析热键")?;
        self.current_hotkey = Some(hotkey);
        tracing::info!("热键已设置为: {}", key);
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
        let hotkey = self.current_hotkey
            .context("未设置热键")?;

        let press_callback = self.press_callback.clone();
        let release_callback = self.release_callback.clone();
        let is_pressed = self.is_pressed.clone();

        // 注册热键
        self.manager.register(hotkey, move |event| {
            match event.state {
                HotKeyState::Pressed => {
                    if let Some(ref cb) = press_callback {
                        *is_pressed.lock() = true;
                        cb();
                    }
                }
                HotKeyState::Released => {
                    if let Some(ref cb) = release_callback {
                        *is_pressed.lock() = false;
                        cb();
                    }
                }
            }
        })?;

        tracing::info!("热键监听已启动");
        Ok(())
    }

    /// 检查热键是否被按下
    pub fn is_pressed(&self) -> bool {
        *self.is_pressed.lock()
    }

    /// 停止监听
    pub fn stop(&mut self) -> Result<()> {
        if let Some(hotkey) = &self.current_hotkey {
            let _ = self.manager.unregister(hotkey);
        }
        tracing::info!("热键监听已停止");
        Ok(())
    }
}

impl Default for HotkeyListener {
    fn default() -> Self {
        Self::new().unwrap()
    }
}

/// 解析按键字符串
fn parse_key(key: &str) -> Result<GlobalHotKey> {
    let key = key.to_uppercase();
    let mut modifiers = Vec::new();
    let mut key_code = None;

    // 解析修饰键
    for modifier in &["CTRL+", "ALT+", "SHIFT+", "META+", "CMD+", "WIN+"] {
        if key.contains(modifier) {
            let m = match *modifier {
                "CTRL+" => Modifier::CONTROL,
                "ALT+" => Modifier::ALT,
                "SHIFT+" => Modifier::SHIFT,
                "META+" | "WIN+" => Modifier::META,
                "CMD+" => Modifier::META,
                _ => continue,
            };
            modifiers.push(m);
        }
    }

    // 解析主按键
    let key_part = key
        .replace("CTRL+", "")
        .replace("ALT+", "")
        .replace("SHIFT+", "")
        .replace("META+", "")
        .replace("CMD+", "")
        .replace("WIN+", "");

    key_code = Some(match key_part.as_str() {
        "F1" => global_hotkey::KeyCode::F1,
        "F2" => global_hotkey::KeyCode::F2,
        "F3" => global_hotkey::KeyCode::F3,
        "F4" => global_hotkey::KeyCode::F4,
        "F5" => global_hotkey::KeyCode::F5,
        "F6" => global_hotkey::KeyCode::F6,
        "F7" => global_hotkey::KeyCode::F7,
        "F8" => global_hotkey::KeyCode::F8,
        "F9" => global_hotkey::KeyCode::F9,
        "F10" => global_hotkey::KeyCode::F10,
        "F11" => global_hotkey::KeyCode::F11,
        "F12" => global_hotkey::KeyCode::F12,
        "A" => global_hotkey::KeyCode::KeyA,
        "B" => global_hotkey::KeyCode::KeyB,
        "C" => global_hotkey::KeyCode::KeyC,
        "D" => global_hotkey::KeyCode::KeyD,
        "E" => global_hotkey::KeyCode::KeyE,
        "F" => global_hotkey::KeyCode::KeyF,
        "G" => global_hotkey::KeyCode::KeyG,
        "H" => global_hotkey::KeyCode::KeyH,
        "I" => global_hotkey::KeyCode::KeyI,
        "J" => global_hotkey::KeyCode::KeyJ,
        "K" => global_hotkey::KeyCode::KeyK,
        "L" => global_hotkey::KeyCode::KeyL,
        "M" => global_hotkey::KeyCode::KeyM,
        "N" => global_hotkey::KeyCode::KeyN,
        "O" => global_hotkey::KeyCode::KeyO,
        "P" => global_hotkey::KeyCode::KeyP,
        "Q" => global_hotkey::KeyCode::KeyQ,
        "R" => global_hotkey::KeyCode::KeyR,
        "S" => global_hotkey::KeyCode::KeyS,
        "T" => global_hotkey::KeyCode::KeyT,
        "U" => global_hotkey::KeyCode::KeyU,
        "V" => global_hotkey::KeyCode::KeyV,
        "W" => global_hotkey::KeyCode::KeyW,
        "X" => global_hotkey::KeyCode::KeyX,
        "Y" => global_hotkey::KeyCode::KeyY,
        "Z" => global_hotkey::KeyCode::KeyZ,
        "SPACE" => global_hotkey::KeyCode::Space,
        "ENTER" | "RETURN" => global_hotkey::KeyCode::Return,
        "TAB" => global_hotkey::KeyCode::Tab,
        "ESCAPE" | "ESC" => global_hotkey::KeyCode::Escape,
        _ => return Err(anyhow::anyhow!("未知按键: {}", key_part)),
    });

    let modifiers: Vec<Modifier> = modifiers;
    Ok(GlobalHotKey::new(key_code.unwrap(), modifiers.as_slice()))
}
