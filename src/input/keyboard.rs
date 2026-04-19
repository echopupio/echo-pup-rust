//! 键盘输入模拟

#[cfg(target_os = "linux")]
use anyhow::Context;
use anyhow::{anyhow, Result};
use enigo::Enigo;
#[cfg(target_os = "linux")]
use std::process::Command;
use tracing::{error, info, warn};

enum KeyboardBackend {
    Enigo(Enigo),
    #[cfg(target_os = "linux")]
    LinuxCommand(LinuxCommandBackend),
    Unavailable(String),
}

#[cfg(target_os = "linux")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LinuxTypingBackend {
    Eitype,
    Xdotool,
    Wtype,
}

#[cfg(target_os = "linux")]
struct LinuxCommandBackend {
    active: LinuxTypingBackend,
    fallbacks: Vec<LinuxTypingBackend>,
}

#[cfg(target_os = "linux")]
impl LinuxCommandBackend {
    fn from_available_backends(mut available: Vec<LinuxTypingBackend>) -> Option<Self> {
        let active = available.first().copied()?;
        available.remove(0);
        Some(Self {
            active,
            fallbacks: available,
        })
    }

    fn label(&self) -> &'static str {
        self.active.label()
    }

    fn type_text(&mut self, text: &str) -> Result<()> {
        if text.is_empty() {
            return Ok(());
        }
        self.try_backends(|backend| backend.type_text(text))
    }

    fn delete_backward(&mut self, count: usize) -> Result<()> {
        if count == 0 {
            return Ok(());
        }
        self.try_backends(|backend| backend.delete_backward(count))
    }

    fn try_backends<F>(&mut self, mut op: F) -> Result<()>
    where
        F: FnMut(LinuxTypingBackend) -> Result<()>,
    {
        let mut errors = Vec::new();

        for backend in self.backends_in_order() {
            match op(backend) {
                Ok(()) => {
                    if backend != self.active {
                        info!(
                            "Linux 文本输入后端从 {} 切换到 {}",
                            self.active.label(),
                            backend.label()
                        );
                        self.promote(backend);
                    }
                    return Ok(());
                }
                Err(err) => {
                    warn!("Linux 文本输入后端 {} 执行失败: {}", backend.label(), err);
                    errors.push(format!("{}: {}", backend.label(), err));
                }
            }
        }

        Err(anyhow!(
            "所有 Linux 文本输入后端均失败: {}",
            errors.join(" | ")
        ))
    }

    fn backends_in_order(&self) -> Vec<LinuxTypingBackend> {
        let mut ordered = vec![self.active];
        for backend in &self.fallbacks {
            if !ordered.contains(backend) {
                ordered.push(*backend);
            }
        }
        ordered
    }

    fn promote(&mut self, backend: LinuxTypingBackend) {
        if backend == self.active {
            return;
        }
        let previous = self.active;
        self.fallbacks.retain(|candidate| *candidate != backend);
        self.fallbacks.insert(0, previous);
        self.active = backend;
    }
}

