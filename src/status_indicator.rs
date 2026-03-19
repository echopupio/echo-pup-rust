#![allow(unexpected_cfgs)]
//! 状态栏反馈与菜单 IPC（macOS）

use anyhow::Result;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::config::HotkeyTriggerMode;
use crate::menu_core::{
    whisper_model_path_from_file_name, EditableField, MenuAction, MenuActionResult, MenuSnapshot,
    DOWNLOAD_MODEL_SIZES, WHISPER_MODEL_FILES,
};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IndicatorState {
    Idle,
    RecordingStart,
    Recording,
    Transcribing,
    Completed,
    Failed,
}

impl IndicatorState {
    fn from_wire(input: &str) -> Option<Self> {
        match input.trim() {
            "idle" => Some(Self::Idle),
            "recording_start" => Some(Self::RecordingStart),
            "recording" => Some(Self::Recording),
            "transcribing" => Some(Self::Transcribing),
            "completed" => Some(Self::Completed),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }

    fn menu_title(self) -> &'static str {
        let _ = self;
        ""
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum ParentMessage {
    SetState { state: IndicatorState },
    SetSnapshot { snapshot: MenuSnapshot },
    SetActionResult { result: MenuActionResult },
    Exit,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum ChildMessage {
    ActionRequest { action: MenuAction },
}

#[cfg(target_os = "macos")]
const STATUS_LOGO_PNG: &[u8] = include_bytes!("../assets/logo.png");
#[cfg(target_os = "macos")]
const STATUS_MICROPHONE_PNG: &[u8] = include_bytes!("../assets/mic.png");
#[cfg(target_os = "macos")]
const STATUS_ITEM_LENGTH_IDLE: f64 = 16.0;
#[cfg(target_os = "macos")]
const STATUS_ITEM_LENGTH_ACTIVE: f64 = 40.0;
#[cfg(target_os = "macos")]
const STATUS_ICON_SIZE: f64 = 28.0;
#[cfg(target_os = "macos")]
const STATUS_LOGO_VISUAL_SCALE: f64 = 2.0;
#[cfg(target_os = "macos")]
const STATUS_MIC_VISUAL_SCALE: f64 = 0.92;
#[cfg(target_os = "macos")]
const STATUS_ICON_H_INSET: f64 = 0.0;
#[cfg(target_os = "macos")]
const STATUS_ICON_V_INSET: f64 = 1.0;

#[cfg(target_os = "macos")]
fn status_item_length_for_state(state: IndicatorState) -> f64 {
    match state {
        IndicatorState::Idle => STATUS_ITEM_LENGTH_IDLE,
        _ => STATUS_ITEM_LENGTH_ACTIVE,
    }
}

#[cfg(target_os = "macos")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VisualStyle {
    Idle,
    RecordingPulse,
    TranscribingPulse,
    CompletedSolid,
    FailedSolid,
}

#[cfg(target_os = "macos")]
impl VisualStyle {
    fn is_pulsing(self) -> bool {
        matches!(self, Self::RecordingPulse | Self::TranscribingPulse)
    }
}

#[cfg(target_os = "macos")]
impl IndicatorState {
    fn visual_style(self) -> VisualStyle {
        match self {
            Self::Idle => VisualStyle::Idle,
            Self::RecordingStart | Self::Recording => VisualStyle::RecordingPulse,
            Self::Transcribing => VisualStyle::TranscribingPulse,
            Self::Completed => VisualStyle::CompletedSolid,
            Self::Failed => VisualStyle::FailedSolid,
        }
    }

    fn auto_reset_duration(self) -> Option<std::time::Duration> {
        match self {
            Self::Completed => Some(std::time::Duration::from_millis(1500)),
            Self::Failed => Some(std::time::Duration::from_secs(2)),
            _ => None,
        }
    }
}

#[derive(Debug)]
pub struct StatusIndicatorClient {
    enabled: bool,

    #[cfg(target_os = "macos")]
    child: Option<std::process::Child>,
    #[cfg(target_os = "macos")]
    stdin: Option<std::process::ChildStdin>,
    #[cfg(target_os = "macos")]
    action_rx: Option<std::sync::mpsc::Receiver<MenuAction>>,
}

impl StatusIndicatorClient {
    pub fn start(enabled: bool, config_path: &str) -> Self {
        #[cfg(target_os = "macos")]
        {
            if !enabled {
                return Self {
                    enabled: false,
                    child: None,
                    stdin: None,
                    action_rx: None,
                };
            }

            let exe = match std::env::current_exe() {
                Ok(path) => path,
                Err(err) => {
                    warn!("无法获取可执行文件路径，状态栏反馈已禁用: {}", err);
                    return Self {
                        enabled: false,
                        child: None,
                        stdin: None,
                        action_rx: None,
                    };
                }
            };

            let mut child = match std::process::Command::new(exe)
                .arg("--config")
                .arg(config_path)
                .arg("status-indicator")
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::null())
                .spawn()
            {
                Ok(child) => child,
                Err(err) => {
                    warn!("启动状态栏子进程失败，状态栏反馈已禁用: {}", err);
                    return Self {
                        enabled: false,
                        child: None,
                        stdin: None,
                        action_rx: None,
                    };
                }
            };

            let stdin = match child.stdin.take() {
                Some(stdin) => stdin,
                None => {
                    let _ = child.kill();
                    let _ = child.wait();
                    warn!("状态栏子进程未提供 stdin，状态栏反馈已禁用");
                    return Self {
                        enabled: false,
                        child: None,
                        stdin: None,
                        action_rx: None,
                    };
                }
            };

            let stdout = match child.stdout.take() {
                Some(stdout) => stdout,
                None => {
                    let _ = child.kill();
                    let _ = child.wait();
                    warn!("状态栏子进程未提供 stdout，状态栏反馈已禁用");
                    return Self {
                        enabled: false,
                        child: None,
                        stdin: None,
                        action_rx: None,
                    };
                }
            };

            let (tx, rx) = std::sync::mpsc::channel::<MenuAction>();
            std::thread::spawn(move || {
                use std::io::{BufRead, BufReader};
                let reader = BufReader::new(stdout);
                for line in reader.lines() {
                    let Ok(line) = line else {
                        break;
                    };
                    let parsed = serde_json::from_str::<ChildMessage>(&line);
                    if let Ok(ChildMessage::ActionRequest { action }) = parsed {
                        let _ = tx.send(action);
                    }
                }
            });

            Self {
                enabled: true,
                child: Some(child),
                stdin: Some(stdin),
                action_rx: Some(rx),
            }
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = (enabled, config_path);
            Self { enabled: false }
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn send(&mut self, state: IndicatorState) {
        if !self.enabled {
            return;
        }
        let _ = self.send_message(&ParentMessage::SetState { state });
    }

    pub fn send_snapshot(&mut self, snapshot: &MenuSnapshot) {
        if !self.enabled {
            return;
        }
        let _ = self.send_message(&ParentMessage::SetSnapshot {
            snapshot: snapshot.clone(),
        });
    }

    pub fn send_action_result(&mut self, result: &MenuActionResult) {
        if !self.enabled {
            return;
        }
        let _ = self.send_message(&ParentMessage::SetActionResult {
            result: result.clone(),
        });
    }

    pub fn try_recv_action(&mut self) -> Option<MenuAction> {
        if !self.enabled {
            return None;
        }

        #[cfg(target_os = "macos")]
        {
            let Some(rx) = self.action_rx.as_ref() else {
                return None;
            };
            return rx.try_recv().ok();
        }

        #[cfg(not(target_os = "macos"))]
        {
            None
        }
    }

    pub fn close_ui(&mut self) {
        if !self.enabled {
            return;
        }

        let _ = self.send_message(&ParentMessage::Exit);

        #[cfg(target_os = "macos")]
        {
            self.stdin.take();
            if let Some(mut child) = self.child.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
            self.action_rx = None;
        }

        self.enabled = false;
    }

    fn send_message(&mut self, message: &ParentMessage) -> Result<()> {
        if !self.enabled {
            return Ok(());
        }

        #[cfg(target_os = "macos")]
        {
            let Some(stdin) = self.stdin.as_mut() else {
                self.enabled = false;
                return Ok(());
            };

            let payload = serde_json::to_string(message)?;
            if std::io::Write::write_all(stdin, payload.as_bytes())
                .and_then(|_| std::io::Write::write_all(stdin, b"\n"))
                .and_then(|_| std::io::Write::flush(stdin))
                .is_err()
            {
                self.enabled = false;
                warn!("向状态栏子进程发送消息失败，状态栏反馈已禁用");
            }
        }

        Ok(())
    }
}

impl Drop for StatusIndicatorClient {
    fn drop(&mut self) {
        self.close_ui();
    }
}

#[cfg(target_os = "macos")]
enum IndicatorCommand {
    Message(ParentMessage),
    Exit,
}

#[cfg(target_os = "macos")]
fn parse_indicator_command(line: &str) -> Option<IndicatorCommand> {
    if let Ok(msg) = serde_json::from_str::<ParentMessage>(line) {
        return Some(IndicatorCommand::Message(msg));
    }

    let trimmed = line.trim();
    if trimmed.eq_ignore_ascii_case("exit") {
        return Some(IndicatorCommand::Exit);
    }
    IndicatorState::from_wire(trimmed)
        .map(|state| IndicatorCommand::Message(ParentMessage::SetState { state }))
}

#[cfg(target_os = "macos")]
fn spawn_stdin_reader() -> std::sync::mpsc::Receiver<IndicatorCommand> {
    use std::io::{BufRead, BufReader};
    let (tx, rx) = std::sync::mpsc::channel::<IndicatorCommand>();
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        let reader = BufReader::new(stdin.lock());
        for line in reader.lines() {
            match line {
                Ok(line) => {
                    if let Some(cmd) = parse_indicator_command(&line) {
                        let _ = tx.send(cmd);
                    }
                }
                Err(_) => break,
            }
        }
        let _ = tx.send(IndicatorCommand::Exit);
    });
    rx
}

#[cfg(target_os = "macos")]
fn send_child_message(message: &ChildMessage) {
    let serialized = match serde_json::to_string(message) {
        Ok(v) => v,
        Err(err) => {
            warn!("状态栏消息序列化失败: {}", err);
            return;
        }
    };

    let mut out = std::io::stdout();
    if std::io::Write::write_all(&mut out, serialized.as_bytes())
        .and_then(|_| std::io::Write::write_all(&mut out, b"\n"))
        .and_then(|_| std::io::Write::flush(&mut out))
        .is_err()
    {
        warn!("状态栏向主进程发送消息失败");
    }
}

#[cfg(target_os = "macos")]
unsafe fn set_status_title(button: cocoa::base::id, title: &str) {
    use cocoa::base::nil;
    use cocoa::foundation::NSString;
    use objc::{msg_send, sel, sel_impl};

    if button == nil {
        return;
    }
    let ns_title = NSString::alloc(nil).init_str(title);
    let _: () = msg_send![button, setTitle: ns_title];
}

#[cfg(target_os = "macos")]
unsafe fn load_png_image(png: &[u8]) -> cocoa::base::id {
    use cocoa::base::id;
    use objc::{class, msg_send, sel, sel_impl};
    use std::ffi::c_void;

    let data: id = msg_send![
        class!(NSData),
        dataWithBytes: png.as_ptr() as *const c_void
        length: png.len()
    ];
    let image: id = msg_send![class!(NSImage), alloc];
    let image: id = msg_send![image, initWithData: data];
    if image != cocoa::base::nil {
        let _: () = msg_send![image, setTemplate: cocoa::base::NO];
    }
    image
}

#[cfg(target_os = "macos")]
unsafe fn compose_status_image(
    logo: cocoa::base::id,
    mic: cocoa::base::id,
    show_mic: bool,
) -> cocoa::base::id {
    use cocoa::base::{id, nil};
    use cocoa::foundation::{NSPoint, NSRect, NSSize};
    use objc::{class, msg_send, sel, sel_impl};

    if logo == nil {
        return nil;
    }

    let width = STATUS_ICON_SIZE;
    let canvas_size = NSSize::new(width, STATUS_ICON_SIZE);
    let canvas: id = msg_send![class!(NSImage), alloc];
    let canvas: id = msg_send![canvas, initWithSize: canvas_size];
    if canvas == nil {
        return nil;
    }
    let _: () = msg_send![canvas, setTemplate: cocoa::base::NO];
    let _: () = msg_send![canvas, lockFocus];

    let zero_rect = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(0.0, 0.0));
    let (image, visual_scale) = if show_mic && mic != nil {
        (mic, STATUS_MIC_VISUAL_SCALE)
    } else {
        (logo, STATUS_LOGO_VISUAL_SCALE)
    };

