//! EchoPup - AI Voice Dictation Tool

mod asr;
mod audio;
mod commit;
mod config;
mod hotkey;
mod input;
mod linux_desktop;
mod llm;
mod menu_core;
mod model_download;
mod punctuation;
mod runtime;
mod session;
mod status_indicator;
mod text_processor;
mod trigger;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use parking_lot::Mutex;
use std::io::{IsTerminal, Write};
#[cfg(all(unix, not(target_os = "macos")))]
use std::os::fd::{AsRawFd, RawFd};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{debug, error, info, warn};

#[cfg(target_os = "macos")]
const MAC_OSASCRIPT_PATH: &str = "/usr/bin/osascript";
#[cfg(target_os = "macos")]
const MAC_AFPLAY_PATH: &str = "/usr/bin/afplay";
#[cfg(target_os = "macos")]
const MAC_SOUND_RECORDING_START: &str = "/System/Library/Sounds/Tink.aiff";
#[cfg(target_os = "macos")]
const MAC_SOUND_RECORDING_END: &str = "/System/Library/Sounds/Pop.aiff";

#[derive(Parser)]
#[command(name = "echopup")]
#[command(about = "AI Voice Dictation Tool", long_about = None)]
struct Cli {
    #[arg(short, long, default_value = "~/.echopup/config.toml")]
    config: String,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    #[command(hide = true)]
    Run,
    Start,
    Stop,
    Status,
    Restart,
    Config {
        #[command(subcommand)]
        command: Option<ConfigCommands>,
    },
    /// 列出可用的音频输入设备
    Devices,
    /// 查看后台运行日志
    Log {
        /// 显示最后 N 行（默认 50）
        #[arg(short = 'n', long, default_value = "50")]
        lines: usize,
        /// 持续跟踪日志（类似 tail -f）
        #[arg(short, long)]
        follow: bool,
    },
    /// 系统环境诊断
    Doctor,
    /// 显示版本信息
    Version,
    DownloadModel,
    /// 发送外部触发动作（主要用于 Linux/Wayland 桌面快捷键绑定）
    Trigger {
        #[command(subcommand)]
        action: TriggerCommands,
    },
    #[command(hide = true)]
    StatusIndicator,
}

#[derive(Subcommand, Clone, Copy, Debug)]
enum TriggerCommands {
    Press,
    Release,
    Toggle,
}

#[derive(Subcommand)]
enum ConfigCommands {
    /// 显示当前配置
    Show,
    /// 初始化默认配置文件
    Init,
    /// 显示配置文件路径
    Path,
    /// 用编辑器打开配置文件
    Edit,
}

#[cfg(all(unix, not(target_os = "macos")))]
struct TerminalEchoGuard {
    fd: RawFd,
    original: Option<libc::termios>,
}

