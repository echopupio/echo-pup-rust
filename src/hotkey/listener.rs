//! 热键监听器 - 默认使用 global-hotkey，右 Ctrl 使用低层键盘事件监听

use anyhow::{anyhow, Result};
use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyEventReceiver, GlobalHotKeyManager};
use parking_lot::Mutex;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use rdev::{EventType, Key};
#[cfg(target_os = "macos")]
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use tracing::{error, info};

const MAX_HOTKEY_KEYS: usize = 3;

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
    FunctionKeyNoMods(Code),
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
        validate_hotkey_config(key)?;

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

        if should_use_low_level_function_key_listener(&hotkey) {
            #[cfg(target_os = "macos")]
            match macos_standard_function_keys_enabled() {
                Some(false) => {
                    info!(
                        "检测到 macOS 未开启标准功能键（fnState=0），将尝试低层监听 {:?}",
                        hotkey.key
                    );
                }
                Some(true) => {
                    info!("macOS 已开启标准功能键（fnState=1），监听 {:?}", hotkey.key);
                }
                None => {
                    info!(
                        "无法读取 macOS 标准功能键设置，将尝试低层监听 {:?}",
                        hotkey.key
                    );
                }
            }

            self.unregister_global_hotkey();
            self.hotkey = Some(hotkey);
            self.event_receiver = None;
            self.mode = ListenerMode::FunctionKeyNoMods(hotkey.key);
            return Ok(());
        }

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
            ListenerMode::FunctionKeyNoMods(code) => self.start_function_key_listener(code),
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

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn start_function_key_listener(&mut self, code: Code) -> Result<()> {
        let target_key =
            code_to_rdev_function_key(code).ok_or_else(|| anyhow!("不支持的功能键: {:?}", code))?;
        info!("热键监听模式: function-key (rdev, {:?})", code);

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
                    EventType::KeyPress(key) if key == target_key => {
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
                    EventType::KeyRelease(key) if key == target_key => {
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
                error!("function-key 热键监听失败({:?}): {:?}", code, err);
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

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    fn start_function_key_listener(&mut self, code: Code) -> Result<()> {
        let _ = code;
        Err(anyhow!(
            "当前系统不支持无修饰功能键的低层监听，请改用组合键（例如 ctrl+f1）"
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

pub fn hotkey_policy_hint() -> &'static str {
    "建议使用 right_ctrl、单独 F1-F24，或至少包含 ctrl/alt/super 的组合键（最多3键，不支持仅 Shift 组合）"
}

pub fn validate_hotkey_config(key: &str) -> Result<()> {
    let key_count = hotkey_key_count(key);
    if key_count == 0 {
        return Err(anyhow!("热键不能为空"));
    }
    if key_count > MAX_HOTKEY_KEYS {
        return Err(anyhow!(
            "热键最多支持 {} 个键，当前为 {} 个",
            MAX_HOTKEY_KEYS,
            key_count
        ));
    }

    if is_right_ctrl_alias(key) {
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            return Err(anyhow!(
                "当前系统不支持 right_ctrl 单键监听，请使用组合键（例如 ctrl+space）"
            ));
        }
        return Ok(());
    }

    let hotkey: HotKey = key.parse().map_err(|e| anyhow!("无法解析热键: {:?}", e))?;
    if !has_primary_modifier(hotkey.mods) {
        if hotkey.mods.contains(Modifiers::SHIFT) {
            return Err(anyhow!(
                "不支持仅使用 Shift 作为修饰键（例如 shift+z）。{}",
                hotkey_policy_hint()
            ));
        }
    }
    if is_hotkey_too_broad(&hotkey) {
        return Err(anyhow!(
            "热键 '{}' 会吞掉常用输入键，风险较高。{}",
            key,
            hotkey_policy_hint()
        ));
    }
    Ok(())
}

fn hotkey_key_count(key: &str) -> usize {
    key.split('+')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .count()
}

fn is_hotkey_too_broad(hotkey: &HotKey) -> bool {
    if has_primary_modifier(hotkey.mods) {
        return false;
    }

    !is_function_key(hotkey.key)
}

fn has_primary_modifier(mods: Modifiers) -> bool {
    mods.intersects(Modifiers::CONTROL | Modifiers::ALT | Modifiers::SUPER)
}

fn is_plain_f1_to_f12(hotkey: &HotKey) -> bool {
    hotkey.mods.is_empty()
        && matches!(
            hotkey.key,
            Code::F1
                | Code::F2
                | Code::F3
                | Code::F4
                | Code::F5
                | Code::F6
                | Code::F7
                | Code::F8
                | Code::F9
                | Code::F10
                | Code::F11
                | Code::F12
        )
}

fn should_use_low_level_function_key_listener(hotkey: &HotKey) -> bool {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        return is_plain_f1_to_f12(hotkey);
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = hotkey;
        false
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn code_to_rdev_function_key(code: Code) -> Option<Key> {
    match code {
        Code::F1 => Some(Key::F1),
        Code::F2 => Some(Key::F2),
        Code::F3 => Some(Key::F3),
        Code::F4 => Some(Key::F4),
        Code::F5 => Some(Key::F5),
        Code::F6 => Some(Key::F6),
        Code::F7 => Some(Key::F7),
        Code::F8 => Some(Key::F8),
        Code::F9 => Some(Key::F9),
        Code::F10 => Some(Key::F10),
        Code::F11 => Some(Key::F11),
        Code::F12 => Some(Key::F12),
        _ => None,
    }
}

#[cfg(target_os = "macos")]
fn macos_standard_function_keys_enabled() -> Option<bool> {
    let output = Command::new("defaults")
        .arg("read")
        .arg("-g")
        .arg("com.apple.keyboard.fnState")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    parse_macos_fn_state_output(&String::from_utf8_lossy(&output.stdout))
}

#[cfg(target_os = "macos")]
fn parse_macos_fn_state_output(output: &str) -> Option<bool> {
    match output.trim() {
        "1" => Some(true),
        "0" => Some(false),
        _ => None,
    }
}

fn is_function_key(code: Code) -> bool {
    matches!(
        code,
        Code::F1
            | Code::F2
            | Code::F3
            | Code::F4
            | Code::F5
            | Code::F6
            | Code::F7
            | Code::F8
            | Code::F9
            | Code::F10
            | Code::F11
            | Code::F12
            | Code::F13
            | Code::F14
            | Code::F15
            | Code::F16
            | Code::F17
            | Code::F18
            | Code::F19
            | Code::F20
            | Code::F21
            | Code::F22
            | Code::F23
            | Code::F24
    )
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
    #[cfg(target_os = "macos")]
    use super::parse_macos_fn_state_output;
    use super::{
        is_right_ctrl_alias, should_use_low_level_function_key_listener, validate_hotkey_config,
    };
    use global_hotkey::hotkey::HotKey;

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

    #[test]
    fn test_validate_hotkey_rejects_single_char() {
        let err = validate_hotkey_config("z").unwrap_err().to_string();
        assert!(err.contains("会吞掉常用输入键"));
    }

    #[test]
    fn test_validate_hotkey_accepts_modifier_combo() {
        validate_hotkey_config("ctrl+space").unwrap();
        validate_hotkey_config("ctrl+shift+a").unwrap();
    }

    #[test]
    fn test_validate_hotkey_rejects_shift_only_combo() {
        let err = validate_hotkey_config("shift+z").unwrap_err().to_string();
        assert!(err.contains("不支持仅使用 Shift"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn test_parse_macos_fn_state_output() {
        assert_eq!(parse_macos_fn_state_output("1\n"), Some(true));
        assert_eq!(parse_macos_fn_state_output("0"), Some(false));
        assert_eq!(parse_macos_fn_state_output("abc"), None);
    }

    #[test]
    fn test_validate_hotkey_accepts_function_key() {
        validate_hotkey_config("f12").unwrap();
    }

    #[test]
    fn test_validate_hotkey_limits_key_count() {
        let err = validate_hotkey_config("ctrl+alt+shift+f12")
            .unwrap_err()
            .to_string();
        assert!(err.contains("最多支持"));
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn test_plain_f1_uses_low_level_listener() {
        let hotkey: HotKey = "f1".parse().unwrap();
        assert!(should_use_low_level_function_key_listener(&hotkey));
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn test_ctrl_f1_does_not_use_low_level_listener() {
        let hotkey: HotKey = "ctrl+f1".parse().unwrap();
        assert!(!should_use_low_level_function_key_listener(&hotkey));
    }
}
