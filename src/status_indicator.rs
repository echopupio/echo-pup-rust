#![allow(unexpected_cfgs)]
//! 状态栏反馈与菜单 IPC（macOS）

use anyhow::Result;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::config::HotkeyTriggerMode;
use crate::menu_core::{MenuAction, MenuActionResult, MenuSnapshot};

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
        #[cfg(target_os = "linux")]
        {
            match self {
                Self::Idle => "空闲",
                Self::RecordingStart => "准备录音",
                Self::Recording => "录音中",
                Self::Transcribing => "识别中",
                Self::Completed => "完成",
                Self::Failed => "失败",
            }
        }

        #[cfg(not(target_os = "linux"))]
        {
            let _ = self;
            ""
        }
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

#[cfg(any(target_os = "macos", target_os = "linux"))]
const STATUS_LOGO_PNG: &[u8] = include_bytes!("../assets/logo.png");
#[cfg(any(target_os = "macos", target_os = "linux"))]
const STATUS_MICROPHONE_PNG: &[u8] = include_bytes!("../assets/mic.png");
#[cfg(target_os = "macos")]
const STATUS_ITEM_LENGTH_IDLE: f64 = 28.0;
#[cfg(target_os = "macos")]
const STATUS_ITEM_LENGTH_ACTIVE: f64 = 40.0;
#[cfg(target_os = "macos")]
const STATUS_ICON_SIZE: f64 = 28.0;
#[cfg(target_os = "macos")]
const STATUS_LOGO_VISUAL_SCALE: f64 = 0.88;
#[cfg(target_os = "macos")]
const STATUS_MIC_VISUAL_SCALE: f64 = 0.68;
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
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
impl IndicatorState {
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

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    child: Option<std::process::Child>,
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    stdin: Option<std::process::ChildStdin>,
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    action_rx: Option<std::sync::mpsc::Receiver<MenuAction>>,
}