    let clip_rect = NSRect::new(
        NSPoint::new(STATUS_ICON_H_INSET, STATUS_ICON_V_INSET),
        NSSize::new(
            (STATUS_ICON_SIZE - STATUS_ICON_H_INSET * 2.0).max(1.0),
            (STATUS_ICON_SIZE - STATUS_ICON_V_INSET * 2.0).max(1.0),
        ),
    );
    let clip_radius = (clip_rect.size.height * 0.36).max(4.0);
    let clip_path: id = msg_send![
        class!(NSBezierPath),
        bezierPathWithRoundedRect: clip_rect
        xRadius: clip_radius
        yRadius: clip_radius
    ];
    let _: () = msg_send![clip_path, addClip];

    let src_size: NSSize = msg_send![image, size];
    let src_w = if src_size.width > 0.0 {
        src_size.width
    } else {
        STATUS_ICON_SIZE
    };
    let src_h = if src_size.height > 0.0 {
        src_size.height
    } else {
        STATUS_ICON_SIZE
    };
    let scale = (clip_rect.size.width / src_w).min(clip_rect.size.height / src_h) * visual_scale;
    let draw_w = src_w * scale;
    let draw_h = src_h * scale;
    let draw_rect = NSRect::new(
        NSPoint::new(
            clip_rect.origin.x + (clip_rect.size.width - draw_w) * 0.5,
            clip_rect.origin.y + (clip_rect.size.height - draw_h) * 0.5,
        ),
        NSSize::new(draw_w, draw_h),
    );
    let _: () = msg_send![
        image,
        drawInRect: draw_rect
        fromRect: zero_rect
        operation: 2isize
        fraction: 1.0f64
    ];

    let _: () = msg_send![canvas, unlockFocus];
    canvas
}

#[cfg(target_os = "macos")]
unsafe fn set_status_image(button: cocoa::base::id, image: cocoa::base::id) {
    use cocoa::base::{nil, YES};
    use objc::{msg_send, sel, sel_impl};

    if button == nil || image == nil {
        return;
    }

    let _: () = msg_send![button, setImage: image];
    let _: () = msg_send![button, setImageScaling: 2isize];
    let _: () = msg_send![button, setImagePosition: 1isize];
    let _: () = msg_send![button, setImageHugsTitle: YES];
}

#[cfg(target_os = "macos")]
unsafe fn create_background_layer(button: cocoa::base::id) -> cocoa::base::id {
    use cocoa::base::{id, nil, YES};
    use objc::{class, msg_send, sel, sel_impl};

    if button == nil {
        return nil;
    }
    let _: () = msg_send![button, setWantsLayer: YES];
    let root_layer: id = msg_send![button, layer];
    if root_layer == nil {
        return nil;
    }
    let layer: id = msg_send![class!(CALayer), layer];
    if layer == nil {
        return nil;
    }
    let _: () = msg_send![layer, setMasksToBounds: YES];
    let _: () = msg_send![root_layer, insertSublayer: layer atIndex: 0u64];
    layer
}

#[cfg(target_os = "macos")]
#[derive(Debug, Clone, Copy)]
struct LayerStyle {
    fill: Option<(f32, f32, f32, f32)>,
    border: Option<(f32, f32, f32, f32)>,
    border_width: f64,
    corner_scale: f64,
}

#[cfg(target_os = "macos")]
fn pulse_wave(phase: f32) -> f32 {
    0.5 + 0.5 * phase.sin()
}

#[cfg(target_os = "macos")]
fn pulsing_border_style(base: (f32, f32, f32), phase: f32) -> LayerStyle {
    let wave = pulse_wave(phase);
    let factor = 0.88 + 0.18 * wave;
    let scale = |v: f32| (v * factor).clamp(0.0, 1.0);
    LayerStyle {
        fill: None,
        border: Some((
            scale(base.0),
            scale(base.1),
            scale(base.2),
            0.34 + 0.46 * wave,
        )),
        border_width: 1.2 + 1.1 * wave as f64,
        corner_scale: 0.95 + 0.08 * wave as f64,
    }
}

#[cfg(target_os = "macos")]
fn style_layer(style: VisualStyle, phase: f32) -> LayerStyle {
    match style {
        VisualStyle::Idle => LayerStyle {
            fill: None,
            border: None,
            border_width: 0.0,
            corner_scale: 1.0,
        },
        VisualStyle::RecordingPulse => pulsing_border_style((0.96, 0.33, 0.18), phase),
        VisualStyle::TranscribingPulse => pulsing_border_style((0.98, 0.66, 0.15), phase),
        VisualStyle::CompletedSolid => LayerStyle {
            fill: Some((0.22, 0.72, 0.36, 0.18)),
            border: Some((0.22, 0.72, 0.36, 0.72)),
            border_width: 1.3,
            corner_scale: 1.0,
        },
        VisualStyle::FailedSolid => LayerStyle {
            fill: Some((0.90, 0.42, 0.20, 0.18)),
            border: Some((0.90, 0.42, 0.20, 0.72)),
            border_width: 1.3,
            corner_scale: 1.0,
        },
    }
}

#[cfg(target_os = "macos")]
unsafe fn apply_button_style(
    button: cocoa::base::id,
    background_layer: cocoa::base::id,
    state: IndicatorState,
    phase: f32,
) {
    use cocoa::base::{id, nil, YES};
    use cocoa::foundation::{NSPoint, NSRect, NSSize};
    use objc::{class, msg_send, sel, sel_impl};

    if button == nil || background_layer == nil {
        return;
    }
    let bounds: NSRect = msg_send![button, bounds];
    let visual = state.visual_style();
    let hpad = if matches!(
        visual,
        VisualStyle::RecordingPulse | VisualStyle::TranscribingPulse
    ) {
        0.4f64
    } else if matches!(visual, VisualStyle::Idle) {
        2.6f64
    } else {
        1.6f64
    };
    let vpad = 4.2f64;
    let width = (bounds.size.width - hpad * 2.0).max(0.0);
    let height = (bounds.size.height - vpad * 2.0).max(0.0);
    let frame = NSRect::new(NSPoint::new(hpad, vpad), NSSize::new(width, height));
    let _: () = msg_send![background_layer, setFrame: frame];
    let _: () = msg_send![background_layer, setMasksToBounds: YES];

    let style = style_layer(state.visual_style(), phase);
    let corner_radius = (height * 0.5 * style.corner_scale).max(3.0);
    let _: () = msg_send![background_layer, setCornerRadius: corner_radius];

    let (fill_r, fill_g, fill_b, fill_a) = style.fill.unwrap_or((0.0, 0.0, 0.0, 0.0));
    let fill_ns_color: id = msg_send![
        class!(NSColor),
        colorWithCalibratedRed: fill_r as f64
        green: fill_g as f64
        blue: fill_b as f64
        alpha: fill_a as f64
    ];
    if fill_ns_color != nil {
        let fill_cg_color: id = msg_send![fill_ns_color, CGColor];
        let _: () = msg_send![background_layer, setBackgroundColor: fill_cg_color];
    }

    if let Some((r, g, b, a)) = style.border {
        let border_ns_color: id = msg_send![
            class!(NSColor),
            colorWithCalibratedRed: r as f64
            green: g as f64
            blue: b as f64
            alpha: a as f64
        ];
        if border_ns_color != nil {
            let border_cg_color: id = msg_send![border_ns_color, CGColor];
            let _: () = msg_send![background_layer, setBorderColor: border_cg_color];
            let _: () = msg_send![background_layer, setBorderWidth: style.border_width];
        }
    } else {
        let _: () = msg_send![background_layer, setBorderWidth: 0.0f64];
    }

    let visible = style.fill.is_some() || style.border.is_some();
    let _: () = msg_send![background_layer, setHidden: if visible { cocoa::base::NO } else { YES }];
}