#[cfg(target_os = "linux")]
impl LinuxTypingBackend {
    fn label(self) -> &'static str {
        match self {
            Self::Eitype => "eitype",
            Self::Xdotool => "xdotool",
            Self::Wtype => "wtype",
        }
    }

    fn command_name(self) -> &'static str {
        match self {
            Self::Eitype => "eitype",
            Self::Xdotool => "xdotool",
            Self::Wtype => "wtype",
        }
    }

    fn type_text(self, text: &str) -> Result<()> {
        let mut command = match self {
            Self::Eitype => {
                let mut command = Command::new("eitype");
                append_text_argument(&mut command, text);
                command
            }
            Self::Xdotool => {
                let mut command = Command::new("xdotool");
                command
                    .arg("type")
                    .arg("--clearmodifiers")
                    .arg("--delay")
                    .arg("1")
                    .arg("--")
                    .arg(text);
                command
            }
            Self::Wtype => {
                let mut command = Command::new("wtype");
                append_text_argument(&mut command, text);
                command
            }
        };
        run_linux_command(&mut command, self, "输入文本")
    }

    fn delete_backward(self, count: usize) -> Result<()> {
        let mut command = match self {
            Self::Eitype => {
                let mut command = Command::new("eitype");
                for _ in 0..count {
                    command.arg("-k").arg("backspace");
                }
                command
            }
            Self::Xdotool => {
                let mut command = Command::new("xdotool");
                command
                    .arg("key")
                    .arg("--repeat")
                    .arg(count.to_string())
                    .arg("BackSpace");
                command
            }
            Self::Wtype => {
                let mut command = Command::new("wtype");
                for _ in 0..count {
                    command.arg("-k").arg("BackSpace");
                }
                command
            }
        };
        run_linux_command(&mut command, self, "发送退格")
    }

    fn preferred_note(self, is_wayland: bool) -> Option<&'static str> {
        match self {
            Self::Eitype => Some("首次使用可能弹出 RemoteDesktop 授权对话框"),
            Self::Xdotool if is_wayland => {
                Some("当前只检测到 xdotool；它只对 XWayland 窗口可靠，原生 Wayland 焦点窗口通常不会收到输入")
            }
            Self::Wtype if is_wayland && wayland_desktop_prefers_eitype() => {
                Some("检测到 GNOME/KDE Wayland；wtype 常因 compositor 不支持 virtual keyboard 协议而失败，建议安装 eitype")
            }
            _ => None,
        }
    }

    fn failure_hint(self, stderr: &str, stdout: &str) -> Option<&'static str> {
        let combined = format!("{}\n{}", stderr, stdout).to_ascii_lowercase();
        match self {
            Self::Eitype => Some(
                "eitype 依赖 XDG RemoteDesktop portal / libei；首次运行可能需要授权，且需要桌面环境提供对应能力",
            ),
            Self::Xdotool if is_wayland_session() => {
                Some("xdotool 只对 X11 / XWayland 窗口可靠，原生 Wayland 窗口通常不会响应")
            }
            Self::Wtype if combined.contains("virtual keyboard protocol") => Some(
                "当前 compositor 不支持 wtype 依赖的 virtual keyboard 协议；GNOME/KDE Wayland 通常应改用 eitype",
            ),
            _ => None,
        }
    }
}

#[cfg(target_os = "linux")]
fn append_text_argument(command: &mut Command, text: &str) {
    if text.starts_with('-') {
        command.arg("--");
    }
    command.arg(text);
}

#[cfg(target_os = "linux")]
fn run_linux_command(
    command: &mut Command,
    backend: LinuxTypingBackend,
    action: &str,
) -> Result<()> {
    let output = command
        .output()
        .with_context(|| format!("执行 {} {}失败", backend.label(), action))?;
    if output.status.success() {
        return Ok(());
    }

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let mut details = Vec::new();
    if !stderr.is_empty() {
        details.push(format!("stderr: {}", stderr));
    }
    if !stdout.is_empty() {
        details.push(format!("stdout: {}", stdout));
    }
    if let Some(hint) = backend.failure_hint(&stderr, &stdout) {
        details.push(hint.to_string());
    }

    let detail_suffix = if details.is_empty() {
        String::new()
    } else {
        format!(" ({})", details.join("; "))
    };

    Err(anyhow!(
        "{} {}返回非零状态 {}{}",
        backend.label(),
        action,
        output.status,
        detail_suffix
    ))
}

pub struct Keyboard {
    backend: KeyboardBackend,
}

