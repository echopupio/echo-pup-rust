//! 状态栏反馈（第一版：macOS 菜单栏 + stdin 文本消息）

use anyhow::Result;
use tracing::{info, warn};

#[derive(Debug, Clone, Copy)]
pub enum IndicatorState {
    Idle,
    RecordingStart,
    Recording,
    Transcribing,
    Completed,
    Failed,
}

impl IndicatorState {
    fn to_wire(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::RecordingStart => "recording_start",
            Self::Recording => "recording",
            Self::Transcribing => "transcribing",
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }

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
                    };
                }
            };

            let mut child = match std::process::Command::new(exe)
                .arg("--config")
                .arg(config_path)
                .arg("status-indicator")
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::null())
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
                    };
                }
            };

            Self {
                enabled: true,
                child: Some(child),
                stdin: Some(stdin),
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

        #[cfg(target_os = "macos")]
        {
            let Some(stdin) = self.stdin.as_mut() else {
                self.enabled = false;
                return;
            };

            if std::io::Write::write_all(stdin, state.to_wire().as_bytes())
                .and_then(|_| std::io::Write::write_all(stdin, b"\n"))
                .and_then(|_| std::io::Write::flush(stdin))
                .is_err()
            {
                self.enabled = false;
                warn!("向状态栏子进程发送状态失败，状态栏反馈已禁用");
            }
        }
    }
}

