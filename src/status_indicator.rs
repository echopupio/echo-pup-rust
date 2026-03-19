#![allow(unexpected_cfgs)]
//! 状态栏反馈与菜单 IPC（macOS）

use anyhow::Result;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::menu_core::{EditableField, MenuAction, MenuActionResult, MenuSnapshot};

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
const STATUS_ICON_SIZE: f64 = 40.0;
#[cfg(target_os = "macos")]
const STATUS_LOGO_VISUAL_SCALE: f64 = 1.45;
#[cfg(target_os = "macos")]
const STATUS_MIC_VISUAL_SCALE: f64 = 0.82;
#[cfg(target_os = "macos")]
const STATUS_ICON_H_INSET: f64 = 2.0;
#[cfg(target_os = "macos")]
const STATUS_ICON_V_INSET: f64 = 4.0;

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
            Self::Completed => Some(std::time::Duration::from_secs(5)),
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
fn pulsing_color(base: (f32, f32, f32), phase: f32) -> (f32, f32, f32, f32) {
    let wave = 0.5 + 0.5 * phase.sin();
    let factor = 0.92 + 0.08 * wave;
    let scale = |v: f32| (v * factor).clamp(0.0, 1.0);
    (
        scale(base.0),
        scale(base.1),
        scale(base.2),
        0.12 + 0.08 * wave,
    )
}

