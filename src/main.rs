//! TypechoAI - AI Voice Dictation Tool

mod audio;
mod config;
mod hotkey;
mod input;
mod llm;
mod stt;
mod vad;

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use std::sync::mpsc;
use parking_lot::Mutex;
use tracing::{info, error, warn};

#[derive(Parser)]
#[command(name = "typechoai")]
#[command(about = "AI Voice Dictation Tool", long_about = None)]
struct Cli {
    #[arg(short, long, default_value = "~/.typechoai/config.toml")]
    config: String,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Run,
    Test,
    Config { show: bool, init: bool },
    DownloadModel { size: String },
}

/// 处理音频数据：转写 -> LLM 整理 -> 键盘输入
/// is_vad_triggered: 是否由 VAD 自动触发（用于日志区分）
fn process_audio(
    audio_data: &[f32],
    whisper: &Arc<Mutex<Option<stt::WhisperSTT>>>,
    llm: &Arc<Mutex<Option<llm::LLMRewrite>>>,
    keyboard: &Arc<Mutex<input::Keyboard>>,
    is_vad_triggered: bool,
) {
    let trigger_type = if is_vad_triggered { "VAD自动" } else { "热键松开" };
    
    if audio_data.is_empty() {
        info!("[{}] 录音数据为空", trigger_type);
        return;
    }
    info!("[{}] 录音完成，采样点: {}", trigger_type, audio_data.len());

    // 1. VAD 语音活动检测 - 过滤静音段
    let processed_audio = {
        // 自动调整阈值以适应不同的麦克风音量
        let mut vad_clone = vad::VadDetector::new();
        vad_clone.auto_threshold(audio_data);

        let speech_audio = vad_clone.filter_speech(audio_data);
        if speech_audio.is_empty() {
            info!("未检测到语音内容");
            return;
        }
        let ratio = speech_audio.len() as f32 / audio_data.len() as f32;
        info!("VAD 过滤后保留 {:.1}% 语音数据", ratio * 100.0);
        speech_audio
    };

    // 2. 音频转写 (Whisper)
    let mut final_text = String::new();
    let mut transcribe_success = false;

    {
        let mut whisper_guard = whisper.lock();
        if let Some(ref mut whisper) = *whisper_guard {
            match whisper.transcribe(&processed_audio) {
                Ok(text) => {
                    // 过滤无效结果
                    let trimmed = text.trim();
                    if trimmed.is_empty() || trimmed == "[BLANK_AUDIO]" {
                        info!("转写结果为空或无效（可能没有说话或音量太小）");
                        return;
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
            error!("Whisper 未初始化");
        }
    }
    
    if !transcribe_success {
        return;
    }
    
    // 3. LLM 整理（如果启用）
    let llm_enabled = {
        let llm_guard = llm.lock();
        llm_guard.as_ref().map(|l| l.is_enabled()).unwrap_or(false)
    };
    
    if llm_enabled {
        let llm_guard = llm.lock();
        if let Some(ref llm) = *llm_guard {
            match llm.rewrite(&final_text) {
                Ok(rewritten) => {
                    info!("LLM 整理完成: {}", rewritten);
                    final_text = rewritten;
                }
                Err(e) => {
                    error!("LLM 整理失败: {}，使用原始转写结果", e);
                }
            }
        }
    }
    
    // 4. 键盘输入
    {
        let mut keyboard_guard = keyboard.lock();
        match keyboard_guard.type_text(&final_text) {
            Ok(_) => {
                info!("文本已输入");
            }
            Err(e) => {
                error!("键盘输入失败: {}", e);
            }
        }
    }
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into()),
        )
        .init();

    info!("TypechoAI 启动中...");

    let cli = Cli::parse();

    match cli.command {
        Commands::Run => run_voice_input(&cli.config)?,
        Commands::Test => test_modules(&cli.config)?,
        Commands::Config { show, init } => {
            if init {
                let config = config::Config::default();
                config.save(&cli.config)?;
                info!("默认配置已保存到: {}", cli.config);
            }
            if show {
                let config = config::Config::load(&cli.config)?;
                println!("{:#?}", config);
            }
        }
        Commands::DownloadModel { size } => {
            info!("下载 Whisper {} 模型", size);
            println!("请运行: ./scripts/download_model.sh {}", size);
        }
    }

    Ok(())
}

fn run_voice_input(config_path: &str) -> Result<()> {
    // ===== 首次运行引导 =====
    if config::Config::is_first_run(config_path) {
        println!("");
        println!("===========================================");
        println!("🎉 欢迎使用 TypechoAI！");
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
        println!("  api_key_env = \"\"");
        println!("");
        println!("📝 配置示例 (OpenAI):");
        println!("");
        println!("  [llm]");
        println!("  enabled = true");
        println!("  provider = \"openai\"");
        println!("  model = \"gpt-4o-mini\"");
        println!("  api_base = \"https://api.openai.com/v1\"");
        println!("  api_key_env = \"OPENAI_API_KEY\"");
        println!("");
        println!("💡 提示：");
        println!("  - Ollama: 从 https://ollama.com 下载安装");
        println!("  - 运行 'ollama serve' 启动 Ollama 服务");
        println!("  - 使用 'ollama pull llama3' 下载模型");
        println!("");
        println!("编辑配置文件: {}", config_path.replace("~", &dirs::home_dir().unwrap_or_default().display().to_string()));
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
    
    // ===== 初始化模块 =====
    let recorder = Arc::new(audio::AudioRecorder::new(
        config.audio.sample_rate, 
        config.audio.channels
    )?);
    info!("音频录制器已初始化");
    
    let whisper = match stt::WhisperSTT::new(&config.whisper.model_path) {
        Ok(w) => {
            info!("Whisper 已初始化");
            Some(w)
        }
        Err(e) => {
            warn!("Whisper 初始化失败: {}，语音转写功能不可用", e);
            None
        }
    };
    // 使用 Mutex 包装，以便在回调中共享（transcribe 需要 &mut self）
    let whisper = Arc::new(Mutex::new(whisper));

    let llm = if config.llm.enabled {
        match llm::LLMRewrite::new(
            &config.llm.provider, 
            &config.llm.api_base, 
            &config.llm.api_key_env, 
            &config.llm.model
        ) {
            Ok(l) => {
                info!("LLM 整理已初始化");
                Some(l)
            }
            Err(e) => {
                warn!("LLM 初始化失败: {}，文本整理功能不可用", e);
                None
            }
        }
    } else {
        info!("LLM 整理未启用");
        None
    };
    // 使用 Mutex 包装，以便在回调中共享
    let llm = Arc::new(Mutex::new(llm));
    
    let keyboard = Arc::new(Mutex::new(input::Keyboard::new()?));
    info!("键盘输入已初始化");
    
    // ===== 状态标记 =====
    let is_recording = Arc::new(AtomicBool::new(false));
    let vad_triggered = Arc::new(AtomicBool::new(false));  // VAD 触发标记
    
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
    // 按下热键时开始录音
    let recorder_press = recorder.clone();
    let is_recording_press = is_recording.clone();
    let recording_animation_press = recording_animation.clone();
    let press_callback: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
        if !is_recording_press.load(Ordering::SeqCst) {
            // 启动动画
            recording_animation_press.store(true, Ordering::SeqCst);
            info!("开始录音...");
            match recorder_press.start() {
                Ok(_) => {
                    is_recording_press.store(true, Ordering::SeqCst);
                }
                Err(e) => {
                    error!("开始录音失败: {}", e);
                    recording_animation_press.store(false, Ordering::SeqCst);
                }
            }
        }
    });
    
    // 松开热键时停止录音并处理（转写 -> LLM整理 -> 键盘输入）
    let recorder_release = recorder.clone();
    let whisper_release = whisper.clone();
    let llm_release = llm.clone();
    let keyboard_release = keyboard.clone();
    let is_recording_release = is_recording.clone();
    let vad_triggered_release = vad_triggered.clone();
    let recording_animation_release = recording_animation.clone();
    let release_callback: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
        // 无论 VAD 是否触发，都需要检查是否正在录音
        if is_recording_release.load(Ordering::SeqCst) {
            // 先标记为非录音状态
            is_recording_release.store(false, Ordering::SeqCst);
            // 停止动画
            recording_animation_release.store(false, Ordering::SeqCst);
            print!("\r");  // 清除动画行
            
            // 获取音频数据
            let audio_data = match recorder_release.stop() {
                Ok(data) => data,
                Err(e) => {
                    error!("停止录音失败: {}", e);
                    return;
                }
            };
            
            // 检查是否由 VAD 触发
            let is_vad = vad_triggered_release.swap(false, Ordering::SeqCst);
            
            process_audio(&audio_data, &whisper_release, &llm_release, &keyboard_release, is_vad);
        }
    });
    
    // ===== 设置端点检测（VAD）回调 =====
    // 当检测到语音结束时自动触发转写
    let vad_recorder = recorder.clone();
    let vad_whisper = whisper.clone();
    let vad_llm = llm.clone();
    let vad_keyboard = keyboard.clone();
    let vad_is_recording = is_recording.clone();
    let vad_triggered_callback = vad_triggered.clone();
    let vad_recording_animation = recording_animation.clone();
    
    let vad_callback: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
        info!("端点检测：语音结束，触发自动转写");
        
        // 标记 VAD 已触发
        vad_triggered_callback.store(true, Ordering::SeqCst);
        
        // 停止录音
        if vad_is_recording.load(Ordering::SeqCst) {
            vad_is_recording.store(false, Ordering::SeqCst);
            // 停止动画
            vad_recording_animation.store(false, Ordering::SeqCst);
            print!("\r");  // 清除动画行
        }
        
        // 获取音频数据并处理
        let audio_data = match vad_recorder.stop() {
            Ok(data) => data,
            Err(e) => {
                error!("VAD 停止录音失败: {}", e);
                return;
            }
        };
        
        // 处理音频
        process_audio(&audio_data, &vad_whisper, &vad_llm, &vad_keyboard, true);
    });
    
    // 配置 VAD 参数并启用
    recorder.set_vad_params(1500, 0.01);  // 1.5秒静音阈值，能量阈值 0.01
    recorder.set_vad_callback(move || {
        // 调用 VAD 回调
        vad_callback();
    });
    recorder.enable_vad();
    info!("端点检测已启用：持续静音 1500ms 将自动结束录音");
    
    // ===== 初始化热键监听器 =====
    let mut hotkey = hotkey::HotkeyListener::new()?;
    hotkey.set_hotkey(&config.hotkey.key)?;
    hotkey.on_press(press_callback);
    hotkey.on_release(release_callback);
    hotkey.start()?;
    
    // ===== 设置 Ctrl+C 信号处理 =====
    let (tx, rx) = mpsc::channel::<()>();
    let tx_clone = tx.clone();
    ctrlc::set_handler(move || {
        let _ = tx_clone.send(());
    }).expect("Error setting Ctrl+C handler");
    
    info!("===========================================");
    info!("🎤 TypechoAI 语音输入已启动");
    info!("   按住 {} 说话，松开后自动输入", config.hotkey.key);
    info!("   按 Ctrl+C 退出");
    info!("===========================================");
    
    // ===== 主循环 =====
    loop {
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
    animation_should_stop.store(true, Ordering::SeqCst);
    recording_animation.store(false, Ordering::SeqCst);
    let _ = animation_handle.join();
    
    info!("TypechoAI 已退出");
    Ok(())
}