#[cfg(all(unix, not(target_os = "macos")))]
impl TerminalEchoGuard {
    fn try_disable_stdin_echo() -> Result<Option<Self>> {
        let fd = std::io::stdin().as_raw_fd();
        let is_tty = unsafe { libc::isatty(fd) };
        if is_tty != 1 {
            return Ok(None);
        }

        let mut current = std::mem::MaybeUninit::<libc::termios>::uninit();
        let read_res = unsafe { libc::tcgetattr(fd, current.as_mut_ptr()) };
        if read_res != 0 {
            return Err(anyhow::anyhow!(
                "读取终端属性失败: {}",
                std::io::Error::last_os_error()
            ));
        }
        let current = unsafe { current.assume_init() };

        let mut no_echo = current;
        no_echo.c_lflag &= !libc::ECHO;
        let set_res = unsafe { libc::tcsetattr(fd, libc::TCSANOW, &no_echo) };
        if set_res != 0 {
            return Err(anyhow::anyhow!(
                "关闭终端输入回显失败: {}",
                std::io::Error::last_os_error()
            ));
        }

        info!("终端输入回显已关闭（TTY）");
        Ok(Some(Self {
            fd,
            original: Some(current),
        }))
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
impl Drop for TerminalEchoGuard {
    fn drop(&mut self) {
        if let Some(original) = self.original.as_ref() {
            let restore_res = unsafe { libc::tcsetattr(self.fd, libc::TCSANOW, original) };
            if restore_res != 0 {
                warn!("恢复终端输入回显失败: {}", std::io::Error::last_os_error());
            } else {
                info!("终端输入回显已恢复");
            }
        }
    }
}

fn clear_terminal_artifacts_if_tty() {
    if !std::io::stdout().is_terminal() {
        return;
    }
    // 尽量清掉功能键在前台终端里留下的转义序列/乱码
    print!("\r\x1b[2K");
    let _ = std::io::Write::flush(&mut std::io::stdout());
}

fn set_status_indicator_state(
    indicator: &Arc<Mutex<status_indicator::StatusIndicatorClient>>,
    state: status_indicator::IndicatorState,
) {
    let mut guard = indicator.lock();
    guard.send(state);
}

fn send_status_snapshot(
    indicator: &Arc<Mutex<status_indicator::StatusIndicatorClient>>,
    snapshot_state: &Arc<Mutex<menu_core::MenuSnapshot>>,
    status: impl Into<String>,
) {
    let snapshot = {
        let mut snapshot = snapshot_state.lock();
        snapshot.status = status.into();
        snapshot.clone()
    };
    let mut guard = indicator.lock();
    guard.send_snapshot(&snapshot);
}

fn log_asr_runtime(label: &str, engine: &dyn asr::AsrEngine) {
    let runtime = engine.runtime_info();
    info!(
        "ASR {}: backend={}, model={}, detail={}, threads={}",
        label,
        runtime.backend.label(),
        runtime.model,
        runtime.detail.unwrap_or_else(|| "n/a".to_string()),
        runtime.threads.unwrap_or_default()
    );
}

fn transcribe_final_with_session(
    asr_engine: &mut dyn asr::AsrEngine,
    audio: &[f32],
) -> Result<String> {
    let session_config = asr::AsrSessionConfig {
        min_partial_samples: audio.len().max(1),
        max_partial_window_samples: audio.len().max(1),
    };

    match asr_engine.start_session(session_config) {
        Ok(mut session) => {
            session.accept_audio(audio)?;
            session.finalize(Arc::new(AtomicBool::new(false)))
        }
        Err(err) => {
            warn!(
                "ASR backend {} 不支持 final session，回退到整段转写: {}",
                asr_engine.backend_kind().label(),
                err
            );
            asr_engine.transcribe(audio)
        }
    }
}

fn detach_thread_handle(handle: &Arc<Mutex<Option<std::thread::JoinHandle<()>>>>) {
    if let Some(join_handle) = handle.lock().take() {
        std::thread::spawn(move || {
            let _ = join_handle.join();
        });
    }
}

fn join_thread_handle(handle: &Arc<Mutex<Option<std::thread::JoinHandle<()>>>>) {
    if let Some(join_handle) = handle.lock().take() {
        let _ = join_handle.join();
    }
}

/// 失败路径清理：清除屏幕上残留的草稿文字和 partial 状态
fn cleanup_draft_on_failure(
    recognition_session: &Arc<Mutex<session::RecognitionSession>>,
    text_commit: &Arc<Mutex<Box<dyn commit::TextCommitBackend>>>,
) {
    let draft_clear = recognition_session.lock().prepare_draft_clear();
    if let Some(action) = draft_clear {
        if let Err(err) = text_commit.lock().apply(action) {
            warn!("失败路径清除草稿失败: {}", err);
        }
    }
    recognition_session.lock().clear_partials();
}

/// 处理音频数据：转写 -> LLM 整理 -> 谐音纠错 -> 键盘输入
fn process_audio(
    audio_data: &[f32],
    config_path: &str,
    asr_runtime: &Arc<Mutex<Option<Box<dyn asr::AsrEngine>>>>,
    llm: &Arc<Mutex<Option<llm::LLMRewrite>>>,
    post_processor: &Arc<text_processor::TextPostProcessor>,
    punct_restorer: &Arc<Mutex<Option<punctuation::PunctuationRestorer>>>,
    text_commit: &Arc<Mutex<Box<dyn commit::TextCommitBackend>>>,
    recognition_session: &Arc<Mutex<session::RecognitionSession>>,
    e2e_start: Instant,
    desktop_notify_enabled: bool,
) -> bool {
    if audio_data.is_empty() {
        info!("录音数据为空");
        cleanup_draft_on_failure(recognition_session, text_commit);
        desktop_notify(desktop_notify_enabled, "EchoPup", "未检测到语音输入");
        return false;
    }
    info!("录音完成，采样点: {}", audio_data.len());

    let mut llm_ms = 0u128;
    let mut postprocess_ms = 0u128;
    let mut type_ms = 0u128;
    let mut stt_backend = "unknown".to_string();

    // 1. 音频转写
    let processed_audio = audio_data;
    let mut final_text = String::new();
    let mut transcribe_success = false;
    let stt_start = Instant::now();
    let mut stt_model = "uninitialized".to_string();
    let mut stt_detail = "unknown".to_string();
    let mut stt_threads = 0;

    if !ensure_main_asr_runtime_ready(config_path, asr_runtime) {
        error!("ASR 运行时未就绪且按需初始化失败");
        desktop_notify(
            desktop_notify_enabled,
            "EchoPup",
            "语音识别引擎初始化失败，请查看日志",
        );
    }

    {
        let mut asr_guard = asr_runtime.lock();
        if let Some(ref mut asr_engine) = *asr_guard {
            let runtime = asr_engine.runtime_info();
            stt_backend = runtime.backend.label().to_string();
            stt_model = runtime.model;
            stt_detail = runtime.detail.unwrap_or_else(|| "n/a".to_string());
            stt_threads = runtime.threads.unwrap_or_default();
            match transcribe_final_with_session(asr_engine.as_mut(), processed_audio) {
                Ok(text) => {
                    // 过滤无效结果
                    let trimmed = text.trim();
                    if trimmed.is_empty() || trimmed == "[BLANK_AUDIO]" {
                        info!("转写结果为空或无效（可能没有说话或音量太小）");
                        cleanup_draft_on_failure(recognition_session, text_commit);
                        desktop_notify(desktop_notify_enabled, "EchoPup", "未识别到有效语音");
                        return false;
                    }
                    info!("转写完成: {}", text);
                    final_text = text;
                    transcribe_success = true;
                }
                Err(e) => {
                    error!("转写失败: {}", e);
                }
            }
        } else {
            error!("ASR 运行时未就绪");
        }
    }
    let stt_ms = stt_start.elapsed().as_millis();

    if !transcribe_success {
        cleanup_draft_on_failure(recognition_session, text_commit);
        desktop_notify(
            desktop_notify_enabled,
            "EchoPup",
            "语音识别失败，请查看日志",
        );
        info!(
            "[热键] 性能埋点: backend={} model={} detail={} threads={} stt_ms={} llm_ms={} postprocess_ms={} type_ms={} e2e_ms={}",
            stt_backend,
            stt_model,
            stt_detail,
            stt_threads,
            stt_ms,
            llm_ms,
            postprocess_ms,
            type_ms,
            e2e_start.elapsed().as_millis()
        );
        return false;
    }

    // 2. 离线标点恢复
    {
        let punct_guard = punct_restorer.lock();
        if let Some(ref restorer) = *punct_guard {
            let punct_start = Instant::now();
            final_text = restorer.add_punctuation(&final_text);
            info!("标点恢复耗时: {}ms", punct_start.elapsed().as_millis());
        }
    }

    // 3. LLM 整理（如果启用）
    let llm_enabled = {
        let llm_guard = llm.lock();
        llm_guard.as_ref().map(|l| l.is_enabled()).unwrap_or(false)
    };

    if llm_enabled {
        let llm_start = Instant::now();
        let llm_guard = llm.lock();
        if let Some(ref llm) = *llm_guard {
            match llm.rewrite(&final_text) {
                Ok(rewritten) => {
                    final_text = rewritten;
                }
                Err(e) => {
                    error!("LLM 整理异常: {}", e);
                }
            }
        }
        llm_ms = llm_start.elapsed().as_millis();
    }

    // 4. 谐音纠错（规则映射）
    let postprocess_start = Instant::now();
    let corrected = post_processor.process(&final_text);
    if corrected != final_text {
        info!("谐音纠错已应用");
        final_text = corrected;
    }
    postprocess_ms = postprocess_start.elapsed().as_millis();

    // 5. 文本提交
    let commit_action = {
        let mut session_guard = recognition_session.lock();
        session_guard.prepare_final_commit(&final_text)
    };
    let Some(commit_action) = commit_action else {
        // 即使跳过提交，也需清理 partial 状态
        recognition_session.lock().clear_partials();
        info!("最终结果为空或与本次会话已提交内容重复，跳过文本提交");
        return false;
    };

    let type_start = Instant::now();
    let mut type_success = false;
    {
        let mut commit_guard = text_commit.lock();
        match commit_guard.apply(commit_action) {
            Ok(_) => {
                info!("文本已提交");
                type_success = true;
                desktop_notify(
                    desktop_notify_enabled,
                    "EchoPup",
                    &format!("识别完成，已输入 {} 字", final_text.chars().count()),
                );
            }
            Err(e) => {
                error!("文本提交失败: {}", e);
                desktop_notify(desktop_notify_enabled, "EchoPup", "识别完成，但输入失败");
            }
        }
    }
    // final commit 完成后清理 partial 状态
    recognition_session.lock().clear_partials();
    type_ms = type_start.elapsed().as_millis();

    info!(
        "[热键] 性能埋点: backend={} model={} detail={} threads={} stt_ms={} llm_ms={} postprocess_ms={} type_ms={} e2e_ms={}",
        stt_backend,
        stt_model,
        stt_detail,
        stt_threads,
        stt_ms,
        llm_ms,
        postprocess_ms,
        type_ms,
        e2e_start.elapsed().as_millis()
    );

    type_success
}

fn desktop_notify(enabled: bool, title: &str, body: &str) {
    if !enabled {
        return;
    }

    if let Err(err) = send_desktop_notify(title, body) {
        warn!("桌面通知发送失败: {}", err);
    }
}

fn detect_desktop_notify_capability() -> (bool, String) {
    #[cfg(target_os = "linux")]
    {
        if !command_exists("notify-send") {
            return (
                false,
                "未找到 notify-send（请安装 libnotify-bin）".to_string(),
            );
        }
        let has_display =
            std::env::var_os("DISPLAY").is_some() || std::env::var_os("WAYLAND_DISPLAY").is_some();
        if !has_display {
            return (
                false,
                "未检测到 DISPLAY/WAYLAND_DISPLAY（需要图形会话）".to_string(),
            );
        }
        return (true, "linux notify-send".to_string());
    }

    #[cfg(target_os = "macos")]
    {
        if !std::path::Path::new(MAC_OSASCRIPT_PATH).is_file() {
            return (false, format!("未找到 osascript: {}", MAC_OSASCRIPT_PATH));
        }
        return (true, format!("macOS osascript ({})", MAC_OSASCRIPT_PATH));
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        (false, "当前系统未实现桌面通知后端".to_string())
    }
}

#[derive(Clone, Copy)]
enum FeedbackSoundEvent {
    RecordingStart,
    RecordingEnd,
}

fn detect_sound_feedback_capability(user_enabled: bool) -> (bool, String) {
    if !user_enabled {
        return (false, "已在配置中关闭 sound_enabled".to_string());
    }

    #[cfg(target_os = "macos")]
    {
        if !std::path::Path::new(MAC_AFPLAY_PATH).is_file() {
            return (
                false,
                format!("未找到 afplay: {}（无法播放提示音）", MAC_AFPLAY_PATH),
            );
        }
        return (true, format!("macOS afplay ({})", MAC_AFPLAY_PATH));
    }

    #[cfg(target_os = "linux")]
    {
        if command_exists("paplay") {
            return (true, "linux paplay".to_string());
        }
        if command_exists("aplay") {
            return (true, "linux aplay".to_string());
        }
        return (
            false,
            "未找到 paplay/aplay（请安装 pulseaudio-utils 或 alsa-utils）".to_string(),
        );
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        (false, "当前系统未实现提示音后端".to_string())
    }
}

fn play_feedback_sound(enabled: bool, event: FeedbackSoundEvent) {
    if !enabled {
        return;
    }

    if let Err(err) = send_feedback_sound(event) {
        warn!("提示音播放失败: {}", err);
    }
}

fn send_feedback_sound(event: FeedbackSoundEvent) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        let sound_path = match event {
            FeedbackSoundEvent::RecordingStart => MAC_SOUND_RECORDING_START,
            FeedbackSoundEvent::RecordingEnd => MAC_SOUND_RECORDING_END,
        };
        if !std::path::Path::new(sound_path).is_file() {
            anyhow::bail!("找不到系统提示音文件: {}", sound_path);
        }
        let _child = std::process::Command::new(MAC_AFPLAY_PATH)
            .arg(sound_path)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .context("启动 afplay 失败")?;
        return Ok(());
    }

    #[cfg(target_os = "linux")]
    {
        let _ = event;
        let mut child = if command_exists("paplay") {
            std::process::Command::new("paplay")
                .arg("/usr/share/sounds/freedesktop/stereo/message.oga")
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .context("启动 paplay 失败")?
        } else if command_exists("aplay") {
            std::process::Command::new("aplay")
                .arg("/usr/share/sounds/alsa/Front_Center.wav")
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .context("启动 aplay 失败")?
        } else {
            anyhow::bail!("未找到 paplay/aplay");
        };
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        return Ok(());
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = event;
        Ok(())
    }
}

fn print_macos_notification_setup_tip(show_tip: bool) {
    if !show_tip {
        return;
    }

    #[cfg(target_os = "macos")]
    {
        println!("提示: macOS 通知由 osascript 发送，通知来源通常显示为“脚本编辑器”。");
        println!(
            "若全屏时看不到横幅，请到“系统设置 -> 通知 -> 脚本编辑器”开启通知，并选择“横幅”或“提醒”。"
        );
        println!("若仍不弹出，请检查“专注模式”和“通知摘要”设置。");
    }
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

fn send_desktop_notify(title: &str, body: &str) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        let status = std::process::Command::new("notify-send")
            .arg("-a")
            .arg("EchoPup")
            .arg("-u")
            .arg("low")
            .arg(title)
            .arg(body)
            .status()
            .context("执行 notify-send 失败")?;
        if !status.success() {
            anyhow::bail!("notify-send 返回非零状态: {}", status);
        }
        return Ok(());
    }

    #[cfg(target_os = "macos")]
    {
        let escaped_title = title.replace('"', "\\\"");
        let escaped_body = body.replace('"', "\\\"");
        let status = std::process::Command::new(MAC_OSASCRIPT_PATH)
            .arg("-e")
            .arg(format!(
                "display notification \"{}\" with title \"{}\"",
                escaped_body, escaped_title
            ))
            .status()
            .context("执行 osascript 失败")?;
        if !status.success() {
            anyhow::bail!("osascript 返回非零状态: {}", status);
        }
        return Ok(());
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (title, body);
        Ok(())
    }
}

fn download_paraformer_model_cli() -> Result<()> {
    info!("开始下载所有必需模型...");
    ensure_all_models_downloaded()
}