#[cfg(target_os = "macos")]
mod menu_bridge {
    use super::*;
    use cocoa::appkit::{
        NSApp, NSBackingStoreType, NSEvent, NSEventModifierFlags, NSEventType, NSWindow,
        NSWindowStyleMask,
    };
    use cocoa::base::{id, nil, NO, YES};
    use cocoa::foundation::{NSPoint, NSRect, NSSize, NSString};
    use objc::declare::ClassDecl;
    use objc::runtime::{Class, Object, Sel};
    use objc::{class, msg_send, sel, sel_impl};
    use std::ffi::CStr;
    use std::sync::{Mutex, Once, OnceLock};

    pub const TAG_TOGGLE_LLM: i64 = 1001;
    pub const TAG_TOGGLE_CORRECTION: i64 = 1002;
    pub const TAG_TOGGLE_VAD: i64 = 1003;
    pub const TAG_EDIT_HOTKEY: i64 = 1004;
    pub const TAG_EDIT_LLM_FORM: i64 = 1005;
    pub const TAG_SWITCH_WHISPER_MODEL: i64 = 1006;
    pub const TAG_DOWNLOAD_MODEL: i64 = 1007;
    pub const TAG_QUIT_UI: i64 = 1008;
    pub const TAG_OPEN_CONFIG_FOLDER: i64 = 1009;
    pub const TAG_RELOAD_CONFIG: i64 = 1010;
    pub const TAG_VIEW_MODEL_FILES: i64 = 1011;
    pub const TAG_SHOW_ABOUT: i64 = 1012;
    pub const TAG_MODE_HOLD: i64 = 1013;
    pub const TAG_MODE_PRESS_TOGGLE: i64 = 1014;
    pub const TAG_HOTKEY_UNDO: i64 = 2001;
    pub const TAG_HOTKEY_CONFIRM: i64 = 2002;
    pub const TAG_HOTKEY_CANCEL: i64 = 2003;
    pub const TAG_LLM_FORM_CONFIRM: i64 = 2101;
    pub const TAG_LLM_FORM_CANCEL: i64 = 2102;
    pub const TAG_DOWNLOAD_DIALOG_CONFIRM: i64 = 3001;
    pub const TAG_DOWNLOAD_DIALOG_START: i64 = 3002;
    pub const TAG_DOWNLOAD_DIALOG_CANCEL: i64 = 3003;

    static MENU_EVENT_TX: OnceLock<Mutex<Option<std::sync::mpsc::Sender<i64>>>> = OnceLock::new();

    pub struct MenuHandles {
        pub target: id,
        pub toggle_llm: id,
        pub toggle_correction: id,
        pub toggle_vad: id,
        pub edit_hotkey: id,
        pub mode_hold: id,
        pub mode_press_toggle: id,
        pub edit_llm_form: id,
        pub switch_whisper_model: id,
        pub download_model: id,
        pub status_line: id,
    }

    #[derive(Debug, Clone, Copy)]
    pub struct HotkeyDialog {
        pub window: id,
        pub value_label: id,
        pub hint_label: id,
        pub undo_button: id,
        pub confirm_button: id,
    }

    #[derive(Debug, Clone, Copy)]
    pub struct DownloadDialog {
        pub window: id,
        pub model_selector: id,
        pub start_button: id,
        pub status_label: id,
        pub progress_label: id,
        pub progress_bar: id,
        pub log_labels: [id; 6],
        pub confirm_button: id,
        pub cancel_button: id,
    }

    #[derive(Debug, Clone, Copy)]
    pub struct LlmFormDialog {
        pub window: id,
        pub provider_input: id,
        pub model_input: id,
        pub api_base_input: id,
        pub api_key_env_input: id,
    }

    extern "C" fn on_menu_item(this: &Object, _cmd: Sel, sender: id) {
        let _ = this;
        unsafe {
            let tag: isize = msg_send![sender, tag];
            if let Some(lock) = MENU_EVENT_TX.get() {
                if let Ok(guard) = lock.lock() {
                    if let Some(tx) = guard.as_ref() {
                        let _ = tx.send(tag as i64);
                    }
                }
            }
        }
    }

    fn target_class() -> &'static Class {
        static ONCE: Once = Once::new();
        static mut CLASS: *const Class = std::ptr::null();

        ONCE.call_once(|| unsafe {
            let superclass = class!(NSObject);
            let mut decl = ClassDecl::new("EchoPupMenuTarget", superclass).unwrap();
            decl.add_method(
                sel!(onMenuItem:),
                on_menu_item as extern "C" fn(&Object, Sel, id),
            );
            CLASS = decl.register();
        });