impl Keyboard {
    /// 创建新的键盘实例（Linux Wayland 优先命令输入，其他环境优先 enigo）
    pub fn new() -> Result<Self> {
        #[cfg(target_os = "linux")]
        if is_wayland_session() {
            if let Some(backend) = detect_linux_command_backend() {
                info!(
                    "检测到 Linux Wayland，会优先使用命令输入后端: {}",
                    backend.label()
                );
                return Ok(Self {
                    backend: KeyboardBackend::LinuxCommand(backend),
                });
            }
            warn!(
                "检测到 Linux Wayland，但未找到 eitype/wtype/xdotool；将退回 enigo，文本输入可能失败"
            );
        }

        let max_retries = 3;
        let mut last_error = None;

        for attempt in 1..=max_retries {
            match Enigo::new(&enigo::Settings::default()) {
                Ok(enigo) => {
                    info!("键盘输入初始化成功 (尝试 {}/{})", attempt, max_retries);
                    return Ok(Self {
                        backend: KeyboardBackend::Enigo(enigo),
                    });
                }
                Err(e) => {
                    last_error = Some(e);
                    warn!(
                        "键盘输入初始化失败 (尝试 {}/{}): {}",
                        attempt, max_retries, e
                    );
                    if attempt < max_retries {
                        std::thread::sleep(std::time::Duration::from_millis(500));
                    }
                }
            }
        }

        #[cfg(target_os = "linux")]
        if let Some(backend) = detect_linux_command_backend() {
            warn!(
                "enigo 初始化失败，回退到 Linux 命令输入后端: {}",
                backend.label()
            );
            return Ok(Self {
                backend: KeyboardBackend::LinuxCommand(backend),
            });
        }

        let reason = format!("键盘初始化失败: {:?}", last_error);
        error!("键盘输入初始化最终失败: {:?}", last_error);
        warn!("键盘输入将以禁用模式继续启动");
        Ok(Self {
            backend: KeyboardBackend::Unavailable(reason),
        })
    }

    pub fn backend_name(&self) -> &str {
        match &self.backend {
            KeyboardBackend::Enigo(_) => "enigo",
            #[cfg(target_os = "linux")]
            KeyboardBackend::LinuxCommand(backend) => backend.label(),
            KeyboardBackend::Unavailable(_) => "unavailable",
        }
    }

    /// 输入文本
    pub fn type_text(&mut self, text: &str) -> Result<()> {
        match &mut self.backend {
            KeyboardBackend::Enigo(enigo) => {
                use enigo::Keyboard as _;
                enigo.text(text)?;
                Ok(())
            }
            #[cfg(target_os = "linux")]
            KeyboardBackend::LinuxCommand(backend) => backend.type_text(text),
            KeyboardBackend::Unavailable(reason) => Err(anyhow!("键盘输入不可用: {}", reason)),
        }
    }

    /// 向前删除指定数量的字符（发送退格键）
    pub fn delete_backward(&mut self, count: usize) -> Result<()> {
        if count == 0 {
            return Ok(());
        }
        match &mut self.backend {
            KeyboardBackend::Enigo(enigo) => {
                use enigo::Keyboard as _;
                for _ in 0..count {
                    enigo.key(enigo::Key::Backspace, enigo::Direction::Click)?;
                }
                Ok(())
            }
            #[cfg(target_os = "linux")]
            KeyboardBackend::LinuxCommand(backend) => backend.delete_backward(count),
            KeyboardBackend::Unavailable(reason) => Err(anyhow!("键盘输入不可用: {}", reason)),
        }
    }

    /// 选中前面 count 个字符，然后用 new_text 替换选中内容。
    ///
    /// 比逐字退格再输入快得多：先 Shift+Left 选中，再直接输入（自动替换选中文字）。
    /// 用户看到的是"文字被高亮→变成新文字"，没有逐字删除的视觉延迟。
    /// 注意：在终端中 Shift+Left 可能产生转义序列，仅适用于 GUI 文本框。
    #[allow(dead_code)]
    pub fn select_backward_and_type(&mut self, select_count: usize, new_text: &str) -> Result<()> {
        if select_count == 0 {
            return self.type_text(new_text);
        }
        match &mut self.backend {
            KeyboardBackend::Enigo(enigo) => {
                use enigo::{Direction, Key, Keyboard as _};
                enigo.key(Key::Shift, Direction::Press)?;
                for _ in 0..select_count {
                    enigo.key(Key::LeftArrow, Direction::Click)?;
                }
                enigo.key(Key::Shift, Direction::Release)?;
                if !new_text.is_empty() {
                    enigo.text(new_text)?;
                } else {
                    enigo.key(Key::Backspace, Direction::Click)?;
                }
                Ok(())
            }
            #[cfg(target_os = "linux")]
            KeyboardBackend::LinuxCommand(backend) => {
                backend.delete_backward(select_count)?;
                if !new_text.is_empty() {
                    backend.type_text(new_text)?;
                }
                Ok(())
            }
            KeyboardBackend::Unavailable(reason) => Err(anyhow!("键盘输入不可用: {}", reason)),
        }
    }
}

impl Default for Keyboard {
    fn default() -> Self {
        Self::new().unwrap()
    }
}