fn ensure_all_models_downloaded() -> Result<()> {
    use model_download::{
        check_missing_models, start_paraformer_model_download, start_punctuation_model_download,
    };

    let missing = check_missing_models();
    if missing.is_empty() {
        return Ok(());
    }

    println!("检测到以下模型缺失，开始自动下载：");
    for m in &missing {
        println!("  ⚠ {}", m);
    }
    println!();

    // Download ASR if missing
    let asr_dir = model_download::paraformer_model_dir();
    let asr_missing = model_download::paraformer_model_files()
        .iter()
        .any(|f| !asr_dir.join(f).exists());

    if asr_missing {
        println!("📦 下载 ASR 模型 (Paraformer)...");
        download_model_with_cli_progress(start_paraformer_model_download()?)?;
        println!();
    }

    // Download punctuation if missing
    let punct_path = model_download::punctuation_model_path();
    let punct_missing = !punct_path.exists()
        || std::fs::metadata(&punct_path)
            .map(|m| m.len() == 0)
            .unwrap_or(true);

    if punct_missing {
        println!("📦 下载标点恢复模型...");
        download_model_with_cli_progress(start_punctuation_model_download()?)?;
        println!();
    }

    println!("✅ 所有模型已就绪！");
    Ok(())
}

/// Helper: consume a DownloadStart and print CLI progress
fn download_model_with_cli_progress(start: model_download::DownloadStart) -> Result<()> {
    use model_download::DownloadEvent;

    let model_download::DownloadStart {
        rx, initial_logs, ..
    } = start;

    for line in initial_logs {
        println!("  {}", line);
    }

    let mut latest_total = None::<u64>;
    let mut progress_active = false;

    loop {
        match rx.recv() {
            Ok(DownloadEvent::Started {
                downloaded, total, ..
            }) => {
                latest_total = total;
                if progress_active {
                    println!();
                    progress_active = false;
                }
                println!(
                    "  [started] 已下载 {}，总大小 {}",
                    model_download::format_bytes(downloaded),
                    total
                        .map(model_download::format_bytes)
                        .unwrap_or_else(|| "未知".to_string())
                );
            }
            Ok(DownloadEvent::Progress { downloaded, total }) => {
                if total.is_some() {
                    latest_total = total;
                }
                let progress_text = match latest_total {
                    Some(total) if total > 0 => {
                        let ratio = (downloaded as f64 / total as f64).clamp(0.0, 1.0);
                        format!(
                            "{} / {} ({:.1}%)",
                            model_download::format_bytes(downloaded),
                            model_download::format_bytes(total),
                            ratio * 100.0
                        )
                    }
                    _ => format!("已下载 {}", model_download::format_bytes(downloaded)),
                };
                print!("\r  [progress] {}", progress_text);
                let _ = std::io::stdout().flush();
                progress_active = true;
            }
            Ok(DownloadEvent::Finished) => {
                if progress_active {
                    println!();
                }
                println!("  ✅ 下载完成");
                return Ok(());
            }
            Ok(DownloadEvent::Failed(err)) => {
                if progress_active {
                    println!();
                }
                return Err(anyhow::anyhow!("下载失败: {}", err));
            }
            Ok(DownloadEvent::Log(line)) => {
                if progress_active {
                    println!();
                    progress_active = false;
                }
                println!("  {}", line);
            }
            Err(_) => {
                if progress_active {
                    println!();
                }
                return Err(anyhow::anyhow!("下载线程已断开"));
            }
        }
    }
}

fn print_banner() {
    let ver = env!("CARGO_PKG_VERSION");
    eprintln!(
        r#"
  █▀▀ █▀▀ █ █ █▀█      █▀█ █ █ █▀█"#
    );
    eprintln!(
        "  █▀▀ █\x1b[38;2;255;200;0m⣤\x1b[38;2;0;200;100m⣿\x1b[0m █▀█ █ █ \x1b[38;2;255;200;0m⣀\x1b[38;2;0;200;100m⣿\x1b[38;2;160;50;200m⣤\x1b[38;2;60;120;255m⣶\x1b[0m █▀▀ █ █ █▀▀"
    );
    eprintln!("  ▀▀▀ ▀▀▀ ▀ ▀ ▀▀▀      ▀   ▀▀▀ ▀");
    eprintln!("  🎙  AI Voice Dictation  v{}\n", ver);
}

fn status_indicator_surface_name() -> &'static str {
    if cfg!(target_os = "macos") {
        "macOS 菜单栏"
    } else if cfg!(target_os = "linux") {
        "Linux 托盘"
    } else {
        "状态栏"
    }
}

fn default_trigger_key() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        "ctrl"
    }

    #[cfg(target_os = "linux")]
    {
        "f6"
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        "ctrl"
    }
}

#[cfg(target_os = "linux")]
fn is_wayland_session() -> bool {
    std::env::var("XDG_SESSION_TYPE")
        .map(|v| v.eq_ignore_ascii_case("wayland"))
        .unwrap_or(false)
}

#[cfg(not(target_os = "linux"))]
fn is_wayland_session() -> bool {
    false
}

fn uses_external_trigger_backend() -> bool {
    #[cfg(target_os = "linux")]
    {
        is_wayland_session()
    }

    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}

fn trigger_backend_description() -> String {
    if uses_external_trigger_backend() {
        format!(
            "外部触发 (CLI/IPC，建议桌面快捷键 {} -> `echopup trigger toggle`)",
            default_trigger_key().to_uppercase()
        )
    } else {
        format!("应用内热键 ({})", default_trigger_key())
    }
}

#[cfg(target_os = "linux")]
fn internal_hotkey_conflict_warning() -> Option<String> {
    if uses_external_trigger_backend() {
        return None;
    }

    let binding = default_trigger_key().to_uppercase();
    match linux_desktop::find_echopup_shortcut_conflict(&binding) {
        Ok(Some(conflict)) => Some(format!(
            "检测到 GNOME 自定义快捷键 {} -> {}，当前 X11 会话已使用应用内热键；请删除或改键该桌面快捷键以避免重复触发",
            conflict.binding,
            conflict
                .name
                .as_deref()
                .unwrap_or("echopup trigger toggle")
        )),
        Ok(None) => None,
        Err(err) => {
            debug!("检查 GNOME 自定义快捷键冲突失败: {}", err);
            None
        }
    }
}

fn shell_escape_arg(input: &str) -> String {
    if input.is_empty() {
        return "''".to_string();
    }

    if !input.contains([' ', '\t', '\n', '\'', '"', '\\']) {
        return input.to_string();
    }

    format!("'{}'", input.replace('\'', "'\"'\"'"))
}

#[cfg(target_os = "linux")]
fn linux_wayland_trigger_command(config_path: &str) -> Result<String> {
    let exe = std::env::current_exe().context("获取当前可执行文件路径失败")?;
    Ok(format!(
        "{} --config {} trigger toggle",
        shell_escape_arg(&exe.display().to_string()),
        shell_escape_arg(config_path)
    ))
}

#[cfg(not(target_os = "linux"))]
fn linux_wayland_trigger_command(_config_path: &str) -> Result<String> {
    anyhow::bail!("仅 Linux 支持 Wayland 外部触发命令");
}

fn maybe_setup_linux_wayland_shortcut(config_path: &str) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        let command_line = linux_wayland_trigger_command(config_path)?;
        match linux_desktop::maybe_install_gnome_wayland_shortcut(
            &command_line,
            &default_trigger_key().to_uppercase(),
        )? {
            linux_desktop::ShortcutInstallResult::Installed => {
                println!(
                    "检测到 GNOME Wayland，已自动创建系统快捷键: {} -> EchoPup 切换录音",
                    default_trigger_key().to_uppercase()
                );
                println!("对应命令: {}", command_line);
                println!("说明: 该快捷键通过 `echopup trigger toggle` 触发后台服务。");
            }
            linux_desktop::ShortcutInstallResult::AlreadyInstalled { .. }
            | linux_desktop::ShortcutInstallResult::UnsupportedEnvironment => {}
            linux_desktop::ShortcutInstallResult::BindingConflict {
                binding,
                shortcut_name,
            } => {
                println!(
                    "检测到 GNOME Wayland，但 {} 已被现有快捷键{}占用，未自动覆盖。",
                    binding,
                    shortcut_name
                        .as_deref()
                        .map(|name| format!(" `{}`", name))
                        .unwrap_or_default()
                );
                println!("请手动将以下命令绑定到其他按键: {}", command_line);
            }
            linux_desktop::ShortcutInstallResult::GsettingsUnavailable => {
                println!(
                    "检测到 Linux Wayland，但当前无法通过 gsettings 自动写入 GNOME 自定义快捷键。"
                );
                println!(
                    "请手动将以下命令绑定到系统快捷键 {}: {}",
                    default_trigger_key().to_uppercase(),
                    command_line
                );
            }
        }
    }

    Ok(())
}

fn ensure_background_launch_supported() -> Result<()> {
    if uses_external_trigger_backend() {
        return Ok(());
    }

    hotkey::listener::validate_hotkey_config(default_trigger_key())
        .context("当前环境无法启动 EchoPup 后台服务")?;
    Ok(())
}