impl StatusIndicatorClient {
    pub fn start(enabled: bool, config_path: &str) -> Self {
        #[cfg(any(target_os = "macos", target_os = "linux"))]
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

        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
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

        #[cfg(any(target_os = "macos", target_os = "linux"))]
        {
            let Some(rx) = self.action_rx.as_ref() else {
                return None;
            };
            return rx.try_recv().ok();
        }

        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            None
        }
    }

    pub fn close_ui(&mut self) {
        if !self.enabled {
            return;
        }

        let _ = self.send_message(&ParentMessage::Exit);

        #[cfg(any(target_os = "macos", target_os = "linux"))]
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

        #[cfg(any(target_os = "macos", target_os = "linux"))]
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

#[cfg(any(target_os = "macos", target_os = "linux"))]
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

    let draw_area = NSRect::new(
        NSPoint::new(STATUS_ICON_H_INSET, STATUS_ICON_V_INSET),
        NSSize::new(
            (STATUS_ICON_SIZE - STATUS_ICON_H_INSET * 2.0).max(1.0),
            (STATUS_ICON_SIZE - STATUS_ICON_V_INSET * 2.0).max(1.0),
        ),
    );

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
    let scale = (draw_area.size.width / src_w).min(draw_area.size.height / src_h) * visual_scale;
    let draw_w = src_w * scale;
    let draw_h = src_h * scale;
    let draw_rect = NSRect::new(
        NSPoint::new(
            draw_area.origin.x + (draw_area.size.width - draw_w) * 0.5,
            draw_area.origin.y + (draw_area.size.height - draw_h) * 0.5,
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

#[cfg(any(target_os = "macos", target_os = "linux"))]
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
    let vpad = 2.0f64;
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
    use cocoa::appkit::{NSApp, NSBackingStoreType, NSWindow, NSWindowStyleMask};
    use cocoa::base::{id, nil, NO, YES};
    use cocoa::foundation::{NSPoint, NSRect, NSSize, NSString};
    use objc::declare::ClassDecl;
    use objc::runtime::{Class, Object, Sel};
    use objc::{class, msg_send, sel, sel_impl};
    use std::ffi::CStr;
    use std::sync::{Mutex, Once, OnceLock};

    pub const TAG_TOGGLE_LLM: i64 = 1001;
    pub const TAG_TOGGLE_CORRECTION: i64 = 1002;
    pub const TAG_EDIT_LLM_FORM: i64 = 1005;
    pub const TAG_QUIT_UI: i64 = 1008;
    pub const TAG_OPEN_CONFIG_FOLDER: i64 = 1009;
    pub const TAG_RELOAD_CONFIG: i64 = 1010;
    pub const TAG_VIEW_MODEL_FILES: i64 = 1011;
    pub const TAG_SHOW_ABOUT: i64 = 1012;
    pub const TAG_MODE_HOLD: i64 = 1013;
    pub const TAG_MODE_PRESS_TOGGLE: i64 = 1014;
    pub const TAG_LLM_FORM_CONFIRM: i64 = 2101;
    pub const TAG_LLM_FORM_CANCEL: i64 = 2102;
    pub const TAG_LLM_PROVIDER_CHANGED: i64 = 2103;

    static MENU_EVENT_TX: OnceLock<Mutex<Option<std::sync::mpsc::Sender<i64>>>> = OnceLock::new();

    pub struct MenuHandles {
        pub target: id,
        pub toggle_llm: id,
        pub toggle_correction: id,
        pub mode_hold: id,
        pub mode_press_toggle: id,
        pub edit_llm_form: id,
        pub status_line: id,
    }

    #[derive(Debug, Clone, Copy)]
    pub struct LlmFormDialog {
        pub window: id,
        pub provider_input: id,
        pub model_input: id,
        pub api_base_input: id,
        pub api_key_input: id,
        pub key_label: id,
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

            let toggle_llm = add_action_item(menu, target, TAG_TOGGLE_LLM, "启用 LLM 润色");
            let edit_llm_form =
                add_action_item(menu, target, TAG_EDIT_LLM_FORM, "编辑 LLM 配置...");
            let toggle_correction =
                add_action_item(menu, target, TAG_TOGGLE_CORRECTION, "启用文本纠错");

            add_separator(menu);

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

            add_separator(menu);

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
                mode_hold,
                mode_press_toggle,
                edit_llm_form,
                status_line,
            };

            (menu, handles)
        }
    }

    pub fn map_tag_to_action(tag: i64, _snapshot: &MenuSnapshot) -> Option<MenuAction> {
        match tag {
            TAG_TOGGLE_LLM => Some(MenuAction::ToggleLlmEnabled),
            TAG_TOGGLE_CORRECTION => Some(MenuAction::ToggleTextCorrectionEnabled),
            TAG_MODE_HOLD => Some(MenuAction::SetHotkeyTriggerMode {
                mode: HotkeyTriggerMode::HoldToRecord,
            }),
            TAG_MODE_PRESS_TOGGLE => Some(MenuAction::SetHotkeyTriggerMode {
                mode: HotkeyTriggerMode::PressToToggle,
            }),
            TAG_EDIT_LLM_FORM => None,
            TAG_RELOAD_CONFIG => Some(MenuAction::ReloadConfig),
            TAG_QUIT_UI => Some(MenuAction::QuitUi),
            _ => None,
        }
    }

    pub fn update_menu(handles: &MenuHandles, snapshot: &MenuSnapshot) {
        unsafe {
            set_check_state(handles.toggle_llm, snapshot.llm_enabled);
            // 始终显示编辑按钮，用户需先配置再启用

            set_check_state(handles.toggle_correction, snapshot.text_correction_enabled);
            set_check_state(
                handles.mode_hold,
                snapshot.hotkey_trigger_mode == HotkeyTriggerMode::HoldToRecord,
            );
            set_check_state(
                handles.mode_press_toggle,
                snapshot.hotkey_trigger_mode == HotkeyTriggerMode::PressToToggle,
            );

            set_title(
                handles.edit_llm_form,
                &format!(
                    "编辑 LLM 配置 ({}/{})",
                    snapshot.llm_provider, snapshot.llm_model
                ),
            );

            set_title(handles.status_line, &format!("状态: {}", snapshot.status));
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
            "模型提供商:",
        );
        let provider_items: &[&str] = &["OpenAI 兼容接口", "Anthropic 兼容接口", "Ollama 本地"];
        let selected_label = provider_key_to_label(&snapshot.llm_provider);
        let provider_input = add_popup(
            content,
            NSRect::new(NSPoint::new(140.0, 222.0), NSSize::new(460.0, 26.0)),
            provider_items,
            &selected_label,
        );
        let _: () = msg_send![provider_input, setTarget: target];
        let _: () = msg_send![provider_input, setAction: sel!(onMenuItem:)];
        let _: () = msg_send![provider_input, setTag: TAG_LLM_PROVIDER_CHANGED as isize];

        let _model_label = add_label(
            content,
            NSRect::new(NSPoint::new(20.0, 182.0), NSSize::new(120.0, 20.0)),
            "模型名称:",
        );
        let model_input = add_input(
            content,
            NSRect::new(NSPoint::new(140.0, 178.0), NSSize::new(460.0, 24.0)),
            &snapshot.llm_model,
        );

        let _base_label = add_label(
            content,
            NSRect::new(NSPoint::new(20.0, 138.0), NSSize::new(120.0, 20.0)),
            "接口地址:",
        );
        let api_base_input = add_input(
            content,
            NSRect::new(NSPoint::new(140.0, 134.0), NSSize::new(460.0, 24.0)),
            &snapshot.llm_api_base,
        );

        let key_label = add_label(
            content,
            NSRect::new(NSPoint::new(20.0, 94.0), NSSize::new(120.0, 20.0)),
            "API 密钥:",
        );
        let api_key_input = add_secure_input(
            content,
            NSRect::new(NSPoint::new(140.0, 90.0), NSSize::new(460.0, 24.0)),
            &snapshot.llm_api_key,
        );

        let is_ollama = snapshot.llm_provider == "ollama";
        let _: () = msg_send![key_label, setHidden: is_ollama as cocoa::base::BOOL];
        let _: () = msg_send![api_key_input, setHidden: is_ollama as cocoa::base::BOOL];

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
            api_key_input,
            key_label,
        })
    }

    pub unsafe fn update_key_field_visibility(dialog: &LlmFormDialog) {
        let title: id = msg_send![dialog.provider_input, titleOfSelectedItem];
        let label = nsstring_to_string(title).trim().to_string();
        let is_ollama = label == "Ollama 本地";
        let _: () = msg_send![dialog.key_label, setHidden: is_ollama as cocoa::base::BOOL];
        let _: () = msg_send![dialog.api_key_input, setHidden: is_ollama as cocoa::base::BOOL];
    }

    pub unsafe fn read_llm_form_values(dialog: &LlmFormDialog) -> (String, String, String, String) {
        let provider_title: id = msg_send![dialog.provider_input, titleOfSelectedItem];
        let label = nsstring_to_string(provider_title).trim().to_string();
        let provider = provider_label_to_key(&label);
        (
            provider,
            text_of(dialog.model_input),
            text_of(dialog.api_base_input),
            text_of(dialog.api_key_input),
        )
    }

    fn provider_key_to_label(key: &str) -> String {
        match key {
            "anthropic" => "Anthropic 兼容接口".to_string(),
            "ollama" => "Ollama 本地".to_string(),
            _ => "OpenAI 兼容接口".to_string(),
        }
    }

    fn provider_label_to_key(label: &str) -> String {
        match label {
            "Anthropic 兼容接口" => "anthropic".to_string(),
            "Ollama 本地" => "ollama".to_string(),
            _ => "openai".to_string(),
        }
    }

    pub unsafe fn close_llm_form_dialog(dialog: LlmFormDialog) {
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

    unsafe fn add_popup(content: id, frame: NSRect, items: &[&str], selected: &str) -> id {
        let popup: id = msg_send![class!(NSPopUpButton), alloc];
        let popup: id = msg_send![popup, initWithFrame:frame pullsDown:NO];
        for item in items {
            let _: () = msg_send![popup, addItemWithTitle: nsstring(item)];
        }
        let _: () = msg_send![popup, selectItemWithTitle: nsstring(selected)];
        let _: () = msg_send![content, addSubview: popup];
        popup
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

    unsafe fn add_secure_input(content: id, frame: NSRect, text: &str) -> id {
        let input: id = msg_send![class!(NSSecureTextField), alloc];
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

    #[allow(dead_code)]
    unsafe fn set_text(control: id, value: &str) {
        if control == nil {
            return;
        }
        let _: () = msg_send![control, setStringValue: nsstring(value)];
    }

    #[allow(dead_code)]
    unsafe fn set_enabled(control: id, enabled: bool) {
        if control == nil {
            return;
        }
        let _: () = msg_send![control, setEnabled: if enabled { YES } else { NO }];
    }

    #[allow(dead_code)]
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

    pub unsafe fn nsstring(value: &str) -> id {
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

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn empty_snapshot() -> MenuSnapshot {
    MenuSnapshot {
        config_path: String::new(),
        status: "就绪".to_string(),
        dirty: false,
        should_quit_ui: false,
        hotkey_trigger_mode: HotkeyTriggerMode::PressToToggle,
        llm_enabled: false,
        text_correction_enabled: true,
        llm_provider: "openai".to_string(),
        llm_model: "gpt-4o-mini".to_string(),
        llm_api_base: "https://api.openai.com/v1".to_string(),
        llm_api_key: String::new(),
    }
}

#[cfg(target_os = "macos")]
#[derive(Debug, Default)]
struct LlmFormPopupState {
    dialog: Option<menu_bridge::LlmFormDialog>,
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
    use cocoa::appkit::{NSBackingStoreType, NSWindow, NSWindowStyleMask};
    use cocoa::base::{id, nil, NO, YES};
    use cocoa::foundation::{NSPoint, NSRect, NSSize};
    use objc::{class, msg_send, sel, sel_impl};

    unsafe {
        let window = NSWindow::alloc(nil).initWithContentRect_styleMask_backing_defer_(
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(360.0, 340.0)),
            NSWindowStyleMask::NSTitledWindowMask | NSWindowStyleMask::NSClosableWindowMask,
            NSBackingStoreType::NSBackingStoreBuffered,
            NO,
        );
        if window == nil {
            return;
        }
        let _: () = msg_send![window, setReleasedWhenClosed: YES];
        let _: () = msg_send![window, setTitle: menu_bridge::nsstring("关于 EchoPup")];
        let _: () = msg_send![window, center];

        let content: id = msg_send![window, contentView];
        if content == nil {
            return;
        }

        // Title: "EchoPup" centered at top
        let title: id = msg_send![class!(NSTextField), alloc];
        let title: id = msg_send![title, initWithFrame: NSRect::new(NSPoint::new(0.0, 280.0), NSSize::new(360.0, 30.0))];
        let _: () = msg_send![title, setBezeled: NO];
        let _: () = msg_send![title, setDrawsBackground: NO];
        let _: () = msg_send![title, setEditable: NO];
        let _: () = msg_send![title, setSelectable: NO];
        let _: () = msg_send![title, setAlignment: 1i64]; // NSTextAlignmentCenter
        let _: () = msg_send![title, setStringValue: menu_bridge::nsstring("EchoPup")];
        let font: id = msg_send![class!(NSFont), boldSystemFontOfSize: 20.0f64];
        let _: () = msg_send![title, setFont: font];
        let _: () = msg_send![content, addSubview: title];

        // Logo: centered, 128x128
        let logo_data = STATUS_LOGO_PNG;
        let ns_data: id =
            msg_send![class!(NSData), dataWithBytes:logo_data.as_ptr() length:logo_data.len()];
        let image: id = msg_send![class!(NSImage), alloc];
        let image: id = msg_send![image, initWithData: ns_data];
        if image != nil {
            let _: () = msg_send![image, setSize: NSSize::new(128.0, 128.0)];
            let image_view: id = msg_send![class!(NSImageView), alloc];
            let image_view: id = msg_send![image_view, initWithFrame: NSRect::new(NSPoint::new(116.0, 140.0), NSSize::new(128.0, 128.0))];
            let _: () = msg_send![image_view, setImage: image];
            let _: () = msg_send![content, addSubview: image_view];
        }

        // Version
        let ver_text = format!("v{}", env!("CARGO_PKG_VERSION"));
        let ver: id = msg_send![class!(NSTextField), alloc];
        let ver: id = msg_send![ver, initWithFrame: NSRect::new(NSPoint::new(0.0, 110.0), NSSize::new(360.0, 20.0))];
        let _: () = msg_send![ver, setBezeled: NO];
        let _: () = msg_send![ver, setDrawsBackground: NO];
        let _: () = msg_send![ver, setEditable: NO];
        let _: () = msg_send![ver, setSelectable: NO];
        let _: () = msg_send![ver, setAlignment: 1i64];
        let _: () = msg_send![ver, setStringValue: menu_bridge::nsstring(&ver_text)];
        let _: () = msg_send![content, addSubview: ver];

        // Developer
        let dev: id = msg_send![class!(NSTextField), alloc];
        let dev: id = msg_send![dev, initWithFrame: NSRect::new(NSPoint::new(0.0, 80.0), NSSize::new(360.0, 20.0))];
        let _: () = msg_send![dev, setBezeled: NO];
        let _: () = msg_send![dev, setDrawsBackground: NO];
        let _: () = msg_send![dev, setEditable: NO];
        let _: () = msg_send![dev, setSelectable: NO];
        let _: () = msg_send![dev, setAlignment: 1i64];
        let _: () = msg_send![dev, setStringValue: menu_bridge::nsstring("开发者: liupx")];
        let small_font: id = msg_send![class!(NSFont), systemFontOfSize: 12.0f64];
        let _: () = msg_send![dev, setFont: small_font];
        let _: () = msg_send![content, addSubview: dev];

        // GitHub URL
        let url: id = msg_send![class!(NSTextField), alloc];
        let url: id = msg_send![url, initWithFrame: NSRect::new(NSPoint::new(0.0, 55.0), NSSize::new(360.0, 20.0))];
        let _: () = msg_send![url, setBezeled: NO];
        let _: () = msg_send![url, setDrawsBackground: NO];
        let _: () = msg_send![url, setEditable: NO];
        let _: () = msg_send![url, setSelectable: YES];
        let _: () = msg_send![url, setAlignment: 1i64];
        let _: () = msg_send![url, setStringValue: menu_bridge::nsstring("https://github.com/pupkit-labs/echo-pup-rust")];
        let _: () = msg_send![url, setFont: small_font];
        let _: () = msg_send![content, addSubview: url];

        menu_bridge::show_window(window);
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
        let mut llm_form_popup = LlmFormPopupState::default();

        info!("macOS 状态栏指示器已启动");

        while !should_exit {
            while let Ok(tag) = menu_rx.try_recv() {
                match tag {
                    menu_bridge::TAG_EDIT_LLM_FORM => {
                        open_llm_form_popup(&menu_handles, &latest_snapshot, &mut llm_form_popup);
                        continue;
                    }
                    menu_bridge::TAG_LLM_FORM_CANCEL => {
                        close_llm_form_popup(&mut llm_form_popup);
                        continue;
                    }
                    menu_bridge::TAG_LLM_PROVIDER_CHANGED => {
                        if let Some(dialog) = llm_form_popup.dialog {
                            menu_bridge::update_key_field_visibility(&dialog);
                        }
                        continue;
                    }
                    menu_bridge::TAG_LLM_FORM_CONFIRM => {
                        if let Some(dialog) = llm_form_popup.dialog {
                            let (provider, model, api_base, api_key) =
                                menu_bridge::read_llm_form_values(&dialog);
                            close_llm_form_popup(&mut llm_form_popup);
                            send_child_message(&ChildMessage::ActionRequest {
                                action: MenuAction::SetLlmConfig {
                                    provider,
                                    model,
                                    api_base,
                                    api_key,
                                },
                            });
                        }
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
                        }
                        ParentMessage::SetActionResult { result } => {
                            let _ok = result.ok;
                            let _message = result.message;
                            latest_snapshot = result.snapshot;
                            menu_bridge::update_menu(&menu_handles, &latest_snapshot);
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
                app.sendEvent_(event);
            }

            std::thread::sleep(std::time::Duration::from_millis(40));
        }

        close_llm_form_popup(&mut llm_form_popup);
        status_bar.removeStatusItem_(status_item);
    }

    Ok(())
}

#[cfg(target_os = "linux")]
enum LinuxIndicatorCommand {
    Message(ParentMessage),
    Exit,
}

#[cfg(target_os = "linux")]
fn parse_linux_indicator_command(line: &str) -> Option<LinuxIndicatorCommand> {
    if let Ok(msg) = serde_json::from_str::<ParentMessage>(line) {
        return Some(LinuxIndicatorCommand::Message(msg));
    }

    let trimmed = line.trim();
    if trimmed.eq_ignore_ascii_case("exit") {
        return Some(LinuxIndicatorCommand::Exit);
    }
    IndicatorState::from_wire(trimmed)
        .map(|state| LinuxIndicatorCommand::Message(ParentMessage::SetState { state }))
}

#[cfg(target_os = "linux")]
fn spawn_linux_stdin_reader() -> std::sync::mpsc::Receiver<LinuxIndicatorCommand> {
    use std::io::{BufRead, BufReader};

    let (tx, rx) = std::sync::mpsc::channel::<LinuxIndicatorCommand>();
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        let reader = BufReader::new(stdin.lock());
        for line in reader.lines() {
            match line {
                Ok(line) => {
                    if let Some(cmd) = parse_linux_indicator_command(&line) {
                        let _ = tx.send(cmd);
                    }
                }
                Err(_) => break,
            }
        }
        let _ = tx.send(LinuxIndicatorCommand::Exit);
    });
    rx
}

#[cfg(target_os = "linux")]
fn load_linux_window_icon_pixbuf() -> Option<gtk::gdk_pixbuf::Pixbuf> {
    use gtk::prelude::*;

    let loader = gtk::gdk_pixbuf::PixbufLoader::new();
    if loader.write(STATUS_LOGO_PNG).is_err() {
        return None;
    }
    if loader.close().is_err() {
        return None;
    }
    loader.pixbuf()
}

#[cfg(target_os = "linux")]
fn apply_linux_dialog_icon(dialog: &gtk::Dialog) {
    use gtk::prelude::*;

    let Some(icon) = load_linux_window_icon_pixbuf() else {
        warn!("加载 Linux 弹窗图标失败，将继续使用系统默认图标");
        return;
    };
    dialog.set_icon(Some(&icon));
}

#[cfg(target_os = "linux")]
const LINUX_TRAY_ICON_SIZE: u32 = 96;
#[cfg(target_os = "linux")]
const LINUX_TRAY_IDLE_MAX_WIDTH_RATIO: f32 = 0.92;
#[cfg(target_os = "linux")]
const LINUX_TRAY_IDLE_MAX_HEIGHT_RATIO: f32 = 0.92;
#[cfg(target_os = "linux")]
const LINUX_TRAY_ACTIVE_MIC_MAX_WIDTH_RATIO: f32 = 0.46;
#[cfg(target_os = "linux")]
const LINUX_TRAY_ACTIVE_MIC_MAX_HEIGHT_RATIO: f32 = 0.70;
#[cfg(target_os = "linux")]
const LINUX_TRAY_ACTIVE_PILL_WIDTH_RATIO: f32 = 1.0;
#[cfg(target_os = "linux")]
const LINUX_TRAY_ACTIVE_PILL_HEIGHT_RATIO: f32 = 0.78;
#[cfg(target_os = "linux")]
const LINUX_TRAY_BACKGROUND_CORNER_RATIO: f32 = 0.50;
#[cfg(target_os = "linux")]
const LINUX_TRAY_ALPHA_TRIM_THRESHOLD: u8 = 8;

#[cfg(target_os = "linux")]
fn linux_foreground_png(state: IndicatorState) -> Option<&'static [u8]> {
    match state {
        IndicatorState::Idle => Some(STATUS_LOGO_PNG),
        IndicatorState::RecordingStart
        | IndicatorState::Recording
        | IndicatorState::Transcribing
        | IndicatorState::Completed
        | IndicatorState::Failed => Some(STATUS_MICROPHONE_PNG),
    }
}

#[cfg(target_os = "linux")]
fn build_linux_icon(state: IndicatorState, phase: f32) -> Result<tray_icon::Icon> {
    use image::imageops::{overlay, resize, FilterType};
    use image::{Rgba, RgbaImage};

    let mut canvas = RgbaImage::from_pixel(
        LINUX_TRAY_ICON_SIZE,
        LINUX_TRAY_ICON_SIZE,
        Rgba([0, 0, 0, 0]),
    );

    if let Some(style) = linux_pill_style(state, phase) {
        draw_rounded_rect_linux(
            &mut canvas,
            style.fill.map(Rgba),
            style.border.map(|(color, width)| (Rgba(color), width)),
            style.width_scale,
            style.height_scale,
            LINUX_TRAY_BACKGROUND_CORNER_RATIO,
        );
    }

    if let Some(base_png) = linux_foreground_png(state) {
        let image = image::load_from_memory_with_format(base_png, image::ImageFormat::Png)
            .map_err(|err| anyhow::anyhow!("解码 Linux 托盘 PNG 图标失败: {}", err))?
            .into_rgba8();
        let trimmed = trim_transparent_edges_linux(&image).unwrap_or(image);
        let (target_width, target_height) = if matches!(state, IndicatorState::Idle) {
            let max_width =
                ((LINUX_TRAY_ICON_SIZE as f32 * LINUX_TRAY_IDLE_MAX_WIDTH_RATIO).round() as u32)
                    .max(1);
            let max_height = ((LINUX_TRAY_ICON_SIZE as f32 * LINUX_TRAY_IDLE_MAX_HEIGHT_RATIO)
                .round() as u32)
                .max(1);
            fit_within_rect_linux(trimmed.width(), trimmed.height(), max_width, max_height)
        } else {
            let max_width = ((LINUX_TRAY_ICON_SIZE as f32 * LINUX_TRAY_ACTIVE_MIC_MAX_WIDTH_RATIO)
                .round() as u32)
                .max(1);
            let max_height =
                ((LINUX_TRAY_ICON_SIZE as f32 * LINUX_TRAY_ACTIVE_MIC_MAX_HEIGHT_RATIO).round()
                    as u32)
                    .max(1);
            fit_within_rect_linux(trimmed.width(), trimmed.height(), max_width, max_height)
        };
        let resized = resize(&trimmed, target_width, target_height, FilterType::Lanczos3);
        let offset_x = (LINUX_TRAY_ICON_SIZE as i32 - resized.width() as i32) / 2;
        let offset_y = (LINUX_TRAY_ICON_SIZE as i32 - resized.height() as i32) / 2;
        overlay(
            &mut canvas,
            &resized,
            i64::from(offset_x),
            i64::from(offset_y),
        );
    }

    tray_icon::Icon::from_rgba(
        canvas.into_raw(),
        LINUX_TRAY_ICON_SIZE,
        LINUX_TRAY_ICON_SIZE,
    )
    .map_err(|err| anyhow::anyhow!("创建 Linux 托盘图标失败: {}", err))
}

#[cfg(target_os = "linux")]
fn trim_transparent_edges_linux(image: &image::RgbaImage) -> Option<image::RgbaImage> {
    use image::GenericImageView;

    let (width, height) = image.dimensions();
    let mut min_x = width;
    let mut min_y = height;
    let mut max_x = 0;
    let mut max_y = 0;
    let mut found = false;

    for (x, y, pixel) in image.enumerate_pixels() {
        if pixel[3] < LINUX_TRAY_ALPHA_TRIM_THRESHOLD {
            continue;
        }
        found = true;
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x);
        max_y = max_y.max(y);
    }

    if !found {
        return None;
    }

    Some(
        image
            .view(min_x, min_y, max_x - min_x + 1, max_y - min_y + 1)
            .to_image(),
    )
}

#[cfg(target_os = "linux")]
fn fit_within_rect_linux(width: u32, height: u32, max_width: u32, max_height: u32) -> (u32, u32) {
    if width == 0 || height == 0 {
        return (max_width.max(1), max_height.max(1));
    }

    let width_scale = max_width as f32 / width as f32;
    let height_scale = max_height as f32 / height as f32;
    let scale = width_scale.min(height_scale);
    (
        ((width as f32 * scale).round() as u32).max(1),
        ((height as f32 * scale).round() as u32).max(1),
    )
}

#[cfg(target_os = "linux")]
#[derive(Debug, Clone, Copy)]
struct LinuxPillStyle {
    fill: Option<[u8; 4]>,
    border: Option<([u8; 4], f32)>,
    width_scale: f32,
    height_scale: f32,
}

#[cfg(target_os = "linux")]
fn linux_tray_state_pulses(state: IndicatorState) -> bool {
    matches!(
        state,
        IndicatorState::RecordingStart
            | IndicatorState::Recording
            | IndicatorState::Transcribing
            | IndicatorState::Completed
    )
}

#[cfg(target_os = "linux")]
fn linux_pill_style(state: IndicatorState, phase: f32) -> Option<LinuxPillStyle> {
    let wave = if linux_tray_state_pulses(state) {
        pulse_wave(phase)
    } else {
        1.0
    };

    match state {
        IndicatorState::Idle => None,
        IndicatorState::RecordingStart | IndicatorState::Recording => Some(LinuxPillStyle {
            fill: Some([245, 92, 52, (120.0 + 60.0 * wave) as u8]),
            border: Some(([245, 92, 52, (180.0 + 60.0 * wave) as u8], 3.0 + 2.0 * wave)),
            width_scale: (LINUX_TRAY_ACTIVE_PILL_WIDTH_RATIO - 0.03) + 0.03 * wave,
            height_scale: (LINUX_TRAY_ACTIVE_PILL_HEIGHT_RATIO - 0.04) + 0.04 * wave,
        }),
        IndicatorState::Transcribing => Some(LinuxPillStyle {
            fill: Some([250, 168, 39, (120.0 + 60.0 * wave) as u8]),
            border: Some((
                [250, 168, 39, (180.0 + 60.0 * wave) as u8],
                3.0 + 1.8 * wave,
            )),
            width_scale: (LINUX_TRAY_ACTIVE_PILL_WIDTH_RATIO - 0.03) + 0.02 * wave,
            height_scale: (LINUX_TRAY_ACTIVE_PILL_HEIGHT_RATIO - 0.04) + 0.03 * wave,
        }),
        IndicatorState::Completed => Some(LinuxPillStyle {
            fill: Some([66, 186, 101, (120.0 + 60.0 * wave) as u8]),
            border: Some((
                [66, 186, 101, (180.0 + 60.0 * wave) as u8],
                3.0 + 1.8 * wave,
            )),
            width_scale: (LINUX_TRAY_ACTIVE_PILL_WIDTH_RATIO - 0.03) + 0.02 * wave,
            height_scale: (LINUX_TRAY_ACTIVE_PILL_HEIGHT_RATIO - 0.04) + 0.03 * wave,
        }),
        IndicatorState::Failed => Some(LinuxPillStyle {
            fill: None,
            border: Some(([228, 87, 58, 224], 3.2)),
            width_scale: LINUX_TRAY_ACTIVE_PILL_WIDTH_RATIO - 0.02,
            height_scale: LINUX_TRAY_ACTIVE_PILL_HEIGHT_RATIO - 0.04,
        }),
    }
}

#[cfg(target_os = "linux")]
fn draw_rounded_rect_linux(
    canvas: &mut image::RgbaImage,
    fill: Option<image::Rgba<u8>>,
    border: Option<(image::Rgba<u8>, f32)>,
    width_scale: f32,
    height_scale: f32,
    corner_ratio: f32,
) {
    let Some(geom) = linux_rounded_rect_geometry(canvas, width_scale, height_scale, corner_ratio)
    else {
        return;
    };

    for (x, y, pixel) in canvas.enumerate_pixels_mut() {
        let px = x as f32 + 0.5;
        let py = y as f32 + 0.5;

        let in_outer = linux_point_in_rounded_rect(px, py, &geom);
        if !in_outer {
            continue;
        }

        let mut painted = false;
        if let Some(color) = fill {
            *pixel = color;
            painted = true;
        }

        if let Some((color, border_width)) = border {
            let inner = geom.inset(border_width);
            let in_inner = inner
                .as_ref()
                .map(|inner| linux_point_in_rounded_rect(px, py, inner))
                .unwrap_or(false);
            if !in_inner {
                *pixel = color;
                painted = true;
            }
        }

        if !painted {
            *pixel = image::Rgba([0, 0, 0, 0]);
        }
    }
}

#[cfg(target_os = "linux")]
#[derive(Debug, Clone, Copy)]
struct LinuxRoundedRectGeometry {
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
    radius: f32,
}

#[cfg(target_os = "linux")]
impl LinuxRoundedRectGeometry {
    fn inset(self, amount: f32) -> Option<Self> {
        let inset_left = self.left + amount;
        let inset_top = self.top + amount;
        let inset_right = self.right - amount;
        let inset_bottom = self.bottom - amount;
        if inset_right <= inset_left || inset_bottom <= inset_top {
            return None;
        }

        let width = inset_right - inset_left;
        let height = inset_bottom - inset_top;
        let max_radius = (width.min(height) * 0.5).max(0.0);
        Some(Self {
            left: inset_left,
            top: inset_top,
            right: inset_right,
            bottom: inset_bottom,
            radius: self.radius.min(max_radius),
        })
    }
}

#[cfg(target_os = "linux")]
fn linux_rounded_rect_geometry(
    canvas: &image::RgbaImage,
    width_scale: f32,
    height_scale: f32,
    corner_ratio: f32,
) -> Option<LinuxRoundedRectGeometry> {
    let Some(side) = i32::try_from(canvas.width().min(canvas.height())).ok() else {
        return None;
    };
    let rect_width = ((side as f32 * width_scale).round() as i32).clamp(1, side);
    let rect_height = ((side as f32 * height_scale).round() as i32).clamp(1, side);
    let left = (side - rect_width) / 2;
    let top = (side - rect_height) / 2;
    let right = left + rect_width;
    let bottom = top + rect_height;
    let radius = (rect_height as f32 * corner_ratio).clamp(2.0, rect_height as f32 * 0.5);

    Some(LinuxRoundedRectGeometry {
        left: left as f32,
        top: top as f32,
        right: right as f32,
        bottom: bottom as f32,
        radius,
    })
}

#[cfg(target_os = "linux")]
fn linux_point_in_rounded_rect(px: f32, py: f32, geom: &LinuxRoundedRectGeometry) -> bool {
    let inner_left = geom.left + geom.radius;
    let inner_right = geom.right - geom.radius;
    let inner_top = geom.top + geom.radius;
    let inner_bottom = geom.bottom - geom.radius;
    let nearest_x = px.clamp(inner_left.min(inner_right), inner_left.max(inner_right));
    let nearest_y = py.clamp(inner_top.min(inner_bottom), inner_top.max(inner_bottom));
    let dx = px - nearest_x;
    let dy = py - nearest_y;
    dx * dx + dy * dy <= geom.radius * geom.radius
}

#[cfg(target_os = "linux")]
const MENU_ID_TOGGLE_LLM: &str = "toggle_llm";
#[cfg(target_os = "linux")]
const MENU_ID_TOGGLE_CORRECTION: &str = "toggle_correction";
#[cfg(target_os = "linux")]
const MENU_ID_MODE_HOLD: &str = "mode_hold";
#[cfg(target_os = "linux")]
const MENU_ID_MODE_TOGGLE: &str = "mode_toggle";
#[cfg(target_os = "linux")]
const MENU_ID_RELOAD_CONFIG: &str = "reload_config";
#[cfg(target_os = "linux")]
const MENU_ID_EDIT_LLM_FORM_LINUX: &str = "edit_llm_form_linux";
#[cfg(target_os = "linux")]
const MENU_ID_OPEN_CONFIG_FOLDER: &str = "open_config_folder";
#[cfg(target_os = "linux")]
const MENU_ID_OPEN_MODEL_FOLDER: &str = "open_model_folder";
#[cfg(target_os = "linux")]
const MENU_ID_QUIT_UI: &str = "quit_ui";

#[cfg(target_os = "linux")]
struct LinuxMenuHandles {
    status_line: muda::MenuItem,
    edit_llm_form: muda::MenuItem,
    llm_enabled: muda::CheckMenuItem,
    correction_enabled: muda::CheckMenuItem,
    mode_hold: muda::CheckMenuItem,
    mode_toggle: muda::CheckMenuItem,
}

#[cfg(target_os = "linux")]
#[derive(Debug, Default)]
struct LlmFormPopupLinux {
    is_editing: bool,
}

#[cfg(target_os = "linux")]
fn open_llm_form_popup_linux(
    snapshot: &MenuSnapshot,
    popup: &mut LlmFormPopupLinux,
) -> Option<(String, String, String, String)> {
    use gtk::prelude::*;

    popup.is_editing = true;
    let dialog = gtk::Dialog::with_buttons(
        Some("编辑 LLM 配置"),
        None::<&gtk::Window>,
        gtk::DialogFlags::MODAL,
        &[
            ("取消", gtk::ResponseType::Cancel),
            ("确认", gtk::ResponseType::Ok),
        ],
    );
    apply_linux_dialog_icon(&dialog);

    let content = dialog.content_area();
    content.set_spacing(8);

    let grid = gtk::Grid::new();
    grid.set_row_spacing(8);
    grid.set_column_spacing(10);
    grid.set_margin_top(10);
    grid.set_margin_bottom(6);
    grid.set_margin_start(10);
    grid.set_margin_end(10);
    content.pack_start(&grid, true, true, 0);

    let provider_label = gtk::Label::new(Some("模型提供商"));
    provider_label.set_xalign(0.0);
    let provider_combo = gtk::ComboBoxText::new();
    provider_combo.append(Some("openai"), "OpenAI 兼容接口");
    provider_combo.append(Some("anthropic"), "Anthropic 兼容接口");
    provider_combo.append(Some("ollama"), "Ollama 本地");
    let provider_id = match snapshot.llm_provider.as_str() {
        "anthropic" => "anthropic",
        "ollama" => "ollama",
        _ => "openai",
    };
    provider_combo.set_active_id(Some(provider_id));

    let model_label = gtk::Label::new(Some("模型名称"));
    model_label.set_xalign(0.0);
    let model_entry = gtk::Entry::new();
    model_entry.set_text(&snapshot.llm_model);

    let api_base_label = gtk::Label::new(Some("接口地址"));
    api_base_label.set_xalign(0.0);
    let api_base_entry = gtk::Entry::new();
    api_base_entry.set_text(&snapshot.llm_api_base);

    let api_key_label = gtk::Label::new(Some("API 密钥"));
    api_key_label.set_xalign(0.0);
    let api_key_entry = gtk::Entry::new();
    api_key_entry.set_visibility(false);
    api_key_entry.set_invisible_char(Some('*'));
    api_key_entry.set_text(&snapshot.llm_api_key);

    grid.attach(&provider_label, 0, 0, 1, 1);
    grid.attach(&provider_combo, 1, 0, 1, 1);
    grid.attach(&model_label, 0, 1, 1, 1);
    grid.attach(&model_entry, 1, 1, 1, 1);
    grid.attach(&api_base_label, 0, 2, 1, 1);
    grid.attach(&api_base_entry, 1, 2, 1, 1);
    grid.attach(&api_key_label, 0, 3, 1, 1);
    grid.attach(&api_key_entry, 1, 3, 1, 1);

    let is_ollama = provider_id == "ollama";
    api_key_label.set_visible(!is_ollama);
    api_key_entry.set_visible(!is_ollama);

    // Dynamic show/hide when provider changes
    let key_label_clone = api_key_label.clone();
    let key_entry_clone = api_key_entry.clone();
    provider_combo.connect_changed(move |combo| {
        let hide = combo
            .active_id()
            .map(|id| id.as_str() == "ollama")
            .unwrap_or(false);
        key_label_clone.set_visible(!hide);
        key_entry_clone.set_visible(!hide);
    });

    if let Some(widget) = dialog.widget_for_response(gtk::ResponseType::Ok) {
        widget.grab_default();
    }

    dialog.show_all();
    let response = dialog.run();
    let provider = provider_combo
        .active_id()
        .map(|v| v.to_string())
        .unwrap_or_else(|| snapshot.llm_provider.clone());
    let model = model_entry.text().trim().to_string();
    let api_base = api_base_entry.text().trim().to_string();
    let api_key = api_key_entry.text().trim().to_string();
    dialog.close();
    popup.is_editing = false;

    if response != gtk::ResponseType::Ok {
        return None;
    }
    Some((provider, model, api_base, api_key))
}

#[cfg(target_os = "linux")]
fn build_linux_menu() -> Result<(muda::Menu, LinuxMenuHandles)> {
    let menu = muda::Menu::new();

    let status_line = muda::MenuItem::new("状态: 就绪", false, None);
    let llm_enabled =
        muda::CheckMenuItem::with_id(MENU_ID_TOGGLE_LLM, "启用 LLM 润色", true, false, None);
    let correction_enabled =
        muda::CheckMenuItem::with_id(MENU_ID_TOGGLE_CORRECTION, "启用文本纠错", true, false, None);

    menu.append(&status_line)?;
    menu.append(&muda::PredefinedMenuItem::separator())?;
    menu.append(&llm_enabled)?;
    let edit_llm_form =
        muda::MenuItem::with_id(MENU_ID_EDIT_LLM_FORM_LINUX, "编辑 LLM 配置", true, None);
    menu.append(&edit_llm_form)?;
    menu.append(&correction_enabled)?;
    menu.append(&muda::PredefinedMenuItem::separator())?;

    let mode_hold = muda::CheckMenuItem::with_id(MENU_ID_MODE_HOLD, "长按模式", true, false, None);
    let mode_toggle =
        muda::CheckMenuItem::with_id(MENU_ID_MODE_TOGGLE, "按压切换模式", true, false, None);
    let mode_submenu = muda::Submenu::new("热键触发模式", true);
    mode_submenu.append(&mode_hold)?;
    mode_submenu.append(&mode_toggle)?;
    menu.append(&mode_submenu)?;

    menu.append(&muda::PredefinedMenuItem::separator())?;
    menu.append(&muda::MenuItem::with_id(
        MENU_ID_OPEN_CONFIG_FOLDER,
        "打开配置文件夹",
        true,
        None,
    ))?;
    menu.append(&muda::MenuItem::with_id(
        MENU_ID_OPEN_MODEL_FOLDER,
        "查看模型文件",
        true,
        None,
    ))?;
    menu.append(&muda::MenuItem::with_id(
        MENU_ID_RELOAD_CONFIG,
        "重载配置",
        true,
        None,
    ))?;
    menu.append(&muda::PredefinedMenuItem::separator())?;
    menu.append(&muda::MenuItem::new("关于 EchoPup", false, None))?;
    menu.append(&muda::MenuItem::with_id(
        MENU_ID_QUIT_UI,
        "退出",
        true,
        None,
    ))?;

    Ok((
        menu,
        LinuxMenuHandles {
            status_line,
            edit_llm_form,
            llm_enabled,
            correction_enabled,
            mode_hold,
            mode_toggle,
        },
    ))
}

#[cfg(target_os = "linux")]
fn update_linux_menu(handles: &LinuxMenuHandles, snapshot: &MenuSnapshot, state: IndicatorState) {
    let mut status_text = linux_status_text(snapshot, state);
    if status_text.chars().count() > 56 {
        status_text = format!("{}...", status_text.chars().take(56).collect::<String>());
    }

    handles
        .status_line
        .set_text(format!("状态: {}", status_text.replace('\n', " ")));
    handles.edit_llm_form.set_text(format!(
        "编辑 LLM 配置 ({}/{})",
        snapshot.llm_provider, snapshot.llm_model
    ));
    handles.llm_enabled.set_checked(snapshot.llm_enabled);
    // 始终可用，用户需先配置再启用
    handles
        .correction_enabled
        .set_checked(snapshot.text_correction_enabled);
    handles
        .mode_hold
        .set_checked(snapshot.hotkey_trigger_mode == HotkeyTriggerMode::HoldToRecord);
    handles
        .mode_toggle
        .set_checked(snapshot.hotkey_trigger_mode == HotkeyTriggerMode::PressToToggle);

    // Whisper 模型切换和下载相关代码已移除
}

#[cfg(target_os = "linux")]
fn linux_status_text(snapshot: &MenuSnapshot, state: IndicatorState) -> String {
    match state {
        IndicatorState::RecordingStart
        | IndicatorState::Recording
        | IndicatorState::Transcribing => state.menu_title().to_string(),
        IndicatorState::Completed | IndicatorState::Failed => {
            let status = snapshot.status.trim();
            if status.is_empty() {
                state.menu_title().to_string()
            } else {
                status.to_string()
            }
        }
        IndicatorState::Idle => {
            let status = snapshot.status.trim();
            if status.is_empty() {
                state.menu_title().to_string()
            } else {
                status.to_string()
            }
        }
    }
}

#[cfg(target_os = "linux")]
fn linux_tooltip_text(snapshot: &MenuSnapshot, state: IndicatorState) -> String {
    format!("EchoPup - {}", linux_status_text(snapshot, state))
}

#[cfg(target_os = "linux")]
fn try_update_linux_tray_icon(
    tray_icon: &tray_icon::TrayIcon,
    state: IndicatorState,
    phase: f32,
    context: &str,
) {
    match build_linux_icon(state, phase) {
        Ok(icon) => {
            if let Err(err) = tray_icon.set_icon(Some(icon)) {
                warn!("{}失败: {}", context, err);
            }
        }
        Err(err) => {
            warn!("{}失败: {}", context, err);
        }
    }
}

#[cfg(target_os = "linux")]
fn map_linux_menu_id_to_action(id: &str) -> Option<MenuAction> {
    match id {
        MENU_ID_TOGGLE_LLM => Some(MenuAction::ToggleLlmEnabled),
        MENU_ID_TOGGLE_CORRECTION => Some(MenuAction::ToggleTextCorrectionEnabled),
        MENU_ID_OPEN_CONFIG_FOLDER => Some(MenuAction::OpenConfigFolder),
        MENU_ID_OPEN_MODEL_FOLDER => Some(MenuAction::OpenModelFolder),
        MENU_ID_MODE_HOLD => Some(MenuAction::SetHotkeyTriggerMode {
            mode: HotkeyTriggerMode::HoldToRecord,
        }),
        MENU_ID_MODE_TOGGLE => Some(MenuAction::SetHotkeyTriggerMode {
            mode: HotkeyTriggerMode::PressToToggle,
        }),
        MENU_ID_RELOAD_CONFIG => Some(MenuAction::ReloadConfig),
        MENU_ID_QUIT_UI => Some(MenuAction::QuitUi),
        _ => None,
    }
}

#[cfg(target_os = "linux")]
fn expand_tilde_path_linux(path: &str) -> std::path::PathBuf {
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

#[cfg(target_os = "linux")]
pub(crate) fn open_config_folder_linux(config_path: &str) -> Result<()> {
    let path = expand_tilde_path_linux(config_path);
    let dir = if path.is_dir() {
        path
    } else {
        path.parent().unwrap_or(path.as_path()).to_path_buf()
    };

    let status = std::process::Command::new("xdg-open").arg(&dir).status()?;
    if !status.success() {
        anyhow::bail!("无法打开目录: {}", dir.display());
    }
    Ok(())
}

#[cfg(target_os = "linux")]
pub(crate) fn open_model_folder_linux() -> Result<()> {
    let dir = crate::runtime::model_dir()?;
    std::fs::create_dir_all(&dir)?;
    let status = std::process::Command::new("xdg-open").arg(&dir).status()?;
    if !status.success() {
        anyhow::bail!("无法打开目录: {}", dir.display());
    }
    Ok(())
}

#[cfg(target_os = "linux")]
pub fn run_status_indicator_process() -> Result<()> {
    use tray_icon::TrayIconBuilder;

    gtk::init().map_err(|err| anyhow::anyhow!("初始化 GTK 失败: {}", err))?;
    if let Some(icon) = load_linux_window_icon_pixbuf() {
        gtk::Window::set_default_icon(&icon);
    } else {
        warn!("设置 Linux 默认窗口图标失败，将继续使用系统默认图标");
    }

    let rx = spawn_linux_stdin_reader();
    let (menu, handles) = build_linux_menu()?;

    let mut state = IndicatorState::Idle;
    let mut snapshot = empty_snapshot();
    let mut llm_form_popup = LlmFormPopupLinux::default();
    update_linux_menu(&handles, &snapshot, state);

    let tray_icon = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_icon(build_linux_icon(state, 0.0)?)
        .with_tooltip("EchoPup 状态栏")
        .with_title(linux_status_text(&snapshot, state))
        .build()?;

    let mut pulse_phase = 0.0f32;
    let mut auto_back_to_idle_deadline: Option<std::time::Instant> = None;
    let mut should_exit = false;
    info!("Linux 托盘状态指示器已启动");

    while !should_exit {
        while let Ok(event) = muda::MenuEvent::receiver().try_recv() {
            if event.id.as_ref() == MENU_ID_EDIT_LLM_FORM_LINUX {
                if let Some((provider, model, api_base, api_key)) =
                    open_llm_form_popup_linux(&snapshot, &mut llm_form_popup)
                {
                    send_child_message(&ChildMessage::ActionRequest {
                        action: MenuAction::SetLlmConfig {
                            provider,
                            model,
                            api_base,
                            api_key,
                        },
                    });
                }
                continue;
            }

            if let Some(action) = map_linux_menu_id_to_action(event.id.as_ref()) {
                send_child_message(&ChildMessage::ActionRequest { action });
            }
        }

        while let Ok(cmd) = rx.try_recv() {
            match cmd {
                LinuxIndicatorCommand::Message(message) => match message {
                    ParentMessage::SetState { state: next_state } => {
                        state = next_state;
                        pulse_phase = 0.0;
                        auto_back_to_idle_deadline = state
                            .auto_reset_duration()
                            .map(|duration| std::time::Instant::now() + duration);
                        let _ = tray_icon.set_tooltip(Some(&linux_tooltip_text(&snapshot, state)));
                        let _ = tray_icon.set_title(Some(linux_status_text(&snapshot, state)));
                        try_update_linux_tray_icon(
                            &tray_icon,
                            state,
                            pulse_phase,
                            "更新 Linux 托盘图标",
                        );
                        update_linux_menu(&handles, &snapshot, state);
                    }
                    ParentMessage::SetSnapshot {
                        snapshot: next_snapshot,
                    } => {
                        snapshot = next_snapshot;
                        let _ = tray_icon.set_tooltip(Some(&linux_tooltip_text(&snapshot, state)));
                        update_linux_menu(&handles, &snapshot, state);
                    }
                    ParentMessage::SetActionResult { result } => {
                        if !result.ok {
                            warn!("状态栏动作执行失败: {}", result.message);
                        }
                        snapshot = result.snapshot;
                        let _ = tray_icon.set_tooltip(Some(&linux_tooltip_text(&snapshot, state)));
                        update_linux_menu(&handles, &snapshot, state);
                    }
                    ParentMessage::Exit => {
                        should_exit = true;
                        break;
                    }
                },
                LinuxIndicatorCommand::Exit => {
                    should_exit = true;
                    break;
                }
            }
        }

        if linux_tray_state_pulses(state) {
            pulse_phase = (pulse_phase + 0.15) % std::f32::consts::TAU;
            // 使用 set_tooltip 来同步 GNOME AppIndicator 的内部状态
            // 这有助于减少 AppIndicator 的渲染问题
            let tooltip_text = linux_tooltip_text(&snapshot, state);
            let _ = tray_icon.set_tooltip(Some(&tooltip_text));
            try_update_linux_tray_icon(&tray_icon, state, pulse_phase, "更新 Linux 托盘脉冲图标");
        }

        if let Some(deadline) = auto_back_to_idle_deadline {
            if std::time::Instant::now() >= deadline {
                state = IndicatorState::Idle;
                pulse_phase = 0.0;
                auto_back_to_idle_deadline = None;
                let _ = tray_icon.set_tooltip(Some(&linux_tooltip_text(&snapshot, state)));
                try_update_linux_tray_icon(&tray_icon, state, pulse_phase, "重置 Linux 托盘图标");
                update_linux_menu(&handles, &snapshot, state);
            }
        }

        while gtk::events_pending() {
            let _ = gtk::main_iteration_do(false);
        }
        std::thread::sleep(std::time::Duration::from_millis(40));
    }

    drop(tray_icon);
    Ok(())
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn run_status_indicator_process() -> Result<()> {
    anyhow::bail!("status-indicator 仅在 macOS/Linux 可用");
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
            menu_bridge::map_tag_to_action(menu_bridge::TAG_QUIT_UI, &snapshot),
            Some(MenuAction::QuitUi)
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

    #[cfg(target_os = "linux")]
    #[test]
    fn test_linux_active_states_keep_microphone_foreground() {
        assert_eq!(
            linux_foreground_png(IndicatorState::Idle),
            Some(STATUS_LOGO_PNG)
        );
        assert_eq!(
            linux_foreground_png(IndicatorState::Recording),
            Some(STATUS_MICROPHONE_PNG)
        );
        assert_eq!(
            linux_foreground_png(IndicatorState::Transcribing),
            Some(STATUS_MICROPHONE_PNG)
        );
        assert_eq!(
            linux_foreground_png(IndicatorState::Completed),
            Some(STATUS_MICROPHONE_PNG)
        );
        assert_eq!(
            linux_foreground_png(IndicatorState::Failed),
            Some(STATUS_MICROPHONE_PNG)
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_linux_completed_state_uses_pulsing_capsule_outline() {
        assert!(linux_tray_state_pulses(IndicatorState::Completed));

        let first = linux_pill_style(IndicatorState::Completed, 0.0).expect("completed pill");
        let second = linux_pill_style(IndicatorState::Completed, std::f32::consts::FRAC_PI_2)
            .expect("completed pill pulse");

        let (first_color, first_width) = first.border.expect("completed border");
        let (second_color, second_width) = second.border.expect("completed border");
        assert_ne!(first_color[3], second_color[3]);
        assert_ne!(first_width.round() as i32, second_width.round() as i32);
        assert!(first.fill.is_some());
        assert!(second.fill.is_some());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_linux_point_in_rounded_rect_handles_float_inversion() {
        let geom = LinuxRoundedRectGeometry {
            left: 0.0,
            top: 10.5,
            right: 96.0,
            bottom: 85.5,
            radius: 48.000_004,
        };

        let inside = linux_point_in_rounded_rect(48.0, 48.0, &geom);
        assert!(inside);
    }
}