fn test_modules(config_path: &str) -> Result<()> {
    println!("=== 测试各模块 ===\n");

    println!("[1/4] 测试配置模块...");
    let config = config::Config::load(config_path)?;
    println!("  ✓ 配置加载成功");

    println!("\n[2/4] 测试音频录制器...");
    match audio::AudioRecorder::new(config.audio.sample_rate, config.audio.channels) {
        Ok(_) => println!("  ✓ 音频录制器创建成功"),
        Err(e) => println!("  ✗ 音频录制器创建失败: {}", e),
    }

    println!("\n[3/4] 测试 Whisper...");
    match stt::WhisperSTT::new(&config.whisper.model_path) {
        Ok(w) => {
            if w.is_ready() {
                println!("  ✓ Whisper 模型加载成功");
            } else {
                println!("  ~ Whisper 模型未找到");
            }
        }
        Err(e) => println!("  ~ Whisper: {}", e),
    }

    println!("\n[4/4] 测试键盘输入...");
    match input::Keyboard::new() {
        Ok(mut k) => {
            println!("  ✓ 键盘输入初始化成功");
            k.type_text("Test")?;
            println!("    - 测试文本已输入");
        }
        Err(e) => println!("  ✗ 键盘输入初始化失败: {}", e),
    }

    println!("\n=== 测试完成 ===");
    Ok(())
}