fn ensure_background_started(pid: u32, log_path: &std::path::Path) -> Result<()> {
    runtime::wait_for_background_start(pid).map_err(|err| {
        let recent_log = runtime::read_recent_background_log(20).unwrap_or_default();
        if recent_log.trim().is_empty() {
            anyhow::anyhow!(
                "后台服务启动失败: {}\n日志文件: {}",
                err,
                log_path.display()
            )
        } else {
            anyhow::anyhow!(
                "后台服务启动失败: {}\n日志文件: {}\n最近日志:\n{}",
                err,
                log_path.display(),
                recent_log
            )
        }
    })
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::DEBUG.into()),
        )
        .init();

    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Run) => run_voice_input(&cli.config)?,
        Some(Commands::Start) => start_background_mode(&cli.config)?,
        Some(Commands::Stop) => stop_background_mode()?,
        Some(Commands::Status) => show_background_status()?,
        Some(Commands::Restart) => restart_background_mode(&cli.config)?,
        Some(Commands::Config { command }) => handle_config_command(&cli.config, command)?,
        Some(Commands::Devices) => list_audio_devices()?,
        Some(Commands::Log { lines, follow }) => show_log(lines, follow)?,
        Some(Commands::Doctor) => run_doctor(&cli.config)?,
        Some(Commands::Version) => {
            println!("echopup {}", env!("CARGO_PKG_VERSION"));
        }
        Some(Commands::Trigger { action }) => handle_trigger_command(&cli.config, action)?,
        Some(Commands::StatusIndicator) => status_indicator::run_status_indicator_process()?,
        Some(Commands::DownloadModel) => download_paraformer_model_cli()?,
        None => start_background_mode(&cli.config)?,
    }

    Ok(())
}

fn start_background_mode(config_path: &str) -> Result<()> {
    print_banner();
    let config = config::Config::load(config_path)?;

    if runtime::is_running()? {
        if let Some(pid) = runtime::running_instance_pid()? {
            println!("echopup 已在后台运行 (pid: {})，不会重复创建进程。", pid);
        } else {
            println!("echopup 已在后台运行，不会重复创建进程。");
        }
        println!("可使用 `echopup config` 管理配置。");
        return Ok(());
    }

    ensure_background_launch_supported()?;
    maybe_setup_linux_wayland_shortcut(config_path)?;
    let pid = runtime::spawn_background(config_path)?;
    let log_path = runtime::background_log_path()?;
    ensure_background_started(pid, &log_path)?;
    println!("echopup 已在后台启动 (pid: {})", pid);
    println!("日志文件: {}", log_path.display());
    print_macos_notification_setup_tip(config.feedback.notify_tip_on_start);

    println!("可使用 `echopup config` 管理配置。");
    Ok(())
}

fn handle_trigger_command(_config_path: &str, action: TriggerCommands) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        if !uses_external_trigger_backend() {
            return Ok(());
        }

        if runtime::running_instance_pid()?.is_none() {
            anyhow::bail!("后台服务未运行，请先执行 `echopup start`");
        }

        let trigger_action = match action {
            TriggerCommands::Press => trigger::ExternalTriggerAction::Press,
            TriggerCommands::Release => trigger::ExternalTriggerAction::Release,
            TriggerCommands::Toggle => trigger::ExternalTriggerAction::Toggle,
        };
        let socket_path = runtime::trigger_socket_path()?;
        trigger::send_action(&socket_path, trigger_action)?;
        return Ok(());
    }

    #[cfg(not(target_os = "linux"))]
    {
        let _ = action;
        anyhow::bail!("`echopup trigger` 当前仅用于 Linux 外部快捷键集成");
    }
}

fn stop_background_mode() -> Result<()> {
    match runtime::stop_running_instance()? {
        Some(pid) => println!("echopup 已停止 (pid: {})", pid),
        None => println!("echopup 未在运行。"),
    }
    Ok(())
}

fn show_background_status() -> Result<()> {
    match runtime::running_instance_pid()? {
        Some(pid) => {
            println!("echopup 正在运行 (pid: {})", pid);
            if let Ok(log_path) = runtime::background_log_path() {
                println!("日志文件: {}", log_path.display());
            }
        }
        None => println!("echopup 未运行。"),
    }
    Ok(())
}

fn restart_background_mode(config_path: &str) -> Result<()> {
    let config = config::Config::load(config_path)?;
    ensure_background_launch_supported()?;

    if let Some(pid) = runtime::stop_running_instance()? {
        println!("已停止旧实例 (pid: {})", pid);
    }

    let pid = runtime::spawn_background(config_path)?;
    let log_path = runtime::background_log_path()?;
    ensure_background_started(pid, &log_path)?;
    println!("echopup 已重启并在后台运行 (pid: {})", pid);
    println!("日志文件: {}", log_path.display());
    print_macos_notification_setup_tip(config.feedback.notify_tip_on_start);
    println!("可使用 `echopup config` 管理配置。");
    Ok(())
}

fn handle_config_command(config_path: &str, command: Option<ConfigCommands>) -> Result<()> {
    let resolved = config_path.replace(
        "~",
        &dirs::home_dir().unwrap_or_default().display().to_string(),
    );
    match command.unwrap_or(ConfigCommands::Show) {
        ConfigCommands::Show => {
            let config = config::Config::load(config_path)?;
            println!("{:#?}", config);
        }
        ConfigCommands::Init => {
            let config = config::Config::default();
            config.save(config_path)?;
            println!("默认配置已保存到: {}", resolved);
        }
        ConfigCommands::Path => {
            println!("{}", resolved);
        }
        ConfigCommands::Edit => {
            let editor = std::env::var("EDITOR").unwrap_or_else(|_| {
                if cfg!(target_os = "macos") {
                    "open -e".to_string()
                } else {
                    "vi".to_string()
                }
            });
            let path = std::path::Path::new(&resolved);
            if !path.exists() {
                let config = config::Config::default();
                config.save(config_path)?;
                println!("配置文件不存在，已创建默认配置: {}", resolved);
            }
            let parts: Vec<&str> = editor.split_whitespace().collect();
            let status = std::process::Command::new(parts[0])
                .args(&parts[1..])
                .arg(&resolved)
                .status()
                .with_context(|| format!("启动编辑器 '{}' 失败", editor))?;
            if !status.success() {
                anyhow::bail!("编辑器退出码: {}", status);
            }
        }
    }
    Ok(())
}

fn list_audio_devices() -> Result<()> {
    use cpal::traits::{DeviceTrait, HostTrait};
    let host = cpal::default_host();
    let default_device = host.default_input_device();
    let default_name = default_device
        .as_ref()
        .and_then(|d| d.name().ok())
        .unwrap_or_default();

    println!("可用音频输入设备：\n");
    match host.input_devices() {
        Ok(devices) => {
            let mut count = 0;
            for device in devices {
                let name = device.name().unwrap_or_else(|_| "(未知)".to_string());
                let is_default = name == default_name;
                let config_info = device
                    .default_input_config()
                    .map(|c| {
                        format!(
                            "{}Hz, {}ch, {:?}",
                            c.sample_rate().0,
                            c.channels(),
                            c.sample_format()
                        )
                    })
                    .unwrap_or_else(|_| "无法获取配置".to_string());
                println!(
                    "  {} {} ({})",
                    if is_default { "►" } else { " " },
                    name,
                    config_info
                );
                count += 1;
            }
            if count == 0 {
                println!("  (无可用设备)");
            }
        }
        Err(e) => println!("获取设备列表失败: {}", e),
    }
    Ok(())
}

fn show_log(lines: usize, follow: bool) -> Result<()> {
    let log_path = runtime::background_log_path()?;
    if !log_path.exists() {
        println!("日志文件不存在: {}", log_path.display());
        println!("提示: 先运行 `echopup start` 启动后台服务");
        return Ok(());
    }

    if follow {
        let status = std::process::Command::new("tail")
            .arg("-f")
            .arg("-n")
            .arg(lines.to_string())
            .arg(&log_path)
            .status()
            .context("执行 tail -f 失败")?;
        if !status.success() {
            anyhow::bail!("tail 退出码: {}", status);
        }
    } else {
        let content = std::fs::read_to_string(&log_path)
            .with_context(|| format!("读取日志失败: {}", log_path.display()))?;
        let all_lines: Vec<&str> = content.lines().collect();
        let start = if all_lines.len() > lines {
            all_lines.len() - lines
        } else {
            0
        };
        for line in &all_lines[start..] {
            println!("{}", line);
        }
    }
    Ok(())
}

