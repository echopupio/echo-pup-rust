use anyhow::{anyhow, Context, Result};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExternalTriggerAction {
    Press,
    Release,
    Toggle,
}

impl ExternalTriggerAction {
    pub fn as_wire(self) -> &'static str {
        match self {
            Self::Press => "press",
            Self::Release => "release",
            Self::Toggle => "toggle",
        }
    }

    pub fn parse_wire(input: &str) -> Option<Self> {
        match input.trim() {
            "press" => Some(Self::Press),
            "release" => Some(Self::Release),
            "toggle" => Some(Self::Toggle),
            _ => None,
        }
    }
}

#[cfg(target_os = "linux")]
pub struct ExternalTriggerServer {
    socket_path: std::path::PathBuf,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    join_handle: Option<std::thread::JoinHandle<()>>,
}

#[cfg(target_os = "linux")]
impl ExternalTriggerServer {
    pub fn start<F>(socket_path: std::path::PathBuf, handler: F) -> Result<Self>
    where
        F: Fn(ExternalTriggerAction) + Send + Sync + 'static,
    {
        use std::os::unix::net::UnixListener;

        if socket_path.exists() {
            std::fs::remove_file(&socket_path)
                .with_context(|| format!("移除旧的触发 socket 失败: {}", socket_path.display()))?;
        }

        let listener = UnixListener::bind(&socket_path)
            .with_context(|| format!("创建触发 socket 失败: {}", socket_path.display()))?;
        listener
            .set_nonblocking(true)
            .context("设置触发 socket 为非阻塞失败")?;

        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stop_in_thread = stop.clone();
        let socket_path_in_thread = socket_path.clone();
        let handler = std::sync::Arc::new(handler);

        let join_handle = std::thread::spawn(move || {
            use std::io::Read;
            use std::sync::atomic::Ordering;
            use std::time::Duration;
            use tracing::warn;

            while !stop_in_thread.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let mut payload = String::new();
                        if let Err(err) = stream.read_to_string(&mut payload) {
                            warn!("读取触发 socket 消息失败: {}", err);
                            continue;
                        }

                        let Some(action) = ExternalTriggerAction::parse_wire(&payload) else {
                            warn!("收到未知触发动作: {:?}", payload.trim());
                            continue;
                        };
                        handler(action);
                    }
                    Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(50));
                    }
                    Err(err) => {
                        if stop_in_thread.load(Ordering::SeqCst) {
                            break;
                        }
                        warn!("接受触发 socket 连接失败: {}", err);
                        std::thread::sleep(Duration::from_millis(100));
                    }
                }
            }

            let _ = std::fs::remove_file(&socket_path_in_thread);
        });

        Ok(Self {
            socket_path,
            stop,
            join_handle: Some(join_handle),
        })
    }
}

#[cfg(target_os = "linux")]
impl Drop for ExternalTriggerServer {
    fn drop(&mut self) {
        use std::os::unix::net::UnixStream;
        use std::sync::atomic::Ordering;

        self.stop.store(true, Ordering::SeqCst);
        let _ = UnixStream::connect(&self.socket_path);
        if let Some(join_handle) = self.join_handle.take() {
            let _ = join_handle.join();
        }
        let _ = std::fs::remove_file(&self.socket_path);
    }
}

#[cfg(not(target_os = "linux"))]
pub struct ExternalTriggerServer;

#[cfg(target_os = "linux")]
pub fn send_action(socket_path: &std::path::Path, action: ExternalTriggerAction) -> Result<()> {
    use std::io::Write;
    use std::os::unix::net::UnixStream;
    use std::time::Duration;

    let mut last_err = None;
    for _ in 0..20 {
        match UnixStream::connect(socket_path) {
            Ok(mut stream) => {
                stream
                    .write_all(action.as_wire().as_bytes())
                    .and_then(|_| stream.write_all(b"\n"))
                    .with_context(|| format!("写入触发 socket 失败: {}", socket_path.display()))?;
                return Ok(());
            }
            Err(err)
                if matches!(
                    err.kind(),
                    std::io::ErrorKind::NotFound
                        | std::io::ErrorKind::ConnectionRefused
                        | std::io::ErrorKind::AddrNotAvailable
                ) =>
            {
                last_err = Some(err);
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(err) => {
                return Err(err)
                    .with_context(|| format!("连接触发 socket 失败: {}", socket_path.display()));
            }
        }
    }

    Err(anyhow!(
        "连接触发 socket 超时: {} ({})",
        socket_path.display(),
        last_err
            .map(|err| err.to_string())
            .unwrap_or_else(|| "未知错误".to_string())
    ))
}

#[cfg(not(target_os = "linux"))]
pub fn send_action(_socket_path: &std::path::Path, _action: ExternalTriggerAction) -> Result<()> {
    anyhow::bail!("外部触发仅在 Linux 可用");
}

#[cfg(test)]
mod tests {
    use super::ExternalTriggerAction;

    #[test]
    fn test_trigger_action_roundtrip() {
        for action in [
            ExternalTriggerAction::Press,
            ExternalTriggerAction::Release,
            ExternalTriggerAction::Toggle,
        ] {
            assert_eq!(
                ExternalTriggerAction::parse_wire(action.as_wire()),
                Some(action)
            );
        }
        assert_eq!(ExternalTriggerAction::parse_wire("unknown"), None);
    }
}