#[cfg(target_os = "macos")]
fn style_color(style: VisualStyle, phase: f32) -> Option<(f32, f32, f32, f32)> {
    match style {
        VisualStyle::Idle => None,
        VisualStyle::RecordingPulse => Some(pulsing_color((0.96, 0.33, 0.18), phase)),
        VisualStyle::TranscribingPulse => Some(pulsing_color((0.98, 0.66, 0.15), phase)),
        VisualStyle::CompletedSolid => Some((0.22, 0.72, 0.36, 0.18)),
        VisualStyle::FailedSolid => Some((0.90, 0.42, 0.20, 0.18)),
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
    let hpad = 3.0f64;
    let vpad = 4.0f64;
    let width = (bounds.size.width - hpad * 2.0).max(0.0);
    let height = (bounds.size.height - vpad * 2.0).max(0.0);
    let frame = NSRect::new(NSPoint::new(hpad, vpad), NSSize::new(width, height));
    let _: () = msg_send![background_layer, setFrame: frame];
    let _: () = msg_send![background_layer, setMasksToBounds: YES];
    let _: () = msg_send![background_layer, setCornerRadius: (height * 0.5)];

    let (r, g, b, a) = match style_color(state.visual_style(), phase) {
        Some(color) => color,
        None => (0.0, 0.0, 0.0, 0.0),
    };

    let ns_color: id = msg_send![
        class!(NSColor),
        colorWithCalibratedRed: r as f64
        green: g as f64
        blue: b as f64
        alpha: a as f64
    ];
    if ns_color == nil {
        return;
    }

    let cg_color: id = msg_send![ns_color, CGColor];
    let _: () = msg_send![background_layer, setBackgroundColor: cg_color];
    let border_alpha = (a * 4.0).clamp(0.0, 0.88);
    let border_ns_color: id = msg_send![
        class!(NSColor),
        colorWithCalibratedRed: r as f64
        green: g as f64
        blue: b as f64
        alpha: border_alpha as f64
    ];
    let border_cg_color: id = msg_send![border_ns_color, CGColor];
    let border_width = if a <= 0.001 {
        0.0f64
    } else if matches!(state.visual_style(), VisualStyle::CompletedSolid) {
        1.2f64
    } else {
        1.8f64
    };
    let _: () = msg_send![background_layer, setBorderColor: border_cg_color];
    let _: () = msg_send![background_layer, setBorderWidth: border_width];
    let _: () =
        msg_send![background_layer, setHidden: if a <= 0.001 { YES } else { cocoa::base::NO }];
}

#[cfg(target_os = "macos")]
mod menu_bridge {
    use super::*;
    use cocoa::base::{id, nil, NO};
    use cocoa::foundation::NSString;
    use objc::declare::ClassDecl;
    use objc::runtime::{Class, Object, Sel};
    use objc::{class, msg_send, sel, sel_impl};
    use std::sync::{Mutex, Once, OnceLock};

    pub const TAG_TOGGLE_LLM: i64 = 1001;
    pub const TAG_TOGGLE_CORRECTION: i64 = 1002;
    pub const TAG_TOGGLE_VAD: i64 = 1003;
    pub const TAG_EDIT_HOTKEY: i64 = 1004;
    pub const TAG_EDIT_PROVIDER: i64 = 1005;
    pub const TAG_EDIT_MODEL: i64 = 1006;
    pub const TAG_EDIT_API_BASE: i64 = 1007;
    pub const TAG_EDIT_API_KEY_ENV: i64 = 1008;
    pub const TAG_EDIT_WHISPER_MODEL: i64 = 1009;
    pub const TAG_DOWNLOAD_LARGE: i64 = 1010;
    pub const TAG_DOWNLOAD_TURBO: i64 = 1011;
    pub const TAG_DOWNLOAD_MEDIUM: i64 = 1012;
    pub const TAG_REFRESH_MODELS: i64 = 1013;
    pub const TAG_SAVE_CONFIG: i64 = 1014;
    pub const TAG_QUIT_UI: i64 = 1015;

    static MENU_EVENT_TX: OnceLock<Mutex<Option<std::sync::mpsc::Sender<i64>>>> = OnceLock::new();

    pub struct MenuHandles {
        pub toggle_llm: id,
        pub toggle_correction: id,
        pub toggle_vad: id,
        pub edit_hotkey: id,
        pub edit_provider: id,
        pub edit_model: id,
        pub edit_api_base: id,
        pub edit_api_key_env: id,
        pub edit_whisper_model: id,
        pub status_line: id,
        pub progress_line: id,
        pub log_lines: Vec<id>,
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
            let progress_line = add_info_item(menu, "下载: 空闲");
            let log1 = add_info_item(menu, "日志: -");
            let log2 = add_info_item(menu, "日志: -");
            let log3 = add_info_item(menu, "日志: -");

            add_separator(menu);

            let toggle_llm = add_action_item(menu, target, TAG_TOGGLE_LLM, "切换 LLM 开关");
            let toggle_correction =
                add_action_item(menu, target, TAG_TOGGLE_CORRECTION, "切换文本纠错开关");
            let toggle_vad = add_action_item(menu, target, TAG_TOGGLE_VAD, "切换 VAD 开关");

            add_separator(menu);

            let edit_hotkey = add_action_item(menu, target, TAG_EDIT_HOTKEY, "编辑热键");
            let edit_provider =
                add_action_item(menu, target, TAG_EDIT_PROVIDER, "编辑 LLM provider");
            let edit_model = add_action_item(menu, target, TAG_EDIT_MODEL, "编辑 LLM model");
            let edit_api_base =
                add_action_item(menu, target, TAG_EDIT_API_BASE, "编辑 LLM api_base");
            let edit_api_key_env =
                add_action_item(menu, target, TAG_EDIT_API_KEY_ENV, "编辑 LLM api_key_env");
            let edit_whisper_model = add_action_item(
                menu,
                target,
                TAG_EDIT_WHISPER_MODEL,
                "编辑 Whisper model_path",
            );

            add_separator(menu);

            let _download_large =
                add_action_item(menu, target, TAG_DOWNLOAD_LARGE, "下载模型 large-v3");
            let _download_turbo =
                add_action_item(menu, target, TAG_DOWNLOAD_TURBO, "下载模型 turbo");
            let _download_medium =
                add_action_item(menu, target, TAG_DOWNLOAD_MEDIUM, "下载模型 medium");
            let _refresh_models =
                add_action_item(menu, target, TAG_REFRESH_MODELS, "刷新本地模型列表");
            let _save_config = add_action_item(menu, target, TAG_SAVE_CONFIG, "保存配置");
            let _quit_ui = add_action_item(menu, target, TAG_QUIT_UI, "退出 UI");

            let handles = MenuHandles {
                toggle_llm,
                toggle_correction,
                toggle_vad,
                edit_hotkey,
                edit_provider,
                edit_model,
                edit_api_base,
                edit_api_key_env,
                edit_whisper_model,
                status_line,
                progress_line,
                log_lines: vec![log1, log2, log3],
            };

            (menu, handles)
        }
    }

    pub fn snapshot_for_edit(
        snapshot: &MenuSnapshot,
        tag: i64,
    ) -> Option<(EditableField, String, &'static str)> {
        match tag {
            TAG_EDIT_HOTKEY => Some((EditableField::Hotkey, snapshot.hotkey.clone(), "编辑热键")),
            TAG_EDIT_PROVIDER => Some((
                EditableField::LlmProvider,
                snapshot.llm_provider.clone(),
                "编辑 LLM provider",
            )),
            TAG_EDIT_MODEL => Some((
                EditableField::LlmModel,
                snapshot.llm_model.clone(),
                "编辑 LLM model",
            )),
            TAG_EDIT_API_BASE => Some((
                EditableField::LlmApiBase,
                snapshot.llm_api_base.clone(),
                "编辑 LLM api_base",
            )),
            TAG_EDIT_API_KEY_ENV => Some((
                EditableField::LlmApiKeyEnv,
                snapshot.llm_api_key_env.clone(),
                "编辑 LLM api_key_env",
            )),
            TAG_EDIT_WHISPER_MODEL => Some((
                EditableField::WhisperModelPath,
                snapshot.whisper_model_path.clone(),
                "编辑 Whisper model_path",
            )),
            _ => None,
        }
    }

    pub fn map_tag_to_action(tag: i64, snapshot: &MenuSnapshot) -> Option<MenuAction> {
        match tag {
            TAG_TOGGLE_LLM => Some(MenuAction::ToggleLlmEnabled),
            TAG_TOGGLE_CORRECTION => Some(MenuAction::ToggleTextCorrectionEnabled),
            TAG_TOGGLE_VAD => Some(MenuAction::ToggleVadEnabled),
            TAG_DOWNLOAD_LARGE => Some(MenuAction::DownloadModel {
                size: "large-v3".to_string(),
            }),
            TAG_DOWNLOAD_TURBO => Some(MenuAction::DownloadModel {
                size: "turbo".to_string(),
            }),
            TAG_DOWNLOAD_MEDIUM => Some(MenuAction::DownloadModel {
                size: "medium".to_string(),
            }),
            TAG_REFRESH_MODELS => Some(MenuAction::RefreshLocalModels),
            TAG_SAVE_CONFIG => Some(MenuAction::SaveConfig),
            TAG_QUIT_UI => Some(MenuAction::QuitUi),
            _ => {
                if let Some((field, default_value, title)) = snapshot_for_edit(snapshot, tag) {
                    let value = prompt_input(title, &default_value)?;
                    return Some(MenuAction::SetField { field, value });
                }
                None
            }
        }
    }

    pub fn update_menu(handles: &MenuHandles, snapshot: &MenuSnapshot) {
        unsafe {
            set_check_state(handles.toggle_llm, snapshot.llm_enabled);
            set_check_state(handles.toggle_correction, snapshot.text_correction_enabled);
            set_check_state(handles.toggle_vad, snapshot.vad_enabled);

            set_title(
                handles.edit_hotkey,
                &format!("编辑热键 ({})", snapshot.hotkey),
            );
            set_title(
                handles.edit_provider,
                &format!("编辑 LLM provider ({})", snapshot.llm_provider),
            );
            set_title(
                handles.edit_model,
                &format!("编辑 LLM model ({})", snapshot.llm_model),
            );
            set_title(
                handles.edit_api_base,
                &format!("编辑 LLM api_base ({})", snapshot.llm_api_base),
            );
            set_title(
                handles.edit_api_key_env,
                &format!("编辑 LLM api_key_env ({})", snapshot.llm_api_key_env),
            );
            set_title(
                handles.edit_whisper_model,
                &format!(
                    "编辑 Whisper model_path ({})",
                    shorten_path(&snapshot.whisper_model_path)
                ),
            );

            set_title(handles.status_line, &format!("状态: {}", snapshot.status));
            let progress = snapshot.download.as_ref().map(|d| {
                let (_, label) = crate::model_download::download_ratio_label(d);
                format!("下载 {}: {}", d.model_size, label)
            });
            set_title(
                handles.progress_line,
                progress.as_deref().unwrap_or("下载: 空闲"),
            );

            let logs = snapshot
                .download_logs
                .iter()
                .rev()
                .take(3)
                .cloned()
                .collect::<Vec<_>>();
            for (idx, item) in handles.log_lines.iter().enumerate() {
                let text = logs
                    .get(idx)
                    .cloned()
                    .unwrap_or_else(|| "日志: -".to_string());
                set_title(*item, &format!("日志: {}", text));
            }
        }
    }

    fn shorten_path(path: &str) -> String {
        const MAX: usize = 42;
        if path.len() <= MAX {
            return path.to_string();
        }
        let tail = &path[path.len() - (MAX - 3)..];
        format!("...{}", tail)
    }

    fn prompt_input(title: &str, default_value: &str) -> Option<String> {
        let escaped_title = applescript_escape(title);
        let escaped_default = applescript_escape(default_value);
        let script = format!(
            "display dialog \"{}\" default answer \"{}\" with title \"EchoPup\"",
            escaped_title, escaped_default
        );

        let output = std::process::Command::new("/usr/bin/osascript")
            .arg("-e")
            .arg(script)
            .output()
            .ok()?;

        if !output.status.success() {
            return None;
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let marker = "text returned:";
        let idx = stdout.find(marker)?;
        Some(stdout[idx + marker.len()..].trim().to_string())
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
pub fn run_status_indicator_process() -> Result<()> {
    use cocoa::appkit::{
        NSApp, NSApplication, NSApplicationActivationPolicy, NSEventMask, NSStatusBar,
        NSStatusItem, NSVariableStatusItemLength,
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
        let status_item = status_bar.statusItemWithLength_(NSVariableStatusItemLength);
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

        info!("macOS 状态栏指示器已启动");

        while !should_exit {
            while let Ok(tag) = menu_rx.try_recv() {
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
            menu_bridge::map_tag_to_action(menu_bridge::TAG_DOWNLOAD_LARGE, &snapshot),
            Some(MenuAction::DownloadModel { ref size }) if size == "large-v3"
        ));
        assert!(matches!(
            menu_bridge::map_tag_to_action(menu_bridge::TAG_DOWNLOAD_TURBO, &snapshot),
            Some(MenuAction::DownloadModel { ref size }) if size == "turbo"
        ));
        assert!(matches!(
            menu_bridge::map_tag_to_action(menu_bridge::TAG_DOWNLOAD_MEDIUM, &snapshot),
            Some(MenuAction::DownloadModel { ref size }) if size == "medium"
        ));
        assert!(matches!(
            menu_bridge::map_tag_to_action(menu_bridge::TAG_REFRESH_MODELS, &snapshot),
            Some(MenuAction::RefreshLocalModels)
        ));
        assert!(matches!(
            menu_bridge::map_tag_to_action(menu_bridge::TAG_SAVE_CONFIG, &snapshot),
            Some(MenuAction::SaveConfig)
        ));
        assert!(matches!(
            menu_bridge::map_tag_to_action(menu_bridge::TAG_QUIT_UI, &snapshot),
            Some(MenuAction::QuitUi)
        ));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn test_phase_e_edit_tag_metadata_mapping() {
        let snapshot = empty_snapshot();
        let hotkey = menu_bridge::snapshot_for_edit(&snapshot, menu_bridge::TAG_EDIT_HOTKEY)
            .expect("hotkey edit metadata");
        assert_eq!(hotkey.0, EditableField::Hotkey);
        assert_eq!(hotkey.1, snapshot.hotkey);

        let provider = menu_bridge::snapshot_for_edit(&snapshot, menu_bridge::TAG_EDIT_PROVIDER)
            .expect("provider edit metadata");
        assert_eq!(provider.0, EditableField::LlmProvider);
        assert_eq!(provider.1, snapshot.llm_provider);

        let model = menu_bridge::snapshot_for_edit(&snapshot, menu_bridge::TAG_EDIT_MODEL)
            .expect("model edit metadata");
        assert_eq!(model.0, EditableField::LlmModel);
        assert_eq!(model.1, snapshot.llm_model);

        let api_base = menu_bridge::snapshot_for_edit(&snapshot, menu_bridge::TAG_EDIT_API_BASE)
            .expect("api_base edit metadata");
        assert_eq!(api_base.0, EditableField::LlmApiBase);
        assert_eq!(api_base.1, snapshot.llm_api_base);

        let api_key = menu_bridge::snapshot_for_edit(&snapshot, menu_bridge::TAG_EDIT_API_KEY_ENV)
            .expect("api_key_env edit metadata");
        assert_eq!(api_key.0, EditableField::LlmApiKeyEnv);
        assert_eq!(api_key.1, snapshot.llm_api_key_env);

        let whisper =
            menu_bridge::snapshot_for_edit(&snapshot, menu_bridge::TAG_EDIT_WHISPER_MODEL)
                .expect("whisper model edit metadata");
        assert_eq!(whisper.0, EditableField::WhisperModelPath);
        assert_eq!(whisper.1, snapshot.whisper_model_path);
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