        unsafe { &*CLASS }
    }

    pub fn create_menu(tx: std::sync::mpsc::Sender<i64>) -> (id, MenuHandles) {
        MENU_EVENT_TX.get_or_init(|| Mutex::new(None));
        if let Some(lock) = MENU_EVENT_TX.get() {
            if let Ok(mut guard) = lock.lock() {
                *guard = Some(tx);
            }
        }

        unsafe {
            let target: id = msg_send![target_class(), new];
            let menu: id = msg_send![class!(NSMenu), alloc];
            let menu: id = msg_send![menu, initWithTitle: nsstring("EchoPup")];

            let status_line = add_info_item(menu, "状态: 启动中");

            add_separator(menu);

            let toggle_llm = add_action_item(menu, target, TAG_TOGGLE_LLM, "切换 LLM 开关");
            let toggle_correction =
                add_action_item(menu, target, TAG_TOGGLE_CORRECTION, "切换文本纠错开关");
            let toggle_vad = add_action_item(menu, target, TAG_TOGGLE_VAD, "切换 VAD 开关");

            add_separator(menu);

            let edit_hotkey =
                add_action_item(menu, target, TAG_EDIT_HOTKEY, "编辑热键（按键捕获）");
            let mode_submenu = add_submenu(menu, "录音触发模式");
            let mode_hold = add_action_item(
                mode_submenu,
                target,
                TAG_MODE_HOLD,
                "长按模式（按住 1 秒开始，松开结束）",
            );
            let mode_press_toggle = add_action_item(
                mode_submenu,
                target,
                TAG_MODE_PRESS_TOGGLE,
                "按压切换模式（按住 1 秒开始，再按结束）",
            );
            let edit_llm_form =
                add_action_item(menu, target, TAG_EDIT_LLM_FORM, "编辑 LLM 配置...");
            let switch_whisper_model = add_action_item(
                menu,
                target,
                TAG_SWITCH_WHISPER_MODEL,
                "切换 Whisper 模型...",
            );

            add_separator(menu);

            let download_model = add_action_item(menu, target, TAG_DOWNLOAD_MODEL, "下载模型...");

            let config_submenu = add_submenu(menu, "配置");
            let _open_cfg = add_action_item(
                config_submenu,
                target,
                TAG_OPEN_CONFIG_FOLDER,
                "打开配置文件夹",
            );
            let _reload_cfg =
                add_action_item(config_submenu, target, TAG_RELOAD_CONFIG, "重载配置文件");
            let _view_models =
                add_action_item(config_submenu, target, TAG_VIEW_MODEL_FILES, "查看模型文件");

            add_separator(menu);

            let _about = add_action_item(menu, target, TAG_SHOW_ABOUT, "关于 EchoPup");
            let _quit_ui = add_action_item(menu, target, TAG_QUIT_UI, "退出");

            let handles = MenuHandles {
                target,
                toggle_llm,
                toggle_correction,
                toggle_vad,
                edit_hotkey,
                mode_hold,
                mode_press_toggle,
                edit_llm_form,
                switch_whisper_model,
                download_model,
                status_line,
            };

            (menu, handles)
        }
    }

    pub fn is_hotkey_capture_tag(tag: i64) -> bool {
        tag == TAG_EDIT_HOTKEY
    }

    pub fn map_tag_to_action(tag: i64, snapshot: &MenuSnapshot) -> Option<MenuAction> {
        match tag {
            TAG_TOGGLE_LLM => Some(MenuAction::ToggleLlmEnabled),
            TAG_TOGGLE_CORRECTION => Some(MenuAction::ToggleTextCorrectionEnabled),
            TAG_TOGGLE_VAD => Some(MenuAction::ToggleVadEnabled),
            TAG_MODE_HOLD => Some(MenuAction::SetHotkeyTriggerMode {
                mode: HotkeyTriggerMode::HoldToRecord,
            }),
            TAG_MODE_PRESS_TOGGLE => Some(MenuAction::SetHotkeyTriggerMode {
                mode: HotkeyTriggerMode::PressToToggle,
            }),
            TAG_EDIT_LLM_FORM => None,
            TAG_SWITCH_WHISPER_MODEL => prompt_switch_whisper_model(snapshot),
            TAG_DOWNLOAD_MODEL => None,
            TAG_RELOAD_CONFIG => Some(MenuAction::ReloadConfig),
            TAG_QUIT_UI => Some(MenuAction::QuitUi),
            _ => None,
        }
    }

    pub fn update_menu(handles: &MenuHandles, snapshot: &MenuSnapshot) {
        unsafe {
            set_check_state(handles.toggle_llm, snapshot.llm_enabled);
            set_check_state(handles.toggle_correction, snapshot.text_correction_enabled);
            set_check_state(handles.toggle_vad, snapshot.vad_enabled);
            set_check_state(
                handles.mode_hold,
                snapshot.hotkey_trigger_mode == HotkeyTriggerMode::HoldToRecord,
            );
            set_check_state(
                handles.mode_press_toggle,
                snapshot.hotkey_trigger_mode == HotkeyTriggerMode::PressToToggle,
            );

            set_title(
                handles.edit_hotkey,
                &format!("编辑热键 ({})", snapshot.hotkey),
            );
            set_title(
                handles.edit_llm_form,
                &format!(
                    "编辑 LLM 配置 ({}/{})",
                    snapshot.llm_provider, snapshot.llm_model
                ),
            );
            set_title(
                handles.switch_whisper_model,
                &format!(
                    "切换 Whisper 模型 ({})",
                    shorten_path(&current_whisper_model_name(snapshot))
                ),
            );
            set_title(handles.download_model, &download_menu_title(snapshot));

            set_title(handles.status_line, &format!("状态: {}", snapshot.status));
        }
    }

    pub enum HotkeyCaptureResult {
        Captured(String),
        Cancelled,
        Ignored,
    }

    pub unsafe fn capture_hotkey_from_event(event: id) -> HotkeyCaptureResult {
        let event_type = event.eventType();
        match event_type {
            NSEventType::NSFlagsChanged => {
                let key_code = event.keyCode();
                // 59: left control, 62: right control
                if key_code == 59 || key_code == 62 {
                    return HotkeyCaptureResult::Captured("ctrl".to_string());
                }
                HotkeyCaptureResult::Ignored
            }
            NSEventType::NSKeyDown => {
                let key_code = event.keyCode();
                if key_code == 53 {
                    return HotkeyCaptureResult::Cancelled;
                }

                let mut parts: Vec<String> = modifier_parts(event.modifierFlags());
                let Some(key_name) = key_name_from_event(event, key_code) else {
                    return HotkeyCaptureResult::Ignored;
                };
                parts.push(key_name);
                let hotkey = parts.join("+");
                if crate::hotkey::validate_hotkey_config(&hotkey).is_ok() {
                    HotkeyCaptureResult::Captured(hotkey)
                } else {
                    HotkeyCaptureResult::Ignored
                }
            }
            _ => HotkeyCaptureResult::Ignored,
        }
    }

    pub unsafe fn show_window(window: id) {
        if window == nil {
            return;
        }
        let app = NSApp();
        let _: () = msg_send![app, activateIgnoringOtherApps: YES];
        let _: () = msg_send![window, makeKeyAndOrderFront: nil];
    }

    pub unsafe fn create_hotkey_dialog(target: id, current_hotkey: &str) -> Option<HotkeyDialog> {
        let window = NSWindow::alloc(nil).initWithContentRect_styleMask_backing_defer_(
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(430.0, 190.0)),
            NSWindowStyleMask::NSTitledWindowMask,
            NSBackingStoreType::NSBackingStoreBuffered,
            NO,
        );
        if window == nil {
            return None;
        }
        let _: () = msg_send![window, setReleasedWhenClosed: YES];
        let _: () = msg_send![window, setTitle: nsstring("编辑热键")];
        let _: () = msg_send![window, center];

        let content: id = msg_send![window, contentView];
        if content == nil {
            return None;
        }

        let _title = add_label(
            content,
            NSRect::new(NSPoint::new(20.0, 142.0), NSSize::new(390.0, 24.0)),
            "请直接按下目标热键组合",
        );
        let value_label = add_label(
            content,
            NSRect::new(NSPoint::new(20.0, 106.0), NSSize::new(390.0, 24.0)),
            &format!("当前按键: {}", current_hotkey),
        );
        let hint_label = add_label(
            content,
            NSRect::new(NSPoint::new(20.0, 84.0), NSSize::new(390.0, 18.0)),
            "Esc 或取消可退出，支持撤销后确认",
        );

        let undo_button = add_button(
            content,
            target,
            TAG_HOTKEY_UNDO,
            "撤销",
            NSRect::new(NSPoint::new(110.0, 28.0), NSSize::new(80.0, 28.0)),
        );
        let confirm_button = add_button(
            content,
            target,
            TAG_HOTKEY_CONFIRM,
            "确认",
            NSRect::new(NSPoint::new(200.0, 28.0), NSSize::new(80.0, 28.0)),
        );
        let _cancel_button = add_button(
            content,
            target,
            TAG_HOTKEY_CANCEL,
            "取消",
            NSRect::new(NSPoint::new(290.0, 28.0), NSSize::new(80.0, 28.0)),
        );

        set_enabled(undo_button, false);
        set_enabled(confirm_button, false);
        show_window(window);

        Some(HotkeyDialog {
            window,
            value_label,
            hint_label,
            undo_button,
            confirm_button,
        })
    }

    pub unsafe fn update_hotkey_dialog(
        dialog: &HotkeyDialog,
        captured_hotkey: &str,
        hint: &str,
        can_undo: bool,
        can_confirm: bool,
    ) {
        set_text(
            dialog.value_label,
            &format!("当前按键: {}", captured_hotkey.trim()),
        );
        set_text(dialog.hint_label, hint);
        set_enabled(dialog.undo_button, can_undo);
        set_enabled(dialog.confirm_button, can_confirm);
    }

    pub unsafe fn close_hotkey_dialog(dialog: HotkeyDialog) {
        if dialog.window == nil {
            return;
        }
        let _: () = msg_send![dialog.window, orderOut: nil];
        let _: () = msg_send![dialog.window, close];
    }

    pub unsafe fn create_llm_form_dialog(
        target: id,
        snapshot: &MenuSnapshot,
    ) -> Option<LlmFormDialog> {
        let window = NSWindow::alloc(nil).initWithContentRect_styleMask_backing_defer_(
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(620.0, 280.0)),
            NSWindowStyleMask::NSTitledWindowMask,
            NSBackingStoreType::NSBackingStoreBuffered,
            NO,
        );
        if window == nil {
            return None;
        }
        let _: () = msg_send![window, setReleasedWhenClosed: YES];
        let _: () = msg_send![window, setTitle: nsstring("编辑 LLM 配置")];
        let _: () = msg_send![window, center];

        let content: id = msg_send![window, contentView];
        if content == nil {
            return None;
        }

        let _provider_label = add_label(
            content,
            NSRect::new(NSPoint::new(20.0, 226.0), NSSize::new(120.0, 20.0)),
            "provider:",
        );
        let provider_input = add_input(
            content,
            NSRect::new(NSPoint::new(140.0, 222.0), NSSize::new(460.0, 24.0)),
            &snapshot.llm_provider,
        );

        let _model_label = add_label(
            content,
            NSRect::new(NSPoint::new(20.0, 182.0), NSSize::new(120.0, 20.0)),
            "model:",
        );
        let model_input = add_input(
            content,
            NSRect::new(NSPoint::new(140.0, 178.0), NSSize::new(460.0, 24.0)),
            &snapshot.llm_model,
        );

        let _base_label = add_label(
            content,
            NSRect::new(NSPoint::new(20.0, 138.0), NSSize::new(120.0, 20.0)),
            "api_base:",
        );
        let api_base_input = add_input(
            content,
            NSRect::new(NSPoint::new(140.0, 134.0), NSSize::new(460.0, 24.0)),
            &snapshot.llm_api_base,
        );

        let _key_label = add_label(
            content,
            NSRect::new(NSPoint::new(20.0, 94.0), NSSize::new(120.0, 20.0)),
            "api_key_env:",
        );
        let api_key_env_input = add_input(
            content,
            NSRect::new(NSPoint::new(140.0, 90.0), NSSize::new(460.0, 24.0)),
            &snapshot.llm_api_key_env,
        );

        let _tip = add_label(
            content,
            NSRect::new(NSPoint::new(20.0, 60.0), NSSize::new(580.0, 18.0)),
            "修改后点击确认会自动保存并立即生效",
        );

        let _cancel = add_button(
            content,
            target,
            TAG_LLM_FORM_CANCEL,
            "取消",
            NSRect::new(NSPoint::new(430.0, 18.0), NSSize::new(80.0, 28.0)),
        );
        let _confirm = add_button(
            content,
            target,
            TAG_LLM_FORM_CONFIRM,
            "确认",
            NSRect::new(NSPoint::new(520.0, 18.0), NSSize::new(80.0, 28.0)),
        );

        show_window(window);
        Some(LlmFormDialog {
            window,
            provider_input,
            model_input,
            api_base_input,
            api_key_env_input,
        })
    }

    pub unsafe fn read_llm_form_values(dialog: &LlmFormDialog) -> (String, String, String, String) {
        (
            text_of(dialog.provider_input),
            text_of(dialog.model_input),
            text_of(dialog.api_base_input),
            text_of(dialog.api_key_env_input),
        )
    }

    pub unsafe fn close_llm_form_dialog(dialog: LlmFormDialog) {
        if dialog.window == nil {
            return;
        }
        let _: () = msg_send![dialog.window, orderOut: nil];
        let _: () = msg_send![dialog.window, close];
    }

    pub unsafe fn create_download_dialog(target: id, default_size: &str) -> Option<DownloadDialog> {
        let window = NSWindow::alloc(nil).initWithContentRect_styleMask_backing_defer_(
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(620.0, 360.0)),
            NSWindowStyleMask::NSTitledWindowMask,
            NSBackingStoreType::NSBackingStoreBuffered,
            NO,
        );
        if window == nil {
            return None;
        }
        let _: () = msg_send![window, setReleasedWhenClosed: YES];
        let _: () = msg_send![window, setTitle: nsstring("下载 Whisper 模型")];
        let _: () = msg_send![window, center];

        let content: id = msg_send![window, contentView];
        if content == nil {
            return None;
        }

        let _choose_label = add_label(
            content,
            NSRect::new(NSPoint::new(20.0, 318.0), NSSize::new(120.0, 20.0)),
            "模型尺寸:",
        );
        let model_selector: id = msg_send![class!(NSPopUpButton), alloc];
        let model_selector: id = msg_send![model_selector,
            initWithFrame: NSRect::new(NSPoint::new(96.0, 312.0), NSSize::new(180.0, 28.0))
            pullsDown: NO
        ];
        for size in DOWNLOAD_MODEL_SIZES {
            let _: () = msg_send![model_selector, addItemWithTitle: nsstring(size)];
        }
        let default_index = DOWNLOAD_MODEL_SIZES
            .iter()
            .position(|s| *s == default_size)
            .unwrap_or(0);
        let _: () = msg_send![model_selector, selectItemAtIndex: default_index as isize];
        let _: () = msg_send![content, addSubview: model_selector];

        let start_button = add_button(
            content,
            target,
            TAG_DOWNLOAD_DIALOG_START,
            "下载",
            NSRect::new(NSPoint::new(290.0, 312.0), NSSize::new(80.0, 28.0)),
        );

        let status_label = add_label(
            content,
            NSRect::new(NSPoint::new(20.0, 284.0), NSSize::new(580.0, 20.0)),
            "请选择模型并点击下载",
        );
        let progress_bar: id = msg_send![class!(NSProgressIndicator), alloc];
        let progress_bar: id = msg_send![progress_bar,
            initWithFrame: NSRect::new(NSPoint::new(20.0, 252.0), NSSize::new(580.0, 20.0))
        ];
        let _: () = msg_send![progress_bar, setStyle: 0isize];
        let _: () = msg_send![progress_bar, setMinValue: 0.0f64];
        let _: () = msg_send![progress_bar, setMaxValue: 100.0f64];
        let _: () = msg_send![progress_bar, setDoubleValue: 0.0f64];
        let _: () = msg_send![progress_bar, setIndeterminate: NO];
        let _: () = msg_send![content, addSubview: progress_bar];

        let progress_label = add_label(
            content,
            NSRect::new(NSPoint::new(20.0, 230.0), NSSize::new(580.0, 18.0)),
            "尚未开始",
        );

        let _log_title = add_label(
            content,
            NSRect::new(NSPoint::new(20.0, 204.0), NSSize::new(580.0, 18.0)),
            "下载日志（最新在下方）:",
        );
        let mut log_labels = [nil; 6];
        for (idx, slot) in log_labels.iter_mut().enumerate() {
            let y = 176.0 - (idx as f64 * 24.0);
            let label = add_label(
                content,
                NSRect::new(NSPoint::new(20.0, y), NSSize::new(580.0, 20.0)),
                "",
            );
            *slot = label;
        }

        let cancel_button = add_button(
            content,
            target,
            TAG_DOWNLOAD_DIALOG_CANCEL,
            "关闭",
            NSRect::new(NSPoint::new(430.0, 18.0), NSSize::new(80.0, 28.0)),
        );
        let confirm_button = add_button(
            content,
            target,
            TAG_DOWNLOAD_DIALOG_CONFIRM,
            "确定",
            NSRect::new(NSPoint::new(520.0, 18.0), NSSize::new(80.0, 28.0)),
        );
        set_enabled(confirm_button, false);

        show_window(window);
        Some(DownloadDialog {
            window,
            model_selector,
            start_button,
            status_label,
            progress_label,
            progress_bar,
            log_labels,
            confirm_button,
            cancel_button,
        })
    }

    pub unsafe fn selected_download_size(dialog: &DownloadDialog) -> String {
        let selected: id = msg_send![dialog.model_selector, titleOfSelectedItem];
        let text = nsstring_to_string(selected);
        if text.is_empty() {
            DOWNLOAD_MODEL_SIZES[0].to_string()
        } else {
            text
        }
    }

    pub unsafe fn update_download_dialog(
        dialog: &DownloadDialog,
        status: &str,
        progress_percent: Option<f64>,
        progress_label: &str,
        logs: &[String],
        can_start: bool,
        can_cancel: bool,
        can_confirm: bool,
    ) {
        set_text(dialog.status_label, status);
        set_text(dialog.progress_label, progress_label);

        match progress_percent {
            Some(value) => {
                let _: () = msg_send![dialog.progress_bar, stopAnimation: nil];
                let _: () = msg_send![dialog.progress_bar, setIndeterminate: NO];
                let _: () = msg_send![dialog.progress_bar, setDoubleValue: value.clamp(0.0, 100.0)];
            }
            None => {
                let _: () = msg_send![dialog.progress_bar, setIndeterminate: YES];
                let _: () = msg_send![dialog.progress_bar, startAnimation: nil];
            }
        }

        let visible = dialog.log_labels.len();
        let start = logs.len().saturating_sub(visible);
        for (idx, label) in dialog.log_labels.iter().enumerate() {
            let line = logs
                .get(start + idx)
                .map(|s| shorten_text(s, 96))
                .unwrap_or_default();
            set_text(*label, &line);
        }

        set_enabled(dialog.model_selector, can_start);
        set_enabled(dialog.start_button, can_start);
        set_enabled(dialog.cancel_button, can_cancel);
        set_enabled(dialog.confirm_button, can_confirm);
    }

    pub unsafe fn close_download_dialog(dialog: DownloadDialog) {
        if dialog.window == nil {
            return;
        }
        let _: () = msg_send![dialog.window, orderOut: nil];
        let _: () = msg_send![dialog.window, close];
    }

    unsafe fn add_label(content: id, frame: NSRect, text: &str) -> id {
        let label: id = msg_send![class!(NSTextField), alloc];
        let label: id = msg_send![label, initWithFrame: frame];
        let _: () = msg_send![label, setBezeled: NO];
        let _: () = msg_send![label, setDrawsBackground: NO];
        let _: () = msg_send![label, setEditable: NO];
        let _: () = msg_send![label, setSelectable: NO];
        let _: () = msg_send![label, setStringValue: nsstring(text)];
        let _: () = msg_send![content, addSubview: label];
        label
    }

    unsafe fn add_input(content: id, frame: NSRect, text: &str) -> id {
        let input: id = msg_send![class!(NSTextField), alloc];
        let input: id = msg_send![input, initWithFrame: frame];
        let _: () = msg_send![input, setEditable: YES];
        let _: () = msg_send![input, setSelectable: YES];
        let _: () = msg_send![input, setStringValue: nsstring(text)];
        let _: () = msg_send![content, addSubview: input];
        input
    }

    unsafe fn add_button(content: id, target: id, tag: i64, title: &str, frame: NSRect) -> id {
        let button: id = msg_send![class!(NSButton), alloc];
        let button: id = msg_send![button, initWithFrame: frame];
        let _: () = msg_send![button, setTitle: nsstring(title)];
        let _: () = msg_send![button, setTarget: target];
        let _: () = msg_send![button, setAction: sel!(onMenuItem:)];
        let _: () = msg_send![button, setTag: tag as isize];
        let _: () = msg_send![content, addSubview: button];
        button
    }

    unsafe fn add_submenu(menu: id, title: &str) -> id {
        let item: id = msg_send![class!(NSMenuItem), alloc];
        let item: id = msg_send![item,
            initWithTitle: nsstring(title)
            action: std::ptr::null::<std::ffi::c_void>()
            keyEquivalent: nsstring("")
        ];
        let submenu: id = msg_send![class!(NSMenu), alloc];
        let submenu: id = msg_send![submenu, initWithTitle: nsstring(title)];
        let _: () = msg_send![item, setSubmenu: submenu];
        let _: () = msg_send![menu, addItem: item];
        submenu
    }

    unsafe fn text_of(control: id) -> String {
        if control == nil {
            return String::new();
        }
        let value: id = msg_send![control, stringValue];
        nsstring_to_string(value).trim().to_string()
    }

    unsafe fn set_text(control: id, value: &str) {
        if control == nil {
            return;
        }
        let _: () = msg_send![control, setStringValue: nsstring(value)];
    }

    unsafe fn set_enabled(control: id, enabled: bool) {
        if control == nil {
            return;
        }
        let _: () = msg_send![control, setEnabled: if enabled { YES } else { NO }];
    }

    fn shorten_text(value: &str, max: usize) -> String {
        if value.chars().count() <= max {
            return value.to_string();
        }
        let keep = max.saturating_sub(3);
        let tail = value
            .chars()
            .rev()
            .take(keep)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<String>();
        format!("...{}", tail)
    }

    fn shorten_path(path: &str) -> String {
        const MAX: usize = 42;
        if path.len() <= MAX {
            return path.to_string();
        }
        let tail = &path[path.len() - (MAX - 3)..];
        format!("...{}", tail)
    }

    fn current_whisper_model_name(snapshot: &MenuSnapshot) -> String {
        std::path::Path::new(&snapshot.whisper_model_path)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string()
    }

    fn download_menu_title(snapshot: &MenuSnapshot) -> String {
        let _ = snapshot;
        "下载模型...".to_string()
    }

    fn prompt_choose_from_list(
        title: &str,
        prompt: &str,
        options: &[String],
        default: &str,
    ) -> Option<String> {
        if options.is_empty() {
            return None;
        }
        let escaped_title = applescript_escape(title);
        let escaped_prompt = applescript_escape(prompt);
        let escaped_default = applescript_escape(default);
        let items = options
            .iter()
            .map(|s| format!("\"{}\"", applescript_escape(s)))
            .collect::<Vec<_>>()
            .join(", ");
        let script = format!(
            "set choices to {{{items}}}\nset picked to choose from list choices with title \"{escaped_title}\" with prompt \"{escaped_prompt}\" default items {{\"{escaped_default}\"}}\nif picked is false then return \"\"\nreturn item 1 of picked"
        );
        let output = std::process::Command::new("/usr/bin/osascript")
            .arg("-e")
            .arg(script)
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if value.is_empty() {
            return None;
        }
        Some(value)
    }

    fn prompt_switch_whisper_model(snapshot: &MenuSnapshot) -> Option<MenuAction> {
        let mut options = snapshot
            .local_models
            .iter()
            .filter(|name| name.ends_with(".bin"))
            .cloned()
            .collect::<Vec<_>>();
        for model in WHISPER_MODEL_FILES {
            if !options.iter().any(|m| m == model) {
                options.push(model.to_string());
            }
        }
        options.sort();
        options.dedup();

        let current = current_whisper_model_name(snapshot);
        let default = if options.iter().any(|m| m == &current) {
            current
        } else {
            options[0].clone()
        };

        let selected = prompt_choose_from_list(
            "切换 Whisper 模型",
            "选择要切换的模型（切换后自动保存并立即生效）",
            &options,
            &default,
        )?;
        let model_path = whisper_model_path_from_file_name(&selected).ok()?;
        Some(MenuAction::SwitchWhisperModel { model_path })
    }

    fn modifier_parts(flags: NSEventModifierFlags) -> Vec<String> {
        let mut parts = Vec::new();
        if flags.contains(NSEventModifierFlags::NSControlKeyMask) {
            parts.push("ctrl".to_string());
        }
        if flags.contains(NSEventModifierFlags::NSAlternateKeyMask) {
            parts.push("alt".to_string());
        }
        if flags.contains(NSEventModifierFlags::NSShiftKeyMask) {
            parts.push("shift".to_string());
        }
        if flags.contains(NSEventModifierFlags::NSCommandKeyMask) {
            parts.push("super".to_string());
        }
        parts
    }

    unsafe fn key_name_from_event(event: id, key_code: u16) -> Option<String> {
        if let Some(name) = key_name_from_key_code(key_code) {
            return Some(name.to_string());
        }

        let chars: id = event.charactersIgnoringModifiers();
        if chars == nil {
            return None;
        }
        let text = nsstring_to_string(chars);
        let ch = text.chars().next()?;
        char_to_hotkey_key(ch)
    }

    fn key_name_from_key_code(key_code: u16) -> Option<&'static str> {
        match key_code {
            36 => Some("enter"),
            48 => Some("tab"),
            51 => Some("backspace"),
            53 => Some("esc"),
            115 => Some("home"),
            119 => Some("end"),
            116 => Some("pageup"),
            121 => Some("pagedown"),
            123 => Some("left"),
            124 => Some("right"),
            125 => Some("down"),
            126 => Some("up"),
            122 => Some("f1"),
            120 => Some("f2"),
            99 => Some("f3"),
            118 => Some("f4"),
            96 => Some("f5"),
            97 => Some("f6"),
            98 => Some("f7"),
            100 => Some("f8"),
            101 => Some("f9"),
            109 => Some("f10"),
            103 => Some("f11"),
            111 => Some("f12"),
            _ => None,
        }
    }

    fn char_to_hotkey_key(c: char) -> Option<String> {
        let key = match c {
            'a'..='z' | '0'..='9' => c.to_string(),
            'A'..='Z' => c.to_ascii_lowercase().to_string(),
            ' ' => "space".to_string(),
            '+' | '=' => "equal".to_string(),
            '-' | '_' => "minus".to_string(),
            ',' | '<' => "comma".to_string(),
            '.' | '>' => "period".to_string(),
            ';' | ':' => "semicolon".to_string(),
            '/' | '?' => "slash".to_string(),
            '\'' | '"' => "quote".to_string(),
            '[' | '{' => "bracketleft".to_string(),
            ']' | '}' => "bracketright".to_string(),
            '\\' | '|' => "backslash".to_string(),
            '`' | '~' => "backquote".to_string(),
            _ => return None,
        };
        Some(key)
    }

    unsafe fn nsstring_to_string(value: id) -> String {
        if value == nil {
            return String::new();
        }
        let cstr: *const std::os::raw::c_char = msg_send![value, UTF8String];
        if cstr.is_null() {
            return String::new();
        }
        CStr::from_ptr(cstr).to_string_lossy().into_owned()
    }

    fn applescript_escape(value: &str) -> String {
        value
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', " ")
    }

    unsafe fn nsstring(value: &str) -> id {
        NSString::alloc(nil).init_str(value)
    }

    unsafe fn add_separator(menu: id) {
        let sep: id = msg_send![class!(NSMenuItem), separatorItem];
        let _: () = msg_send![menu, addItem: sep];
    }

    unsafe fn add_info_item(menu: id, title: &str) -> id {
        let item: id = msg_send![class!(NSMenuItem), alloc];
        let item: id = msg_send![item,
            initWithTitle: nsstring(title)
            action: std::ptr::null::<std::ffi::c_void>()
            keyEquivalent: nsstring("")
        ];
        let _: () = msg_send![item, setEnabled: NO];
        let _: () = msg_send![menu, addItem: item];
        item
    }

    unsafe fn add_action_item(menu: id, target: id, tag: i64, title: &str) -> id {
        let item: id = msg_send![class!(NSMenuItem), alloc];
        let item: id = msg_send![item,
            initWithTitle: nsstring(title)
            action: sel!(onMenuItem:)
            keyEquivalent: nsstring("")
        ];
        let _: () = msg_send![item, setTarget: target];
        let _: () = msg_send![item, setTag: tag as isize];
        let _: () = msg_send![menu, addItem: item];
        item
    }

    unsafe fn set_title(item: id, title: &str) {
        let _: () = msg_send![item, setTitle: nsstring(title)];
    }

    unsafe fn set_check_state(item: id, checked: bool) {
        let value = if checked { 1isize } else { 0isize };
        let _: () = msg_send![item, setState: value];
    }
}