#[cfg(target_os = "linux")]
fn detect_linux_command_backend() -> Option<LinuxCommandBackend> {
    LinuxCommandBackend::from_available_backends(detect_linux_command_backends())
}

#[cfg(target_os = "linux")]
fn detect_linux_command_backends() -> Vec<LinuxTypingBackend> {
    preferred_linux_command_backends(is_wayland_session(), wayland_desktop_prefers_eitype())
        .into_iter()
        .filter(|backend| command_exists(backend.command_name()))
        .collect()
}

#[cfg(target_os = "linux")]
pub fn preferred_linux_command_backend_label() -> Option<&'static str> {
    detect_linux_command_backend().map(|backend| backend.label())
}

#[cfg(target_os = "linux")]
pub fn preferred_linux_command_backend_note() -> Option<&'static str> {
    detect_linux_command_backend()
        .and_then(|backend| backend.active.preferred_note(is_wayland_session()))
}

#[cfg(target_os = "linux")]
fn preferred_linux_command_backends(
    is_wayland: bool,
    prefer_eitype_on_wayland: bool,
) -> Vec<LinuxTypingBackend> {
    if is_wayland {
        if prefer_eitype_on_wayland {
            vec![
                LinuxTypingBackend::Eitype,
                LinuxTypingBackend::Wtype,
                LinuxTypingBackend::Xdotool,
            ]
        } else {
            vec![
                LinuxTypingBackend::Wtype,
                LinuxTypingBackend::Eitype,
                LinuxTypingBackend::Xdotool,
            ]
        }
    } else {
        vec![LinuxTypingBackend::Xdotool]
    }
}

#[cfg(target_os = "linux")]
fn is_wayland_session() -> bool {
    std::env::var("XDG_SESSION_TYPE")
        .map(|value| value.eq_ignore_ascii_case("wayland"))
        .unwrap_or(false)
}

#[cfg(target_os = "linux")]
fn wayland_desktop_prefers_eitype() -> bool {
    is_wayland_session()
        && desktop_name_prefers_eitype(std::env::var("XDG_CURRENT_DESKTOP").ok().as_deref())
}

#[cfg(target_os = "linux")]
fn desktop_name_prefers_eitype(name: Option<&str>) -> bool {
    let Some(name) = name else {
        return false;
    };
    name.split(':').any(|entry| {
        matches!(
            entry.trim().to_ascii_lowercase().as_str(),
            "gnome" | "kde" | "plasma"
        )
    })
}

#[cfg(target_os = "linux")]
fn command_exists(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    if name.contains('/') {
        return std::path::Path::new(name).is_file();
    }
    let Some(path_var) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path_var).any(|dir| dir.join(name).is_file())
}

#[cfg(test)]
mod tests {
    #[cfg(target_os = "linux")]
    use super::{
        desktop_name_prefers_eitype, preferred_linux_command_backends, LinuxTypingBackend,
    };

    #[cfg(target_os = "linux")]
    #[test]
    fn test_preferred_linux_command_backends_for_wayland_wlroots() {
        assert_eq!(
            preferred_linux_command_backends(true, false),
            vec![
                LinuxTypingBackend::Wtype,
                LinuxTypingBackend::Eitype,
                LinuxTypingBackend::Xdotool
            ]
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_preferred_linux_command_backends_for_wayland_gnome_prefers_eitype() {
        assert_eq!(
            preferred_linux_command_backends(true, true),
            vec![
                LinuxTypingBackend::Eitype,
                LinuxTypingBackend::Wtype,
                LinuxTypingBackend::Xdotool
            ]
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_preferred_linux_command_backends_for_x11() {
        assert_eq!(
            preferred_linux_command_backends(false, false),
            vec![LinuxTypingBackend::Xdotool]
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_desktop_name_prefers_eitype_for_gnome_and_kde() {
        assert!(desktop_name_prefers_eitype(Some("ubuntu:GNOME")));
        assert!(desktop_name_prefers_eitype(Some("KDE")));
        assert!(desktop_name_prefers_eitype(Some("plasma")));
        assert!(!desktop_name_prefers_eitype(Some("sway")));
        assert!(!desktop_name_prefers_eitype(None));
    }
}