impl Drop for StatusIndicatorClient {
    fn drop(&mut self) {
        #[cfg(target_os = "macos")]
        {
            if let Some(stdin) = self.stdin.as_mut() {
                let _ = std::io::Write::write_all(stdin, b"exit\n");
                let _ = std::io::Write::flush(stdin);
            }
            self.stdin.take();

            if let Some(mut child) = self.child.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }
}

#[cfg(target_os = "macos")]
enum IndicatorCommand {
    Set(IndicatorState),
    Exit,
}

#[cfg(target_os = "macos")]
fn parse_indicator_command(line: &str) -> Option<IndicatorCommand> {
    let trimmed = line.trim();
    if trimmed.eq_ignore_ascii_case("exit") {
        return Some(IndicatorCommand::Exit);
    }
    IndicatorState::from_wire(trimmed).map(IndicatorCommand::Set)
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
unsafe fn set_status_title(button: cocoa::base::id, title: &str) {
    use cocoa::base::nil;
    use cocoa::foundation::NSString;
    use objc::{msg_send, sel, sel_impl};

    if button == nil {
        return;
    }
    let ns_title = NSString::alloc(nil).init_str(title);
    #[allow(unexpected_cfgs)]
    let _: () = msg_send![button, setTitle: ns_title];
}

#[cfg(target_os = "macos")]
unsafe fn load_png_image(png: &[u8]) -> cocoa::base::id {
    use cocoa::base::id;
    use objc::{class, msg_send, sel, sel_impl};
    use std::ffi::c_void;

    #[allow(unexpected_cfgs)]
    let data: id = msg_send![
        class!(NSData),
        dataWithBytes: png.as_ptr() as *const c_void
        length: png.len()
    ];
    #[allow(unexpected_cfgs)]
    let image: id = msg_send![class!(NSImage), alloc];
    #[allow(unexpected_cfgs)]
    let image: id = msg_send![image, initWithData: data];
    if image != cocoa::base::nil {
        #[allow(unexpected_cfgs)]
        let _: () = msg_send![image, setTemplate: cocoa::base::NO];
    }
    image
}

#[cfg(target_os = "macos")]
unsafe fn compose_status_image(logo: cocoa::base::id, mic: cocoa::base::id, show_mic: bool) -> cocoa::base::id {
    use cocoa::base::{id, nil};
    use cocoa::foundation::{NSPoint, NSRect, NSSize};
    use objc::{class, msg_send, sel, sel_impl};

    if logo == nil {
        return nil;
    }

    let width = STATUS_ICON_SIZE;
    let canvas_size = NSSize::new(width, STATUS_ICON_SIZE);
    #[allow(unexpected_cfgs)]
    let canvas: id = msg_send![class!(NSImage), alloc];
    #[allow(unexpected_cfgs)]
    let canvas: id = msg_send![canvas, initWithSize: canvas_size];
    if canvas == nil {
        return nil;
    }
    #[allow(unexpected_cfgs)]
    let _: () = msg_send![canvas, setTemplate: cocoa::base::NO];
    #[allow(unexpected_cfgs)]
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
    #[allow(unexpected_cfgs)]
    let clip_path: id = msg_send![
        class!(NSBezierPath),
        bezierPathWithRoundedRect: clip_rect
        xRadius: clip_radius
        yRadius: clip_radius
    ];
    #[allow(unexpected_cfgs)]
    let _: () = msg_send![clip_path, addClip];

    #[allow(unexpected_cfgs)]
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
    let scale = (clip_rect.size.width / src_w)
        .min(clip_rect.size.height / src_h)
        * visual_scale;
    let draw_w = src_w * scale;
    let draw_h = src_h * scale;
    let draw_rect = NSRect::new(
        NSPoint::new(
            clip_rect.origin.x + (clip_rect.size.width - draw_w) * 0.5,
            clip_rect.origin.y + (clip_rect.size.height - draw_h) * 0.5,
        ),
        NSSize::new(draw_w, draw_h),
    );
    #[allow(unexpected_cfgs)]
    let _: () = msg_send![
        image,
        drawInRect: draw_rect
        fromRect: zero_rect
        operation: 2isize
        fraction: 1.0f64
    ];

    #[allow(unexpected_cfgs)]
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

    #[allow(unexpected_cfgs)]
    let _: () = msg_send![button, setImage: image];
    #[allow(unexpected_cfgs)]
    let _: () = msg_send![button, setImageScaling: 2isize];
    #[allow(unexpected_cfgs)]
    let _: () = msg_send![button, setImagePosition: 1isize];
    #[allow(unexpected_cfgs)]
    let _: () = msg_send![button, setImageHugsTitle: YES];
}

#[cfg(target_os = "macos")]
unsafe fn create_background_layer(button: cocoa::base::id) -> cocoa::base::id {
    use cocoa::base::{id, nil, YES};
    use objc::{class, msg_send, sel, sel_impl};

    if button == nil {
        return nil;
    }
    #[allow(unexpected_cfgs)]
    let _: () = msg_send![button, setWantsLayer: YES];
    #[allow(unexpected_cfgs)]
    let root_layer: id = msg_send![button, layer];
    if root_layer == nil {
        return nil;
    }
    #[allow(unexpected_cfgs)]
    let layer: id = msg_send![class!(CALayer), layer];
    if layer == nil {
        return nil;
    }
    #[allow(unexpected_cfgs)]
    let _: () = msg_send![layer, setMasksToBounds: YES];
    #[allow(unexpected_cfgs)]
    let _: () = msg_send![root_layer, insertSublayer: layer atIndex: 0u64];
    layer
}

#[cfg(target_os = "macos")]
fn pulsing_color(base: (f32, f32, f32), phase: f32) -> (f32, f32, f32, f32) {
    let wave = 0.5 + 0.5 * phase.sin();
    let factor = 0.92 + 0.08 * wave;
    let scale = |v: f32| (v * factor).clamp(0.0, 1.0);
    (scale(base.0), scale(base.1), scale(base.2), 0.12 + 0.08 * wave)
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
    #[allow(unexpected_cfgs)]
    let bounds: NSRect = msg_send![button, bounds];
    let hpad = 3.0f64;
    let vpad = 4.0f64;
    let width = (bounds.size.width - hpad * 2.0).max(0.0);
    let height = (bounds.size.height - vpad * 2.0).max(0.0);
    let frame = NSRect::new(NSPoint::new(hpad, vpad), NSSize::new(width, height));
    #[allow(unexpected_cfgs)]
    let _: () = msg_send![background_layer, setFrame: frame];
    #[allow(unexpected_cfgs)]
    let _: () = msg_send![background_layer, setMasksToBounds: YES];
    #[allow(unexpected_cfgs)]
    let _: () = msg_send![background_layer, setCornerRadius: (height * 0.5)];

    let (r, g, b, a) = match style_color(state.visual_style(), phase) {
        Some(color) => color,
        None => (0.0, 0.0, 0.0, 0.0),
    };

    #[allow(unexpected_cfgs)]
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

    #[allow(unexpected_cfgs)]
    let cg_color: id = msg_send![ns_color, CGColor];
    #[allow(unexpected_cfgs)]
    let _: () = msg_send![background_layer, setBackgroundColor: cg_color];
    let border_alpha = (a * 4.0).clamp(0.0, 0.88);
    #[allow(unexpected_cfgs)]
    let border_ns_color: id = msg_send![
        class!(NSColor),
        colorWithCalibratedRed: r as f64
        green: g as f64
        blue: b as f64
        alpha: border_alpha as f64
    ];
    #[allow(unexpected_cfgs)]
    let border_cg_color: id = msg_send![border_ns_color, CGColor];
    let border_width = if a <= 0.001 {
        0.0f64
    } else if matches!(state.visual_style(), VisualStyle::CompletedSolid) {
        1.2f64
    } else {
        1.8f64
    };
    #[allow(unexpected_cfgs)]
    let _: () = msg_send![background_layer, setBorderColor: border_cg_color];
    #[allow(unexpected_cfgs)]
    let _: () = msg_send![background_layer, setBorderWidth: border_width];
    #[allow(unexpected_cfgs)]
    let _: () = msg_send![background_layer, setHidden: if a <= 0.001 { YES } else { cocoa::base::NO }];
}

#[cfg(target_os = "macos")]
pub fn run_status_indicator_process() -> Result<()> {
    use cocoa::appkit::{
        NSApp, NSApplication, NSApplicationActivationPolicy, NSEventMask, NSStatusBar,
        NSStatusItem,
        NSVariableStatusItemLength,
    };
    use cocoa::base::{id, nil, YES};
    use cocoa::foundation::{NSAutoreleasePool, NSString};
    use objc::{class, msg_send, sel, sel_impl};

    let rx = spawn_stdin_reader();

    unsafe {
        let _pool = NSAutoreleasePool::new(nil);
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

        info!("macOS 状态栏指示器已启动");

        while !should_exit {
            while let Ok(cmd) = rx.try_recv() {
                match cmd {
                    IndicatorCommand::Set(state) => {
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
                        auto_back_to_idle_deadline =
                            state.auto_reset_duration().map(|d| std::time::Instant::now() + d);
                    }
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
                        apply_button_style(button, background_layer, IndicatorState::Idle, pulse_phase);
                    }
                    auto_back_to_idle_deadline = None;
                }
            }

            #[allow(unexpected_cfgs)]
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
