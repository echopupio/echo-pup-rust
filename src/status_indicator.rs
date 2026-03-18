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
        match self {
            Self::Idle => "⚪️ EchoPup",
            Self::RecordingStart => "🔴 录音开始",
            Self::Recording => "🔴 录音中",
            Self::Transcribing => "🟡 识别中",
            Self::Completed => "🟢 识别完成",
            Self::Failed => "🟠 识别失败",
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
unsafe fn set_status_title(status_item: cocoa::base::id, title: &str) {
    use cocoa::appkit::NSStatusItem;
    use cocoa::base::{id, nil};
    use cocoa::foundation::NSString;
    use objc::{msg_send, sel, sel_impl};

    let button: id = status_item.button();
    if button == nil {
        return;
    }
    let ns_title = NSString::alloc(nil).init_str(title);
    #[allow(unexpected_cfgs)]
    let _: () = msg_send![button, setTitle: ns_title];
}

#[cfg(target_os = "macos")]
pub fn run_status_indicator_process() -> Result<()> {
    use cocoa::appkit::{
        NSApp, NSApplication, NSApplicationActivationPolicy, NSEventMask, NSStatusBar,
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
        set_status_title(status_item, IndicatorState::Idle.menu_title());

        let run_loop_mode = NSString::alloc(nil).init_str("kCFRunLoopDefaultMode");
        let mut auto_back_to_idle_deadline: Option<std::time::Instant> = None;
        let mut should_exit = false;

        info!("macOS 状态栏指示器已启动");

        while !should_exit {
            while let Ok(cmd) = rx.try_recv() {
                match cmd {
                    IndicatorCommand::Set(state) => {
                        set_status_title(status_item, state.menu_title());
                        auto_back_to_idle_deadline = match state {
                            IndicatorState::Completed | IndicatorState::Failed => Some(
                                std::time::Instant::now() + std::time::Duration::from_millis(1400),
                            ),
                            _ => None,
                        };
                    }
                    IndicatorCommand::Exit => {
                        should_exit = true;
                        break;
                    }
                }
            }

            if let Some(deadline) = auto_back_to_idle_deadline {
                if std::time::Instant::now() >= deadline {
                    set_status_title(status_item, IndicatorState::Idle.menu_title());
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