#[cfg(target_os = "macos")]
fn empty_snapshot() -> MenuSnapshot {
    MenuSnapshot {
        config_path: String::new(),
        status: "就绪".to_string(),
        dirty: false,
        should_quit_ui: false,
        hotkey: "right_ctrl".to_string(),
        hotkey_trigger_mode: HotkeyTriggerMode::PressToToggle,
        llm_enabled: false,
        text_correction_enabled: true,
        vad_enabled: false,
        llm_provider: "openai".to_string(),
        llm_model: "gpt-4o-mini".to_string(),
        llm_api_base: "https://api.openai.com/v1".to_string(),
        llm_api_key_env: "OPENAI_API_KEY".to_string(),
        whisper_model_path: String::new(),
        local_models: vec![],
        download: None,
        download_logs: vec![],
    }
}

#[cfg(target_os = "macos")]
#[derive(Debug, Default)]
struct HotkeyPopupState {
    dialog: Option<menu_bridge::HotkeyDialog>,
    original: String,
    current: String,
    history: Vec<String>,
}

#[cfg(target_os = "macos")]
#[derive(Debug, Default)]
struct LlmFormPopupState {
    dialog: Option<menu_bridge::LlmFormDialog>,
}

#[cfg(target_os = "macos")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum DownloadDialogPhase {
    #[default]
    Selecting,
    Starting,
    Downloading,
    Finished,
    Failed,
}

