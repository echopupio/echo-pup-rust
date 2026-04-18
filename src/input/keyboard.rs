//! 键盘输入模拟

use anyhow::{anyhow, Result};
#[cfg(target_os = "linux")]
use anyhow::Context;
use enigo::Enigo;
use tracing::{error, info, warn};

enum KeyboardBackend {
    Enigo(Enigo),
    #[cfg(target_os = "linux")]
    LinuxCommand(LinuxTypingBackend),
    Unavailable(String),
}

#[cfg(target_os = "linux")]
#[derive(Debug, Clone, Copy)]
enum LinuxTypingBackend {
    Xdotool,
    Wtype,
}

#[cfg(target_os = "linux")]
impl LinuxTypingBackend {
    fn label(self) -> &'static str {
        match self {
            Self::Xdotool => "xdotool",
            Self::Wtype => "wtype",
        }
    }

    fn type_text(self, text: &str) -> Result<()> {
        let mut command = match self {
            Self::Xdotool => {
                let mut command = std::process::Command::new("xdotool");
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
                let mut command = std::process::Command::new("wtype");
                command.arg(text);
                command
            }
        };
        let status = command
            .status()
            .with_context(|| format!("执行 {} 失败", self.label()))?;
        if !status.success() {
            return Err(anyhow!("{} 返回非零状态: {}", self.label(), status));
        }
        Ok(())
    }

    fn delete_backward(self, count: usize) -> Result<()> {
        if count == 0 {
            return Ok(());
        }
        let mut command = match self {
            Self::Xdotool => {
                let mut command = std::process::Command::new("xdotool");
                command
                    .arg("key")
                    .arg("--repeat")
                    .arg(count.to_string())
                    .arg("BackSpace");
                command
            }
            Self::Wtype => {
                let mut command = std::process::Command::new("wtype");
                for _ in 0..count {
                    command.arg("-k").arg("BackSpace");
                }
                command
            }
        };
        let status = command
            .status()
            .with_context(|| format!("执行 {} 退格失败", self.label()))?;
        if !status.success() {
            return Err(anyhow!("{} 退格返回非零状态: {}", self.label(), status));
        }
        Ok(())
    }
}

pub struct Keyboard {
    backend: KeyboardBackend,
}

impl Keyboard {
    /// 创建新的键盘实例（优先 enigo，Linux 下自动回退 xdotool / wtype）
    pub fn new() -> Result<Self> {
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
fn detect_linux_command_backend() -> Option<LinuxTypingBackend> {
    if command_exists("xdotool") {
        return Some(LinuxTypingBackend::Xdotool);
    }
    if command_exists("wtype") {
        return Some(LinuxTypingBackend::Wtype);
    }
    None
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