fn run_doctor(config_path: &str) -> Result<()> {
    println!("🔍 EchoPup 系统诊断\n");
    let mut issues = 0;
    let mut warnings = 0;

    // 1. 配置文件
    print!("[配置] ");
    match config::Config::load(config_path) {
        Ok(config) => {
            println!("✅ 配置加载成功");
            // LLM 配置
            print!("[LLM ] ");
            if config.is_llm_configured() {
                println!(
                    "✅ 已配置 (provider={}, model={})",
                    config.llm.provider, config.llm.model
                );
            } else {
                println!("⚠️  未配置（仅基础语音转文字）");
                warnings += 1;
            }
        }
        Err(e) => {
            println!("❌ 加载失败: {}", e);
            issues += 1;
        }
    }

    // 2. 音频设备
    print!("[音频] ");
    {
        use cpal::traits::{DeviceTrait, HostTrait};
        let host = cpal::default_host();
        match host.default_input_device() {
            Some(device) => {
                let name = device.name().unwrap_or_else(|_| "(未知)".to_string());
                match device.default_input_config() {
                    Ok(config) => println!(
                        "✅ {} ({}Hz, {}ch)",
                        name,
                        config.sample_rate().0,
                        config.channels()
                    ),
                    Err(e) => {
                        println!("⚠️  设备 {} 无法获取配置: {}", name, e);
                        issues += 1;
                    }
                }
            }
            None => {
                println!("❌ 未找到音频输入设备");
                issues += 1;
            }
        }
    }

    // 3. ASR 模型
    print!("[模型] ");
    let missing = model_download::check_missing_models();
    if missing.is_empty() {
        println!("✅ 所有模型文件已就绪");
    } else {
        println!("❌ 缺失 {} 个模型文件:", missing.len());
        for m in &missing {
            println!("       - {}", m);
        }
        issues += 1;
    }

    // 4. Linux 会话 / 热键约束
    #[cfg(target_os = "linux")]
    {
        print!("[会话] ");
        let session_type =
            std::env::var("XDG_SESSION_TYPE").unwrap_or_else(|_| "unknown".to_string());
        if session_type.eq_ignore_ascii_case("wayland") {
            println!(
                "✅ {}（使用外部触发 backend，建议将桌面快捷键 {} 绑定到 `echopup trigger toggle`）",
                session_type,
                default_trigger_key().to_uppercase()
            );

            print!("[输入] ");
            match (
                input::keyboard::preferred_linux_command_backend_label(),
                input::keyboard::preferred_linux_command_backend_note(),
            ) {
                (Some("eitype"), Some(note)) => {
                    println!("✅ Wayland 文本输入将优先使用 eitype（{}）", note);
                }
                (Some(backend), Some(note)) => {
                    println!("⚠️  Wayland 文本输入将优先使用 {}（{}）", backend, note);
                    warnings += 1;
                }
                (Some(backend), None) => {
                    println!("✅ Wayland 文本输入将优先使用 {}", backend);
                }
                (None, _) => {
                    println!("⚠️  未检测到 eitype/wtype/xdotool；Wayland 下文本输入可能失败");
                    warnings += 1;
                }
            }
        } else {
            println!("✅ {}", session_type);

            if let Some(warning) = internal_hotkey_conflict_warning() {
                print!("[热键] ");
                println!("⚠️  {}", warning);
                warnings += 1;
            }
        }
    }

    // 5. 运行状态
    print!("[运行] ");
    match runtime::running_instance_pid() {
        Ok(Some(pid)) => println!("✅ 后台服务运行中 (pid: {})", pid),
        Ok(None) => {
            println!("⚠️  后台服务未运行");
            warnings += 1;
        }
        Err(e) => {
            println!("⚠️  无法检查: {}", e);
            warnings += 1;
        }
    }

    println!();
    if issues == 0 && warnings == 0 {
        println!("✅ 所有检查通过！");
    } else if issues == 0 {
        println!("⚠️  核心环境可用，但有 {} 个注意项", warnings);
    } else if warnings == 0 {
        println!("⚠️  发现 {} 个问题", issues);
    } else {
        println!("⚠️  发现 {} 个问题，{} 个注意项", issues, warnings);
    }

    Ok(())
}

fn build_sherpa_paraformer_asr_engine(
    config: &config::config::Config,
    preview: bool,
) -> Result<asr::sherpa_paraformer::SherpaParaformerEngine> {
    let mut num_threads = config.asr.sherpa_paraformer.num_threads.max(1);
    if preview {
        num_threads = num_threads.clamp(1, 2);
    }

    let cfg = config::SherpaParaformerConfig {
        encoder_path: config.asr.sherpa_paraformer.encoder_path.clone(),
        decoder_path: config.asr.sherpa_paraformer.decoder_path.clone(),
        tokens_path: config.asr.sherpa_paraformer.tokens_path.clone(),
        provider: config.asr.sherpa_paraformer.provider.clone(),
        num_threads,
    };

    asr::sherpa_paraformer::SherpaParaformerEngine::new(cfg)
}

fn build_selected_asr_engine(
    config: &config::config::Config,
    preview: bool,
) -> Result<Box<dyn asr::AsrEngine>> {
    match config.asr.backend {
        config::AsrBackend::SherpaParaformer => Ok(Box::new(build_sherpa_paraformer_asr_engine(
            config, preview,
        )?)),
    }
}

fn build_asr_engine_with_fallback(
    config: &config::config::Config,
    preview: bool,
) -> Result<Box<dyn asr::AsrEngine>> {
    build_selected_asr_engine(config, preview)
}

fn spawn_background_asr_runtime_init(
    config: config::config::Config,
    asr_runtime: Arc<Mutex<Option<Box<dyn asr::AsrEngine>>>>,
    preview_asr_runtime: Arc<Mutex<Option<Box<dyn asr::AsrEngine>>>>,
) {
    std::thread::spawn(move || {
        match build_asr_engine_with_fallback(&config, false) {
            Ok(engine) => {
                log_asr_runtime("主转写运行时已启用", engine.as_ref());
                info!("ASR 运行时已初始化");
                let mut guard = asr_runtime.lock();
                if guard.is_none() {
                    *guard = Some(engine);
                }
            }
            Err(e) => {
                warn!("ASR 初始化失败: {}，语音转写功能不可用", e);
            }
        }

        match build_asr_engine_with_fallback(&config, true) {
            Ok(engine) => {
                log_asr_runtime("预览运行时已启用", engine.as_ref());
                let mut guard = preview_asr_runtime.lock();
                if guard.is_none() {
                    *guard = Some(engine);
                }
            }
            Err(e) => {
                warn!("ASR 预览运行时初始化失败: {}，将禁用流式预览", e);
            }
        }
    });
}

fn ensure_main_asr_runtime_ready(
    config_path: &str,
    asr_runtime: &Arc<Mutex<Option<Box<dyn asr::AsrEngine>>>>,
) -> bool {
    if asr_runtime.lock().is_some() {
        return true;
    }

    info!("ASR 运行时未就绪，尝试按需初始化主转写引擎");
    let cfg = match config::Config::load(config_path) {
        Ok(cfg) => cfg,
        Err(err) => {
            warn!("读取配置失败，无法按需初始化 ASR: {}", err);
            return false;
        }
    };

    match build_asr_engine_with_fallback(&cfg, false) {
        Ok(engine) => {
            log_asr_runtime("主转写运行时已按需初始化", engine.as_ref());
            let mut guard = asr_runtime.lock();
            if guard.is_none() {
                *guard = Some(engine);
            }
            true
        }
        Err(err) => {
            warn!("按需初始化 ASR 失败: {}", err);
            false
        }
    }
}

fn build_llm_runtime(llm_cfg: &config::config::LLMConfig) -> Option<llm::LLMRewrite> {
    if !llm_cfg.enabled {
        return None;
    }
    match llm::LLMRewrite::new(
        &llm_cfg.provider,
        &llm_cfg.api_base,
        &llm_cfg.api_key,
        &llm_cfg.model,
    ) {
        Ok(l) => Some(l),
        Err(err) => {
            warn!("LLM 热更新失败: {}", err);
            None
        }
    }
}

fn apply_runtime_menu_action(
    action: &menu_core::MenuAction,
    snapshot: &menu_core::MenuSnapshot,
    hotkey_listener: Option<&mut hotkey::HotkeyListener>,
    hotkey_trigger_mode_runtime: &Arc<Mutex<config::HotkeyTriggerMode>>,
    asr_runtime: &Arc<Mutex<Option<Box<dyn asr::AsrEngine>>>>,
    preview_asr_runtime: &Arc<Mutex<Option<Box<dyn asr::AsrEngine>>>>,
    llm_runtime: &Arc<Mutex<Option<llm::LLMRewrite>>>,
) -> Result<()> {
    match action {
        menu_core::MenuAction::SetHotkeyTriggerMode { mode } => {
            *hotkey_trigger_mode_runtime.lock() = *mode;
            info!("热键触发模式已热更新为 {}", mode.label());
        }
        menu_core::MenuAction::OpenConfigFolder => {
            #[cfg(target_os = "linux")]
            {
                status_indicator::open_config_folder_linux(&snapshot.config_path)?;
                info!("已打开配置文件夹: {}", snapshot.config_path);
            }
        }
        menu_core::MenuAction::OpenModelFolder => {
            #[cfg(target_os = "linux")]
            {
                status_indicator::open_model_folder_linux()?;
                info!("已打开模型文件夹");
            }
        }
        menu_core::MenuAction::SetField {
            field:
                menu_core::EditableField::LlmProvider
                | menu_core::EditableField::LlmModel
                | menu_core::EditableField::LlmApiBase
                | menu_core::EditableField::LlmApiKey,
            ..
        }
        | menu_core::MenuAction::SetLlmConfig { .. }
        | menu_core::MenuAction::ToggleLlmEnabled => {
            let cfg = config::Config::load(&snapshot.config_path)?;
            let mut guard = llm_runtime.lock();
            *guard = build_llm_runtime(&cfg.llm);
            info!("LLM 运行时配置已热更新");
        }
        menu_core::MenuAction::ReloadConfig => {
            let cfg = config::Config::load(&snapshot.config_path)?;

            if let Some(hotkey_listener) = hotkey_listener {
                hotkey_listener.set_hotkey(default_trigger_key())?;
                hotkey_listener.start()?;
            } else if uses_external_trigger_backend() {
                info!("当前会话使用外部触发 backend，跳过应用内热键重载");
            }
            *hotkey_trigger_mode_runtime.lock() = cfg.hotkey.trigger_mode;

            match build_asr_engine_with_fallback(&cfg, false) {
                Ok(new_asr) => {
                    log_asr_runtime("主转写运行时已重载", new_asr.as_ref());
                    let mut guard = asr_runtime.lock();
                    *guard = Some(new_asr);
                    drop(guard);
                    let mut callback_guard = preview_asr_runtime.lock();
                    *callback_guard = build_asr_engine_with_fallback(&cfg, true).ok();
                }
                Err(err) => {
                    warn!("ASR 重载失败: {}", err);
                    if let Some(current) = asr_runtime.lock().as_ref() {
                        let runtime = current.runtime_info();
                        warn!(
                            "ASR 重载失败后继续使用当前运行时: backend={}, model={}",
                            runtime.backend.label(),
                            runtime.model
                        );
                    }
                }
            }

            let mut llm_guard = llm_runtime.lock();
            *llm_guard = build_llm_runtime(&cfg.llm);
            info!("配置文件重载并已应用到运行时");
        }
        _ => {}
    }
    Ok(())
}