#[cfg(target_os = "macos")]
#[derive(Debug, Default)]
struct DownloadPopupState {
    dialog: Option<menu_bridge::DownloadDialog>,
    phase: DownloadDialogPhase,
}

#[cfg(target_os = "macos")]
fn open_llm_form_popup(
    menu_handles: &menu_bridge::MenuHandles,
    snapshot: &MenuSnapshot,
    popup: &mut LlmFormPopupState,
) {
    close_llm_form_popup(popup);
    unsafe {
        popup.dialog = menu_bridge::create_llm_form_dialog(menu_handles.target, snapshot);
        if let Some(dialog) = popup.dialog {
            menu_bridge::show_window(dialog.window);
        }
    }
}

#[cfg(target_os = "macos")]
fn close_llm_form_popup(popup: &mut LlmFormPopupState) {
    if let Some(dialog) = popup.dialog.take() {
        unsafe {
            menu_bridge::close_llm_form_dialog(dialog);
        }
    }
}

#[cfg(target_os = "macos")]
fn expand_tilde_path(path: &str) -> std::path::PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    if path == "~" {
        if let Some(home) = dirs::home_dir() {
            return home;
        }
    }
    std::path::PathBuf::from(path)
}

#[cfg(target_os = "macos")]
fn open_path_in_finder(path: &std::path::Path) -> Result<()> {
    let status = std::process::Command::new("/usr/bin/open")
        .arg(path)
        .status()?;
    if !status.success() {
        anyhow::bail!("无法打开目录: {}", path.display());
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn show_about_popup() {
    let text = "EchoPup 主要功能:\\n- 支持长按模式与按压切换模式\\n- 长按阈值 1 秒启动录音\\n- 语音转写并自动输入\\n- 支持模型下载与切换\\n\\n开发者: liupx\\n开源地址: https://github.com/echopupio/echo-pup-rust";
    let script = format!(
        "display dialog \"{}\" buttons {{\"确定\"}} default button \"确定\" with title \"关于 EchoPup\"",
        text
    );
    let _ = std::process::Command::new("/usr/bin/osascript")
        .arg("-e")
        .arg(script)
        .output();
}

#[cfg(target_os = "macos")]
fn preferred_download_size(snapshot: &MenuSnapshot) -> &'static str {
    let model_name = std::path::Path::new(&snapshot.whisper_model_path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    if model_name.contains("turbo") {
        "turbo"
    } else if model_name.contains("medium") {
        "medium"
    } else {
        "large-v3"
    }
}

#[cfg(target_os = "macos")]
fn open_hotkey_popup(
    menu_handles: &menu_bridge::MenuHandles,
    snapshot: &MenuSnapshot,
    popup: &mut HotkeyPopupState,
) {
    unsafe {
        if popup.dialog.is_none() {
            popup.dialog = menu_bridge::create_hotkey_dialog(menu_handles.target, &snapshot.hotkey);
        }
        popup.original = snapshot.hotkey.clone();
        popup.current = snapshot.hotkey.clone();
        popup.history.clear();
        if let Some(dialog) = popup.dialog {
            menu_bridge::update_hotkey_dialog(
                &dialog,
                &popup.current,
                "按下按键后可撤销，确认后自动保存并生效",
                false,
                false,
            );
            menu_bridge::show_window(dialog.window);
        }
    }
}

#[cfg(target_os = "macos")]
fn close_hotkey_popup(popup: &mut HotkeyPopupState) {
    if let Some(dialog) = popup.dialog.take() {
        unsafe {
            menu_bridge::close_hotkey_dialog(dialog);
        }
    }
    popup.original.clear();
    popup.current.clear();
    popup.history.clear();
}

#[cfg(target_os = "macos")]
fn update_hotkey_popup_view(popup: &mut HotkeyPopupState, hint: &str) {
    let Some(dialog) = popup.dialog else {
        return;
    };
    unsafe {
        menu_bridge::update_hotkey_dialog(
            &dialog,
            &popup.current,
            hint,
            !popup.history.is_empty(),
            popup.current != popup.original,
        );
    }
}

#[cfg(target_os = "macos")]
fn open_download_popup(
    menu_handles: &menu_bridge::MenuHandles,
    snapshot: &MenuSnapshot,
    popup: &mut DownloadPopupState,
) {
    unsafe {
        if popup.dialog.is_none() {
            popup.dialog = menu_bridge::create_download_dialog(
                menu_handles.target,
                preferred_download_size(snapshot),
            );
        }
        if snapshot
            .download
            .as_ref()
            .map(|d| d.in_progress)
            .unwrap_or(false)
        {
            popup.phase = DownloadDialogPhase::Downloading;
        } else {
            popup.phase = DownloadDialogPhase::Selecting;
        }
        if let Some(dialog) = popup.dialog {
            menu_bridge::show_window(dialog.window);
        }
    }
    sync_download_popup(snapshot, popup);
}

#[cfg(target_os = "macos")]
fn close_download_popup(popup: &mut DownloadPopupState) {
    if let Some(dialog) = popup.dialog.take() {
        unsafe {
            menu_bridge::close_download_dialog(dialog);
        }
    }
    popup.phase = DownloadDialogPhase::Selecting;
}

#[cfg(target_os = "macos")]
fn sync_download_popup(snapshot: &MenuSnapshot, popup: &mut DownloadPopupState) {
    let Some(dialog) = popup.dialog else {
        return;
    };
    unsafe {
        if let Some(download) = snapshot.download.as_ref() {
            let (ratio, ratio_label) = crate::model_download::download_ratio_label(download);
            if download.in_progress {
                popup.phase = DownloadDialogPhase::Downloading;
                let progress = if download.total.is_some() {
                    Some(ratio * 100.0)
                } else {
                    None
                };
                menu_bridge::update_download_dialog(
                    &dialog,
                    &format!("正在下载模型 {}", download.model_size),
                    progress,
                    &ratio_label,
                    &snapshot.download_logs,
                    false,
                    false,
                    false,
                );
                return;
            }

            if snapshot.status.contains("下载失败")
                && matches!(
                    popup.phase,
                    DownloadDialogPhase::Starting | DownloadDialogPhase::Downloading
                )
            {
                popup.phase = DownloadDialogPhase::Failed;
                menu_bridge::update_download_dialog(
                    &dialog,
                    &snapshot.status,
                    download.total.map(|_| ratio * 100.0),
                    &ratio_label,
                    &snapshot.download_logs,
                    false,
                    false,
                    true,
                );
                return;
            }

            if matches!(
                popup.phase,
                DownloadDialogPhase::Starting | DownloadDialogPhase::Downloading
            ) {
                popup.phase = DownloadDialogPhase::Finished;
                menu_bridge::update_download_dialog(
                    &dialog,
                    &format!("模型 {} 下载完成", download.model_size),
                    Some((ratio * 100.0).clamp(0.0, 100.0)),
                    &ratio_label,
                    &snapshot.download_logs,
                    false,
                    false,
                    true,
                );
                return;
            }
        }

        if snapshot.status.contains("下载失败")
            && matches!(
                popup.phase,
                DownloadDialogPhase::Starting | DownloadDialogPhase::Downloading
            )
        {
            popup.phase = DownloadDialogPhase::Failed;
            menu_bridge::update_download_dialog(
                &dialog,
                &snapshot.status,
                Some(0.0),
                "下载失败",
                &snapshot.download_logs,
                false,
                false,
                true,
            );
            return;
        }

        if matches!(popup.phase, DownloadDialogPhase::Selecting) {
            menu_bridge::update_download_dialog(
                &dialog,
                "请选择模型并点击下载",
                Some(0.0),
                "尚未开始",
                &snapshot.download_logs,
                true,
                true,
                false,
            );
        }
    }
}

#[cfg(target_os = "macos")]
pub fn run_status_indicator_process() -> Result<()> {
    use cocoa::appkit::{
        NSApp, NSApplication, NSApplicationActivationPolicy, NSEventMask, NSStatusBar, NSStatusItem,
    };
    use cocoa::base::{id, nil, YES};
    use cocoa::foundation::NSString;
    use objc::{class, msg_send, sel, sel_impl};

    let rx = spawn_stdin_reader();
    let (menu_tx, menu_rx) = std::sync::mpsc::channel::<i64>();

    unsafe {
        let app = NSApp();
        app.setActivationPolicy_(
            NSApplicationActivationPolicy::NSApplicationActivationPolicyAccessory,
        );
        app.finishLaunching();

        let status_bar = NSStatusBar::systemStatusBar(nil);
        let status_item = status_bar.statusItemWithLength_(STATUS_ITEM_LENGTH_IDLE);
        let button: id = status_item.button();

        let mut background_layer: id = nil;
        let mut image_idle: id = nil;
        let mut image_active: id = nil;

        let (menu, menu_handles) = menu_bridge::create_menu(menu_tx);
        let _: () = msg_send![status_item, setMenu: menu];

        if button != nil {
            background_layer = create_background_layer(button);
            let logo = load_png_image(STATUS_LOGO_PNG);
            let mic = load_png_image(STATUS_MICROPHONE_PNG);
            image_idle = compose_status_image(logo, mic, false);
            image_active = compose_status_image(logo, mic, true);

            if image_idle != nil {
                set_status_image(button, image_idle);
            } else {
                warn!("状态栏 logo 加载失败，将仅显示空白状态");
            }
            set_status_title(button, IndicatorState::Idle.menu_title());
            apply_button_style(button, background_layer, IndicatorState::Idle, 0.0);
        } else {
            warn!("状态栏按钮不可用，状态展示可能异常");
        }

        let run_loop_mode = NSString::alloc(nil).init_str("kCFRunLoopDefaultMode");
        let mut auto_back_to_idle_deadline: Option<std::time::Instant> = None;
        let mut current_state = IndicatorState::Idle;
        let mut pulse_phase = 0.0f32;
        let mut should_exit = false;
        let mut latest_snapshot = empty_snapshot();
        let mut hotkey_popup = HotkeyPopupState::default();
        let mut llm_form_popup = LlmFormPopupState::default();
        let mut download_popup = DownloadPopupState::default();

        info!("macOS 状态栏指示器已启动");

        while !should_exit {
            while let Ok(tag) = menu_rx.try_recv() {
                match tag {
                    menu_bridge::TAG_EDIT_HOTKEY => {
                        open_hotkey_popup(&menu_handles, &latest_snapshot, &mut hotkey_popup);
                        continue;
                    }
                    menu_bridge::TAG_HOTKEY_UNDO => {
                        if let Some(previous) = hotkey_popup.history.pop() {
                            hotkey_popup.current = previous;
                            update_hotkey_popup_view(
                                &mut hotkey_popup,
                                "已撤销到上一个按键，确认后生效",
                            );
                        }
                        continue;
                    }
                    menu_bridge::TAG_HOTKEY_CONFIRM => {
                        if hotkey_popup.dialog.is_some() {
                            let should_apply = hotkey_popup.current != hotkey_popup.original;
                            let next_hotkey = hotkey_popup.current.clone();
                            close_hotkey_popup(&mut hotkey_popup);
                            if should_apply {
                                send_child_message(&ChildMessage::ActionRequest {
                                    action: MenuAction::SetField {
                                        field: EditableField::Hotkey,
                                        value: next_hotkey,
                                    },
                                });
                            }
                        }
                        continue;
                    }
                    menu_bridge::TAG_HOTKEY_CANCEL => {
                        close_hotkey_popup(&mut hotkey_popup);
                        continue;
                    }
                    menu_bridge::TAG_EDIT_LLM_FORM => {
                        open_llm_form_popup(&menu_handles, &latest_snapshot, &mut llm_form_popup);
                        continue;
                    }
                    menu_bridge::TAG_LLM_FORM_CANCEL => {
                        close_llm_form_popup(&mut llm_form_popup);
                        continue;
                    }
                    menu_bridge::TAG_LLM_FORM_CONFIRM => {
                        if let Some(dialog) = llm_form_popup.dialog {
                            let (provider, model, api_base, api_key_env) =
                                menu_bridge::read_llm_form_values(&dialog);
                            close_llm_form_popup(&mut llm_form_popup);
                            send_child_message(&ChildMessage::ActionRequest {
                                action: MenuAction::SetLlmConfig {
                                    provider,
                                    model,
                                    api_base,
                                    api_key_env,
                                },
                            });
                        }
                        continue;
                    }
                    menu_bridge::TAG_DOWNLOAD_MODEL => {
                        open_download_popup(&menu_handles, &latest_snapshot, &mut download_popup);
                        continue;
                    }
                    menu_bridge::TAG_OPEN_CONFIG_FOLDER => {
                        let path = expand_tilde_path(&latest_snapshot.config_path);
                        let dir = if path.is_dir() {
                            path
                        } else {
                            path.parent().unwrap_or(path.as_path()).to_path_buf()
                        };
                        if let Err(err) = open_path_in_finder(&dir) {
                            warn!("打开配置文件夹失败: {}", err);
                        }
                        continue;
                    }
                    menu_bridge::TAG_VIEW_MODEL_FILES => {
                        match crate::runtime::model_dir() {
                            Ok(dir) => {
                                if let Err(err) = open_path_in_finder(&dir) {
                                    warn!("打开模型目录失败: {}", err);
                                }
                            }
                            Err(err) => warn!("获取模型目录失败: {}", err),
                        }
                        continue;
                    }
                    menu_bridge::TAG_SHOW_ABOUT => {
                        show_about_popup();
                        continue;
                    }
                    menu_bridge::TAG_DOWNLOAD_DIALOG_START => {
                        if let Some(dialog) = download_popup.dialog {
                            if matches!(download_popup.phase, DownloadDialogPhase::Selecting) {
                                let size = menu_bridge::selected_download_size(&dialog);
                                download_popup.phase = DownloadDialogPhase::Starting;
                                let logs = vec![format!("[start] 已请求下载 {}", size)];
                                menu_bridge::update_download_dialog(
                                    &dialog,
                                    "正在创建下载任务...",
                                    None,
                                    "等待下载线程启动...",
                                    &logs,
                                    false,
                                    false,
                                    false,
                                );
                                send_child_message(&ChildMessage::ActionRequest {
                                    action: MenuAction::DownloadModel { size },
                                });
                            }
                        }
                        continue;
                    }
                    menu_bridge::TAG_DOWNLOAD_DIALOG_CANCEL => {
                        if matches!(download_popup.phase, DownloadDialogPhase::Selecting) {
                            close_download_popup(&mut download_popup);
                        }
                        continue;
                    }
                    menu_bridge::TAG_DOWNLOAD_DIALOG_CONFIRM => {
                        if matches!(
                            download_popup.phase,
                            DownloadDialogPhase::Finished | DownloadDialogPhase::Failed
                        ) {
                            close_download_popup(&mut download_popup);
                        }
                        continue;
                    }
                    _ => {}
                }

                if let Some(action) = menu_bridge::map_tag_to_action(tag, &latest_snapshot) {
                    send_child_message(&ChildMessage::ActionRequest { action });
                }
            }

            while let Ok(cmd) = rx.try_recv() {
                match cmd {
                    IndicatorCommand::Message(msg) => match msg {
                        ParentMessage::SetState { state } => {
                            current_state = state;
                            pulse_phase = 0.0;
                            status_item.setLength_(status_item_length_for_state(state));
                            if button != nil {
                                if matches!(state, IndicatorState::Idle) {
                                    set_status_image(button, image_idle);
                                } else {
                                    set_status_image(button, image_active);
                                }
                                set_status_title(button, state.menu_title());
                                apply_button_style(button, background_layer, state, pulse_phase);
                            }
                            auto_back_to_idle_deadline = state
                                .auto_reset_duration()
                                .map(|d| std::time::Instant::now() + d);
                        }
                        ParentMessage::SetSnapshot { snapshot } => {
                            latest_snapshot = snapshot;
                            menu_bridge::update_menu(&menu_handles, &latest_snapshot);
                            sync_download_popup(&latest_snapshot, &mut download_popup);
                        }
                        ParentMessage::SetActionResult { result } => {
                            let ok = result.ok;
                            let message = result.message;
                            latest_snapshot = result.snapshot;
                            menu_bridge::update_menu(&menu_handles, &latest_snapshot);
                            if !ok
                                && matches!(download_popup.phase, DownloadDialogPhase::Starting)
                                && (message.contains("下载")
                                    || latest_snapshot.status.contains("下载"))
                            {
                                download_popup.phase = DownloadDialogPhase::Failed;
                                if let Some(dialog) = download_popup.dialog {
                                    let logs = if latest_snapshot.download_logs.is_empty() {
                                        vec![format!("[error] {}", message)]
                                    } else {
                                        latest_snapshot.download_logs.clone()
                                    };
                                    menu_bridge::update_download_dialog(
                                        &dialog,
                                        &format!("下载失败: {}", message),
                                        Some(0.0),
                                        "下载失败",
                                        &logs,
                                        false,
                                        false,
                                        true,
                                    );
                                }
                            }
                            sync_download_popup(&latest_snapshot, &mut download_popup);
                        }
                        ParentMessage::Exit => {
                            should_exit = true;
                            break;
                        }
                    },
                    IndicatorCommand::Exit => {
                        should_exit = true;
                        break;
                    }
                }
            }

            if current_state.visual_style().is_pulsing() && button != nil {
                pulse_phase = (pulse_phase + 0.20) % std::f32::consts::TAU;
                apply_button_style(button, background_layer, current_state, pulse_phase);
            }

            if let Some(deadline) = auto_back_to_idle_deadline {
                if std::time::Instant::now() >= deadline {
                    current_state = IndicatorState::Idle;
                    pulse_phase = 0.0;
                    status_item.setLength_(status_item_length_for_state(IndicatorState::Idle));
                    if button != nil {
                        set_status_image(button, image_idle);
                        set_status_title(button, IndicatorState::Idle.menu_title());
                        apply_button_style(
                            button,
                            background_layer,
                            IndicatorState::Idle,
                            pulse_phase,
                        );
                    }
                    auto_back_to_idle_deadline = None;
                }
            }

            let distant_past: id = msg_send![class!(NSDate), distantPast];
            let event = app.nextEventMatchingMask_untilDate_inMode_dequeue_(
                NSEventMask::NSAnyEventMask.bits(),
                distant_past,
                run_loop_mode,
                YES,
            );
            if event != nil {
                if hotkey_popup.dialog.is_some() {
                    match menu_bridge::capture_hotkey_from_event(event) {
                        menu_bridge::HotkeyCaptureResult::Captured(hotkey) => {
                            if hotkey != hotkey_popup.current {
                                hotkey_popup.history.push(hotkey_popup.current.clone());
                                hotkey_popup.current = hotkey.clone();
                            }
                            update_hotkey_popup_view(
                                &mut hotkey_popup,
                                &format!("已捕获: {}。可撤销或确认", hotkey),
                            );
                            continue;
                        }
                        menu_bridge::HotkeyCaptureResult::Cancelled => {
                            close_hotkey_popup(&mut hotkey_popup);
                            continue;
                        }
                        menu_bridge::HotkeyCaptureResult::Ignored => {}
                    }
                }
                app.sendEvent_(event);
            }

            std::thread::sleep(std::time::Duration::from_millis(40));
        }

        close_hotkey_popup(&mut hotkey_popup);
        close_llm_form_popup(&mut llm_form_popup);
        close_download_popup(&mut download_popup);
        status_bar.removeStatusItem_(status_item);
    }

    Ok(())
}

#[cfg(not(target_os = "macos"))]
pub fn run_status_indicator_process() -> Result<()> {
    anyhow::bail!("status-indicator 仅在 macOS 可用");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "macos")]
    #[test]
    fn test_phase_e_non_edit_tag_action_mapping() {
        let snapshot = empty_snapshot();
        assert!(matches!(
            menu_bridge::map_tag_to_action(menu_bridge::TAG_TOGGLE_LLM, &snapshot),
            Some(MenuAction::ToggleLlmEnabled)
        ));
        assert!(matches!(
            menu_bridge::map_tag_to_action(menu_bridge::TAG_TOGGLE_CORRECTION, &snapshot),
            Some(MenuAction::ToggleTextCorrectionEnabled)
        ));
        assert!(matches!(
            menu_bridge::map_tag_to_action(menu_bridge::TAG_TOGGLE_VAD, &snapshot),
            Some(MenuAction::ToggleVadEnabled)
        ));
        assert!(matches!(
            menu_bridge::map_tag_to_action(menu_bridge::TAG_DOWNLOAD_MODEL, &snapshot),
            Some(MenuAction::DownloadModel { .. }) | None
        ));
        assert!(matches!(
            menu_bridge::map_tag_to_action(menu_bridge::TAG_QUIT_UI, &snapshot),
            Some(MenuAction::QuitUi)
        ));
        assert!(menu_bridge::is_hotkey_capture_tag(
            menu_bridge::TAG_EDIT_HOTKEY
        ));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn test_phase_e_parse_indicator_command_compat() {
        let cmd = parse_indicator_command("recording").expect("legacy state wire");
        assert!(matches!(
            cmd,
            IndicatorCommand::Message(ParentMessage::SetState {
                state: IndicatorState::Recording
            })
        ));

        let line = serde_json::to_string(&ParentMessage::SetState {
            state: IndicatorState::Idle,
        })
        .unwrap();
        let cmd2 = parse_indicator_command(&line).expect("json wire");
        assert!(matches!(
            cmd2,
            IndicatorCommand::Message(ParentMessage::SetState {
                state: IndicatorState::Idle
            })
        ));
    }
}
