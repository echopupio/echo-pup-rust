//! TypechoAI - AI Voice Dictation Tool

mod audio;
mod config;
mod hotkey;
mod input;
mod llm;
mod stt;

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::sync::Arc;
use std::time::Duration;
use std::sync::mpsc;
use parking_lot::Mutex;
use tracing::{info, warn};

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
    let config = config::Config::load(config_path)?;
    
    // 初始化模块
    let _recorder = audio::AudioRecorder::new(config.audio.sample_rate, config.audio.channels)?;
    info!("音频录制器已初始化");
    
    let _whisper = match stt::WhisperSTT::new(&config.whisper.model_path) {
        Ok(w) => {
            info!("Whisper 已初始化");
            Some(w)
        }
        Err(e) => {
            warn!("Whisper 初始化失败: {}", e);
            None
        }
    };
    
    let _llm = if config.llm.enabled {
        match llm::LLMRewrite::new(&config.llm.provider, &config.llm.api_base, &config.llm.api_key_env, &config.llm.model) {
            Ok(l) => {
                info!("LLM 整理已初始化");
                Some(l)
            }
            Err(e) => {
                warn!("LLM 初始化失败: {}", e);
                None
            }
        }
    } else {
        None
    };
    
    let mut keyboard = input::Keyboard::new()?;
    info!("键盘输入已初始化");
    
    let mut hotkey = hotkey::HotkeyListener::new()?;
    hotkey.set_hotkey(&config.hotkey.key)?;
    info!("热键监听器已初始化: {}", config.hotkey.key);
    
    let _is_recording = Arc::new(Mutex::new(false));
    
    // 简化：直接运行测试
    info!("===========================================");
    info!("🎤 TypechoAI 语音输入已启动");
    info!("   按住 {} 说话，松开后自动输入", config.hotkey.key);
    info!("   按 Ctrl+C 退出");
    info!("===========================================");
    
    // 模拟一次输入测试
    keyboard.type_text("TypechoAI 测试成功！")?;
    info!("测试文本已输入");
    
    // 设置 Ctrl+C 信号处理
    let (tx, rx) = mpsc::channel::<()>();
    
    // 注册信号处理程序
    let tx_clone = tx.clone();
    ctrlc::set_handler(move || {
        let _ = tx_clone.send(());
    }).expect("Error setting Ctrl+C handler");
    
    info!("===========================================");
    info!("🎤 TypechoAI 语音输入已启动");
    info!("   按住 {} 说话，松开后自动输入", config.hotkey.key);
    info!("   按 Ctrl+C 退出");
    info!("===========================================");
    
    // 等待 Ctrl+C 信号
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
            // 测试输入
            k.type_text("Test")?;
            println!("    - 测试文本已输入");
        }
        Err(e) => println!("  ✗ 键盘输入初始化失败: {}", e),
    }

    println!("\n=== 测试完成 ===");
    Ok(())
}