fn run_voice_input(config_path: &str) -> Result<()> {
    let _instance_guard = match runtime::InstanceGuard::try_acquire()? {
        Some(guard) => guard,
        None => {
            info!("实例已在运行，本次启动退出");
            return Ok(());
        }
    };
    print_banner();

    // ===== 首次运行引导 =====
    if config::Config::is_first_run(config_path) {
        println!("");
        println!("===========================================");
        println!("🎉 欢迎使用 EchoPup！");
        println!("===========================================");
        println!("");
        println!("首次运行，请先配置 LLM 以启用文本整理功能。");
        println!("");
        println!("📝 配置示例 (Ollama - 本地部署):");
        println!("");
        println!("  [llm]");
        println!("  enabled = true");
        println!("  provider = \"ollama\"");
        println!("  model = \"llama3\"");
        println!("  api_base = \"http://localhost:11434/v1\"");
        println!("  api_key = \"\"");
        println!("");
        println!("📝 配置示例 (OpenAI):");
        println!("");
        println!("  [llm]");
        println!("  enabled = true");
        println!("  provider = \"openai\"");
        println!("  model = \"gpt-4o-mini\"");
        println!("  api_base = \"https://api.openai.com/v1\"");
        println!("  api_key = \"\"");
        println!("");
        println!("💡 提示：");
        println!("  - Ollama: 从 https://ollama.com 下载安装");
        println!("  - 运行 'ollama serve' 启动 Ollama 服务");
        println!("  - 使用 'ollama pull llama3' 下载模型");
        println!("");
        println!(
            "编辑配置文件: {}",
            config_path.replace(
                "~",
                &dirs::home_dir().unwrap_or_default().display().to_string()
            )
        );
        println!("");
        println!("===========================================");
        println!("");

        // 如果默认配置下 LLM 未配置，也显示提示
        let default_config = config::Config::default();
        if !default_config.is_llm_configured() {
            info!("首次运行引导：LLM 未配置，将以基础模式运行（仅语音转文字）");
        }
    } else {
        // 非首次运行，检查 LLM 配置状态
        let config = config::Config::load(config_path)?;
        if !config.is_llm_configured() {
            info!("LLM 未配置，将以基础模式运行（仅语音转文字）");
        }
    }

    let config = config::Config::load(config_path)?;

    // ===== 自动下载缺失模型 =====
    if let Err(err) = ensure_all_models_downloaded() {
        warn!("模型自动下载失败: {}，部分功能可能不可用", err);
    }

    let mut menu_runtime = menu_core::MenuCore::new(config_path)?;
    print_macos_notification_setup_tip(config.feedback.notify_tip_on_start);

    let status_indicator = Arc::new(Mutex::new(status_indicator::StatusIndicatorClient::start(
        config.feedback.status_bar_enabled,
        config_path,
    )));
    let menu_snapshot_state = Arc::new(Mutex::new(menu_runtime.snapshot()));
    let status_indicator_enabled = {
        let guard = status_indicator.lock();
        guard.is_enabled()
    };
    if status_indicator_enabled {
        info!("状态栏反馈已启用: {}", status_indicator_surface_name());
        set_status_indicator_state(&status_indicator, status_indicator::IndicatorState::Idle);
        let snapshot = menu_snapshot_state.lock().clone();
        let mut guard = status_indicator.lock();
        guard.send_snapshot(&snapshot);
    } else if config.feedback.status_bar_enabled {
        warn!(
            "状态栏反馈未启用（{} 子进程未启动）",
            status_indicator_surface_name()
        );
    } else {
        info!("状态栏反馈已关闭（feedback.status_bar_enabled=false）");
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    let _terminal_echo_guard = match TerminalEchoGuard::try_disable_stdin_echo() {
        Ok(guard) => guard,
        Err(err) => {
            warn!("无法关闭终端输入回显，将继续运行: {}", err);
            None
        }
    };

    // ===== 初始化模块 =====
    let recorder = Arc::new(audio::AudioRecorder::new(
        config.audio.sample_rate,
        config.audio.channels,
    )?);
    info!("音频录制器已初始化");

    match config.asr.backend {
        config::AsrBackend::SherpaParaformer => {
            info!(
                "Sherpa Paraformer 已选中: encoder={}, decoder={}, tokens={}, provider={}",
                config.asr.sherpa_paraformer.encoder_path,
                config.asr.sherpa_paraformer.decoder_path,
                config.asr.sherpa_paraformer.tokens_path,
                config
                    .asr
                    .sherpa_paraformer
                    .provider
                    .as_deref()
                    .unwrap_or("cpu")
            );
        }
    }

    let asr_runtime = Arc::new(Mutex::new(None::<Box<dyn asr::AsrEngine>>));
    let preview_asr_runtime = Arc::new(Mutex::new(None::<Box<dyn asr::AsrEngine>>));
    info!("ASR 运行时将后台初始化，热键与录音会先启动");

    let llm = if config.llm.enabled {
        match build_llm_runtime(&config.llm) {
            Some(l) => {
                info!("LLM 整理已初始化");
                Some(l)
            }
            None => {
                warn!("LLM 初始化失败，文本整理功能不可用");
                None
            }
        }
    } else {
        info!("LLM 整理未启用");
        None
    };
    // 使用 Mutex 包装，以便在回调中共享
    let llm = Arc::new(Mutex::new(llm));

    let post_processor = Arc::new(text_processor::TextPostProcessor::new(
        &config.text_correction,
    ));
    if config.text_correction.enabled {
        info!(
            "谐音纠错已启用，规则数: {}",
            config.text_correction.homophone_map.len()
        );
    } else {
        info!("谐音纠错未启用");
    }

    let punct_restorer: Arc<Mutex<Option<punctuation::PunctuationRestorer>>> =
        Arc::new(Mutex::new(
            match punctuation::PunctuationRestorer::new(&config.punctuation) {
                Ok(restorer) => restorer,
                Err(e) => {
                    warn!("离线标点恢复初始化失败: {}，继续运行", e);
                    None
                }
            },
        ));

    let text_commit = Arc::new(Mutex::new(
        Box::new(commit::InsertOnlyTextCommit::new().map_err(|e| {
            error!("文本提交后端初始化失败: {}", e);
            e
        })?) as Box<dyn commit::TextCommitBackend>,
    ));
    {
        let commit_guard = text_commit.lock();
        info!(
            "文本提交后端已初始化: backend={}, draft_replace_supported={}",
            commit_guard.backend_name(),
            commit_guard.supports_draft_replacement()
        );
    }
    #[cfg(target_os = "linux")]
    if let Some(note) = input::keyboard::preferred_linux_command_backend_note() {
        info!("Linux 文本输入提示: {}", note);
    }

    let (desktop_notify_enabled, notify_desc) = detect_desktop_notify_capability();
    if desktop_notify_enabled {
        info!("桌面通知已启用: {}", notify_desc);
    } else {
        warn!("桌面通知未启用: {}", notify_desc);
    }
    #[cfg(target_os = "macos")]
    if desktop_notify_enabled {
        info!("提示：macOS 通知来源显示为“脚本编辑器”属于系统行为（由 osascript 发送）");
    }

    let (sound_feedback_enabled, sound_desc) =
        detect_sound_feedback_capability(config.feedback.sound_enabled);
    if sound_feedback_enabled {
        info!("提示音反馈已启用: {}", sound_desc);
    } else {
        warn!("提示音反馈未启用: {}", sound_desc);
    }

    // ===== 状态标记 =====
    let is_recording = Arc::new(AtomicBool::new(false));
    let partial_stt_should_stop = Arc::new(AtomicBool::new(false));
    let partial_stt_handle = Arc::new(Mutex::new(None::<std::thread::JoinHandle<()>>));
    let partial_stt_callback_handle = Arc::new(Mutex::new(None::<std::thread::JoinHandle<()>>));
    let recognition_session = Arc::new(Mutex::new(session::RecognitionSession::new()));

    // 录音动画控制
    let recording_animation = Arc::new(AtomicBool::new(false));
    let animation_should_stop = Arc::new(AtomicBool::new(false));

    // 启动录音动画线程
    let anim_is_recording = is_recording.clone();
    let anim_should_stop = animation_should_stop.clone();
    let animation_handle = std::thread::spawn(move || {
        let chars = ['|', '/', '-', '\\'];
        let mut index = 0;
        loop {
            if anim_should_stop.load(Ordering::SeqCst) {
                break;
            }
            if anim_is_recording.load(Ordering::SeqCst) {
                print!("\r🔴 录音中... {}", chars[index]);
                std::io::Write::flush(&mut std::io::stdout()).ok();
                index = (index + 1) % chars.len();
                std::thread::sleep(std::time::Duration::from_millis(200));
            } else {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }
    });

    // ===== 设置热键回调 =====
    #[derive(Default)]
    struct HotkeyPressState {
        pressed: bool,
        sequence: u64,
        started_by_hold_on_current_press: bool,
    }
    #[derive(Default)]
    struct ExternalToggleBurstState {
        sequence: u64,
    }
    let hold_to_record_duration = Duration::from_secs(1);
    let stop_press_debounce_window = Duration::from_millis(500);
    let external_toggle_burst_gap = Duration::from_millis(250);
    let hotkey_press_state = Arc::new(Mutex::new(HotkeyPressState::default()));
    let external_toggle_burst_state = Arc::new(Mutex::new(ExternalToggleBurstState::default()));
    let stop_debounce_until = Arc::new(Mutex::new(None::<Instant>));
    let hotkey_trigger_mode = Arc::new(Mutex::new(config.hotkey.trigger_mode));

    let recorder_start = recorder.clone();
    let preview_asr_runtime_on_start = preview_asr_runtime.clone();
    let is_recording_start = is_recording.clone();
    let recording_animation_start = recording_animation.clone();
    let desktop_notify_on_start = desktop_notify_enabled;
    let sound_feedback_on_start = sound_feedback_enabled;
    let status_indicator_on_start = status_indicator.clone();
    let menu_snapshot_on_start = menu_snapshot_state.clone();
    let stop_debounce_on_start = stop_debounce_until.clone();
    let partial_stt_stop_on_start = partial_stt_should_stop.clone();
    let partial_stt_handle_on_start = partial_stt_handle.clone();
    let partial_stt_callback_handle_on_start = partial_stt_callback_handle.clone();
    let recognition_session_on_start = recognition_session.clone();
    let text_commit_on_start = text_commit.clone();
    let start_recording_action: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
        info!(
            "start_recording_action 被调用, is_recording_start={}",
            is_recording_start.load(Ordering::SeqCst)
        );
        if !is_recording_start.load(Ordering::SeqCst) {
            clear_terminal_artifacts_if_tty();
            recording_animation_start.store(true, Ordering::SeqCst);
            info!("开始录音...");
            set_status_indicator_state(
                &status_indicator_on_start,
                status_indicator::IndicatorState::Recording,
            );
            match recorder_start.start() {
                Ok(_) => {
                    is_recording_start.store(true, Ordering::SeqCst);
                    *stop_debounce_on_start.lock() =
                        Some(Instant::now() + stop_press_debounce_window);
                    play_feedback_sound(
                        sound_feedback_on_start,
                        FeedbackSoundEvent::RecordingStart,
                    );
                    desktop_notify(desktop_notify_on_start, "EchoPup", "开始录音");

                    partial_stt_stop_on_start.store(false, Ordering::SeqCst);
                    recognition_session_on_start.lock().reset();
                    detach_thread_handle(&partial_stt_handle_on_start);
                    detach_thread_handle(&partial_stt_callback_handle_on_start);

                    let recorder_callback = recorder_start.clone();
                    let preview_asr_runtime_for_callback = preview_asr_runtime_on_start.clone();
                    let status_indicator_callback = status_indicator_on_start.clone();
                    let menu_snapshot_callback = menu_snapshot_on_start.clone();
                    let is_recording_callback = is_recording_start.clone();
                    let partial_stt_stop_callback = partial_stt_stop_on_start.clone();
                    let recognition_session_for_callback = recognition_session_on_start.clone();
                    let text_commit_for_callback = text_commit_on_start.clone();
                    let spinner_interval = Duration::from_millis(150);
                    let asr_poll_every_n: u32 = 3; // 每 3 个 spinner tick poll 一次 ASR (~450ms)
                    let min_samples = (recorder_callback.target_sample_rate() as usize)
                        .saturating_mul(800)
                        / 1000;
                    let max_preview_samples =
                        (recorder_callback.target_sample_rate() as usize).saturating_mul(3);
                    let callback_handle = std::thread::spawn(move || {
                        let mut preview_session: Box<dyn asr::AsrSession> = {
                            let asr_guard = preview_asr_runtime_for_callback.lock();
                            let Some(ref asr_engine) = *asr_guard else {
                                return;
                            };

                            match asr_engine.start_session(asr::AsrSessionConfig {
                                min_partial_samples: min_samples,
                                max_partial_window_samples: max_preview_samples,
                            }) {
                                Ok(session) => {
                                    info!(
                                        "ASR 预览会话已启动: backend={}, min_samples={}, window_samples={}",
                                        session.backend_kind().label(),
                                        min_samples,
                                        max_preview_samples
                                    );
                                    session
                                }
                                Err(err) => {
                                    warn!("创建 ASR 预览会话失败: {}", err);
                                    return;
                                }
                            }
                        };

                        let draft_enabled =
                            text_commit_for_callback.lock().supports_draft_replacement();

                        let mut preview_cursor: audio::AudioChunkCursor =
                            recorder_callback.incremental_cursor();

                        let mut tick_count: u32 = 0;

                        while is_recording_callback.load(Ordering::SeqCst)
                            && !partial_stt_stop_callback.load(Ordering::SeqCst)
                        {
                            std::thread::sleep(spinner_interval);
                            if !is_recording_callback.load(Ordering::SeqCst)
                                || partial_stt_stop_callback.load(Ordering::SeqCst)
                            {
                                break;
                            }

                            tick_count += 1;
                            let should_poll_asr = tick_count % asr_poll_every_n == 0;

                            if !should_poll_asr {
                                // 非 ASR poll tick：只旋转 spinner
                                if draft_enabled {
                                    let action = recognition_session_for_callback
                                        .lock()
                                        .tick_stability(false);
                                    if let Some(action) = action {
                                        if let Err(err) =
                                            text_commit_for_callback.lock().apply(action)
                                        {
                                            warn!("spinner 旋转失败: {}", err);
                                        }
                                    }
                                }
                                continue;
                            }

                            let new_samples = recorder_callback
                                .read_incremental_target_samples(&mut preview_cursor);
                            if new_samples.is_empty() {
                                // 即使无新音频，也旋转 spinner
                                if draft_enabled {
                                    let action = recognition_session_for_callback
                                        .lock()
                                        .tick_stability(false);
                                    if let Some(action) = action {
                                        if let Err(err) =
                                            text_commit_for_callback.lock().apply(action)
                                        {
                                            warn!("spinner 旋转失败: {}", err);
                                        }
                                    }
                                }
                                continue;
                            }

                            if let Err(err) = preview_session.accept_audio(&new_samples) {
                                warn!("ASR 预览会话接收音频失败: {}", err);
                                break;
                            }

                            match preview_session.poll_partial(partial_stt_stop_callback.clone()) {
                                Ok(Some(text)) => {
                                    if partial_stt_stop_callback.load(Ordering::SeqCst) {
                                        break;
                                    }

                                    let (update, draft_action) = {
                                        let mut session_guard =
                                            recognition_session_for_callback.lock();
                                        let update = session_guard.update_partial(&text);
                                        let draft_action = if draft_enabled {
                                            session_guard.prepare_draft_commit(&text)
                                        } else {
                                            None
                                        };
                                        (update, draft_action)
                                    };

                                    if let Some(update) = update {
                                        send_status_snapshot(
                                            &status_indicator_callback,
                                            &menu_snapshot_callback,
                                            update.status_text,
                                        );
                                    }

                                    if let Some(action) = draft_action {
                                        if let Err(err) =
                                            text_commit_for_callback.lock().apply(action)
                                        {
                                            warn!("草稿提交失败: {}", err);
                                        }
                                    }
                                }
                                Ok(None) => {
                                    // 无新 partial：旋转 spinner
                                    if draft_enabled {
                                        let action = recognition_session_for_callback
                                            .lock()
                                            .tick_stability(false);
                                        if let Some(action) = action {
                                            if let Err(err) =
                                                text_commit_for_callback.lock().apply(action)
                                            {
                                                warn!("spinner 旋转失败: {}", err);
                                            }
                                        }
                                    }
                                }
                                Err(err) => {
                                    warn!(
                                        "ASR 预览会话 partial 识别失败: backend={}, buffered_samples={}, err={}",
                                        preview_session.backend_kind().label(),
                                        preview_session.buffered_samples(),
                                        err
                                    );
                                }
                            }
                        }
                    });
                    *partial_stt_callback_handle_on_start.lock() = Some(callback_handle);
                }
                Err(e) => {
                    error!("开始录音失败: {}", e);
                    recording_animation_start.store(false, Ordering::SeqCst);
                    set_status_indicator_state(
                        &status_indicator_on_start,
                        status_indicator::IndicatorState::Failed,
                    );
                    desktop_notify(desktop_notify_on_start, "EchoPup", "开始录音失败");
                }
            }
        }
    });

    let recorder_stop = recorder.clone();
    let asr_runtime_on_stop = asr_runtime.clone();
    let llm_stop = llm.clone();
    let post_processor_stop = post_processor.clone();
    let punct_restorer_stop = punct_restorer.clone();
    let text_commit_stop = text_commit.clone();
    let is_recording_stop = is_recording.clone();
    let recording_animation_stop = recording_animation.clone();
    let config_path_on_stop = config_path.to_string();
    let desktop_notify_on_stop = desktop_notify_enabled;
    let sound_feedback_on_stop = sound_feedback_enabled;
    let status_indicator_on_stop = status_indicator.clone();
    let stop_debounce_on_stop = stop_debounce_until.clone();
    let partial_stt_stop_on_stop = partial_stt_should_stop.clone();
    let partial_stt_handle_on_stop = partial_stt_handle.clone();
    let partial_stt_callback_handle_on_stop = partial_stt_callback_handle.clone();
    let recognition_session_on_stop = recognition_session.clone();
    let stop_recording_action: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
        info!("stop_recording_action 被调用");
        if is_recording_stop.load(Ordering::SeqCst) {
            *stop_debounce_on_stop.lock() = None;
            is_recording_stop.store(false, Ordering::SeqCst);
            recording_animation_stop.store(false, Ordering::SeqCst);
            print!("\r");
            clear_terminal_artifacts_if_tty();
            partial_stt_stop_on_stop.store(true, Ordering::SeqCst);
            info!("stop_recording_action: 准备调用 recorder.stop()");
            let audio_data = match recorder_stop.stop() {
                Ok(data) => data,
                Err(e) => {
                    error!("停止录音失败: {}", e);
                    set_status_indicator_state(
                        &status_indicator_on_stop,
                        status_indicator::IndicatorState::Failed,
                    );
                    desktop_notify(desktop_notify_on_stop, "EchoPup", "停止录音失败");
                    return;
                }
            };
            info!("stop_recording_action: recorder.stop() 返回成功");
            // 草稿保留在光标处，final commit 时通过增量 diff 平滑过渡
            info!("stop_recording_action: 预览线程转为后台回收");
            detach_thread_handle(&partial_stt_handle_on_stop);
            detach_thread_handle(&partial_stt_callback_handle_on_stop);
            play_feedback_sound(sound_feedback_on_stop, FeedbackSoundEvent::RecordingEnd);

            let e2e_start = Instant::now();
            set_status_indicator_state(
                &status_indicator_on_stop,
                status_indicator::IndicatorState::Transcribing,
            );
            desktop_notify(desktop_notify_on_stop, "EchoPup", "识别中...");

            let ok = process_audio(
                &audio_data,
                &config_path_on_stop,
                &asr_runtime_on_stop,
                &llm_stop,
                &post_processor_stop,
                &punct_restorer_stop,
                &text_commit_stop,
                &recognition_session_on_stop,
                e2e_start,
                desktop_notify_on_stop,
            );
            set_status_indicator_state(
                &status_indicator_on_stop,
                if ok {
                    status_indicator::IndicatorState::Completed
                } else {
                    status_indicator::IndicatorState::Failed
                },
            );
        }
    });

    let press_state_on_press = hotkey_press_state.clone();
    let start_action_on_press = start_recording_action.clone();
    let stop_action_on_press = stop_recording_action.clone();
    let is_recording_on_press = is_recording.clone();
    let stop_debounce_on_press = stop_debounce_until.clone();
    let mode_on_press = hotkey_trigger_mode.clone();
    let press_callback: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
        let trigger_mode = *mode_on_press.lock();
        let seq = {
            let mut state = press_state_on_press.lock();
            if state.pressed {
                debug!("press_callback: state.pressed is true, returning early");
                return;
            }
            debug!(
                "press_callback: setting pressed=true, seq={}",
                state.sequence.wrapping_add(1)
            );
            state.pressed = true;
            state.sequence = state.sequence.wrapping_add(1);
            state.started_by_hold_on_current_press = false;
            state.sequence
        };

        if is_recording_on_press.load(Ordering::SeqCst) {
            if trigger_mode != config::HotkeyTriggerMode::PressToToggle {
                return;
            }
            if let Some(deadline) = *stop_debounce_on_press.lock() {
                if Instant::now() < deadline {
                    return;
                }
            }
            stop_action_on_press();
            return;
        }

        let press_state_timer = press_state_on_press.clone();
        let start_action_timer = start_action_on_press.clone();
        let is_recording_timer = is_recording_on_press.clone();
        std::thread::spawn(move || {
            std::thread::sleep(hold_to_record_duration);
            let should_start = {
                let mut state = press_state_timer.lock();
                if !(state.pressed && state.sequence == seq) {
                    false
                } else {
                    state.started_by_hold_on_current_press = true;
                    true
                }
            };
            if !should_start || is_recording_timer.load(Ordering::SeqCst) {
                return;
            }
            start_action_timer();
        });
    });

    let press_state_on_release = hotkey_press_state.clone();
    let is_recording_on_release = is_recording.clone();
    let stop_action_on_release = stop_recording_action.clone();
    let mode_on_release = hotkey_trigger_mode.clone();
    let release_callback: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
        info!("release_callback 被调用");
        let started_by_hold = {
            let mut state = press_state_on_release.lock();
            if !state.pressed {
                info!("release_callback: state.pressed was false, returning early");
                return;
            }
            state.pressed = false;
            state.sequence = state.sequence.wrapping_add(1);
            let started = state.started_by_hold_on_current_press;
            state.started_by_hold_on_current_press = false;
            info!("release_callback: started_by_hold={}", started);
            started
        };

        let mode = *mode_on_release.lock();
        let recording = is_recording_on_release.load(Ordering::SeqCst);
        info!(
            "release_callback: mode={:?}, started_by_hold={}, is_recording={}",
            mode, started_by_hold, recording
        );

        if mode == config::HotkeyTriggerMode::HoldToRecord && started_by_hold && recording {
            info!("release_callback: calling stop_action_on_release");
            stop_action_on_release();
        }
    });

    let toggle_recording_action: Arc<dyn Fn() + Send + Sync> = {
        let is_recording_toggle = is_recording.clone();
        let start_recording_toggle = start_recording_action.clone();
        let stop_recording_toggle = stop_recording_action.clone();
        Arc::new(move || {
            if is_recording_toggle.load(Ordering::SeqCst) {
                stop_recording_toggle();
            } else {
                start_recording_toggle();
            }
        })
    };
    let external_toggle_action: Arc<dyn Fn() + Send + Sync> = if uses_external_trigger_backend() {
        let burst_state = external_toggle_burst_state.clone();
        let toggle_action = toggle_recording_action.clone();
        Arc::new(move || {
            let sequence = {
                let mut state = burst_state.lock();
                state.sequence = state.sequence.wrapping_add(1);
                state.sequence
            };
            let burst_state_for_wait = burst_state.clone();
            let toggle_action_for_wait = toggle_action.clone();
            std::thread::spawn(move || {
                std::thread::sleep(external_toggle_burst_gap);
                let should_fire = burst_state_for_wait.lock().sequence == sequence;
                if should_fire {
                    toggle_action_for_wait();
                }
            });
        })
    } else {
        toggle_recording_action.clone()
    };

    // ===== 初始化热键监听器 / 外部触发 =====
    let external_trigger_enabled = uses_external_trigger_backend();
    let mut hotkey = if external_trigger_enabled {
        None
    } else {
        let mut hotkey = hotkey::HotkeyListener::new()?;
        hotkey.set_hotkey(default_trigger_key())?;
        hotkey.on_press(press_callback.clone());
        hotkey.on_release(release_callback.clone());
        hotkey.start()?;
        Some(hotkey)
    };

    #[cfg(target_os = "linux")]
    let external_trigger_server = if external_trigger_enabled {
        let socket_path = runtime::trigger_socket_path()?;
        let press_action = press_callback.clone();
        let release_action = release_callback.clone();
        let toggle_action = external_toggle_action.clone();
        Some(trigger::ExternalTriggerServer::start(
            socket_path.clone(),
            move |action| match action {
                trigger::ExternalTriggerAction::Press => press_action(),
                trigger::ExternalTriggerAction::Release => release_action(),
                trigger::ExternalTriggerAction::Toggle => toggle_action(),
            },
        )?)
    } else {
        None
    };

    #[cfg(not(target_os = "linux"))]
    let external_trigger_server: Option<trigger::ExternalTriggerServer> = None;

    spawn_background_asr_runtime_init(
        config.clone(),
        asr_runtime.clone(),
        preview_asr_runtime.clone(),
    );

    // ===== 设置 Ctrl+C 信号处理 =====
    let (tx, rx) = mpsc::channel::<()>();
    let tx_clone = tx.clone();
    ctrlc::set_handler(move || {
        let _ = tx_clone.send(());
    })
    .expect("Error setting Ctrl+C handler");

    info!("===========================================");
    info!("🎤 EchoPup 语音输入已启动");
    info!("   触发后端: {}", trigger_backend_description());
    info!("   模式: {}", config.hotkey.trigger_mode.label());
    if external_trigger_enabled {
        info!(
            "   桌面快捷键建议: {} -> `echopup trigger toggle`",
            default_trigger_key().to_uppercase()
        );
        if config.hotkey.trigger_mode == config::HotkeyTriggerMode::HoldToRecord {
            info!("   注: 当前外部快捷键默认使用 toggle 语义；长按语义仅对 `trigger press/release` 生效");
        }
    } else {
        info!("   热键: {} (固定)", default_trigger_key());
        info!("   长按 1 秒开始录音");
        match config.hotkey.trigger_mode {
            config::HotkeyTriggerMode::HoldToRecord => {
                info!("   松开热键后停止录音并开始转写");
            }
            config::HotkeyTriggerMode::PressToToggle => {
                info!("   松开后继续录音，下一次按下热键结束并转写");
            }
        }
    }
    #[cfg(target_os = "linux")]
    if let Some(warning) = internal_hotkey_conflict_warning() {
        warn!("{}", warning);
    }
    info!("   按 Ctrl+C 退出");
    info!("===========================================");

    // ===== 主循环 =====
    let mut shutdown_requested = false;
    loop {
        {
            let mut guard = status_indicator.lock();
            while let Some(action) = guard.try_recv_action() {
                let action_for_runtime = action.clone();
                let mut result = menu_runtime.execute(action);
                if result.ok {
                    if let Err(err) = apply_runtime_menu_action(
                        &action_for_runtime,
                        &result.snapshot,
                        hotkey.as_mut(),
                        &hotkey_trigger_mode,
                        &asr_runtime,
                        &preview_asr_runtime,
                        &llm,
                    ) {
                        warn!("菜单动作热更新失败: {}", err);
                        menu_runtime.set_status(format!("已保存配置，但热更新失败: {}", err));
                        result.snapshot = menu_runtime.snapshot();
                    }
                }
                // 同步共享快照，确保录音回调等异步路径拿到最新状态
                *menu_snapshot_state.lock() = result.snapshot.clone();
                guard.send_action_result(&result);
                if result.quit_ui {
                    guard.close_ui();
                    shutdown_requested = true;
                    break;
                }
            }
        }

        if shutdown_requested {
            info!("收到菜单退出指令，准备退出主进程");
            break;
        }

        match rx.recv_timeout(Duration::from_millis(500)) {
            Ok(_) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                info!("收到退出信号，正在优雅退出...");
                break;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // 继续运行
            }
        }
    }

    // 停止动画线程
    partial_stt_should_stop.store(true, Ordering::SeqCst);
    join_thread_handle(&partial_stt_handle);
    join_thread_handle(&partial_stt_callback_handle);
    animation_should_stop.store(true, Ordering::SeqCst);
    recording_animation.store(false, Ordering::SeqCst);
    let _ = animation_handle.join();
    set_status_indicator_state(&status_indicator, status_indicator::IndicatorState::Idle);
    drop(external_trigger_server);

    info!("EchoPup 已退出");
    Ok(())
}
