//! TypechoAI - AI Voice Dictation Tool
//!
//! 核心流程: 按住 F12 → 说话 → Whisper 转写 → LLM 整理 → 自动输入

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
use parking_lot::Mutex;
use tracing::{info, error, warn};

#[derive(Parser)]
#[command(name = "typechoai")]
#[command(about = "AI Voice Dictation Tool", long_about = None)]
struct Cli {
    /// 配置文件路径
    #[arg(short, long, default_value = "~/.typechoai/config.toml")]
    config: String,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// 运行语音输入
    Run,
    /// 测试各模块
    Test,
    /// 配置管理
    Config {
        /// 显示当前配置
        #[arg(short, long)]
        show: bool,

        /// 生成默认配置
        #[arg(short, long)]
        init: bool,
    },
    /// 下载 Whisper 模型
    DownloadModel {
        /// 模型大小: tiny, base, small, medium, large
        #[arg(default_value = "small")]
        size: String,
    },
}

/// 应用状态
struct AppState {
    config: config::Config,
    recorder: Option<audio::AudioRecorder>,
    whisper: Option<stt::WhisperSTT>,
    llm: Option<llm::LLMRewrite>,
    keyboard: Option<input::Keyboard>,
    hotkey: Option<hotkey::HotkeyListener>,
    is_recording: Arc<Mutex<bool>>,
}

impl AppState {
    fn new(config_path: &str) -> Result<Self> {
        let config = config::Config::load(config_path)?;

        Ok(Self {
            config,
            recorder: None,
            whisper: None,
            llm: None,
            keyboard: None,
            hotkey: None,
            is_recording: Arc::new(Mutex::new(false)),
        })
    }

    fn init_modules(&mut self) -> Result<()> {
        // 初始化音频录制器
        self.recorder = Some(audio::AudioRecorder::new(
            self.config.audio.sample_rate,
            self.config.audio.channels,
        )?);
        info!("音频录制器已初始化");

        // 初始化 Whisper
        match stt::WhisperSTT::new(&self.config.whisper.model_path) {
            Ok(w) => {
                self.whisper = Some(w);
                info!("Whisper 已初始化");
            }
            Err(e) => {
                warn!("Whisper 初始化失败: {}，语音转写功能不可用", e);
            }
        }

        // 初始化 LLM
        if self.config.llm.enabled {
            match llm::LLMRewrite::new(
                &self.config.llm.provider,
                &self.config.llm.api_base,
                &self.config.llm.api_key_env,
                &self.config.llm.model,
            ) {
                Ok(l) => {
                    self.llm = Some(l);
                    info!("LLM 整理已初始化");
                }
                Err(e) => {
                    warn!("LLM 初始化失败: {}，文本整理功能不可用", e);
                }
            }
        }

        // 初始化键盘
        match input::Keyboard::new() {
            Ok(k) => {
                self.keyboard = Some(k);
                info!("键盘输入已初始化");
            }
            Err(e) => {
                error!("键盘输入初始化失败: {}", e);
            }
        }

        // 初始化热键监听器
        match hotkey::HotkeyListener::new() {
            Ok(mut h) => {
                h.set_hotkey(&self.config.hotkey.key)?;
                self.hotkey = Some(h);
                info!("热键监听器已初始化: {}", self.config.hotkey.key);
            }
            Err(e) => {
                error!("热键监听器初始化失败: {}", e);
            }
        }

        Ok(())
    }

    fn run_voice_input(&mut self) -> Result<()> {
        let is_recording = self.is_recording.clone();
        let recorder = self.recorder.as_ref().ok_or_else(|| anyhow::anyhow!("录音器未初始化"))?;
        let whisper = self.whisper.as_ref();
        let llm = self.llm.as_ref();
        let mut keyboard = self.keyboard.take().ok_or_else(|| anyhow::anyhow!("键盘未初始化"))?;

        // 设置热键回调
        {
            let mut hotkey = self.hotkey.as_mut().ok_or_else(|| anyhow::anyhow!("热键未初始化"))?;
            
            let recorder = recorder.clone();
            let is_recording = is_recording.clone();
            
            // 按下热键时开始录音
            hotkey.on_press(Arc::new(move || {
                if !*is_recording.lock() {
                    info!("开始录音...");
                    if let Err(e) = recorder.start() {
                        error!("开始录音失败: {}", e);
                    }
                }
            }));

            // 松开热键时停止录音并处理
            hotkey.on_release(Arc::new(move || {
                if *is_recording.lock() {
                    // 停止录音
                    match recorder.stop() {
                        Ok(audio_data) => {
                            if audio_data.is_empty() {
                                info!("录音数据为空，跳过");
                                return;
                            }
                            info!("录音完成，采样点: {}", audio_data.len());

                            // Whisper 转写
                            if let Some(w) = whisper {
                                match w.transcribe(&audio_data) {
                                    Ok(raw_text) => {
                                        info!("转写结果: {}", raw_text);
                                        
                                        // LLM 整理
                                        let final_text = if let Some(l) = llm {
                                            if l.is_enabled() {
                                                match l.rewrite(&raw_text) {
                                                    Ok(clean) => {
                                                        info!("LLM 整理后: {}", clean);
                                                        clean
                                                    }
                                                    Err(e) => {
                                                        error!("LLM 整理失败: {}", e);
                                                        raw_text
                                                    }
                                                }
                                            } else {
                                                raw_text
                                            }
                                        } else {
                                            raw_text
                                        };

                                        // 输入文本
                                        if let Err(e) = keyboard.type_text(&final_text) {
                                            error!("文本输入失败: {}", e);
                                        } else {
                                            info!("文本已输入");
                                        }
                                    }
                                    Err(e) => {
                                        error!("Whisper 转写失败: {}", e);
                                    }
                                }
                            } else {
                                warn!("Whisper 未初始化，无法转写");
                            }
                        }
                        Err(e) => {
                            error!("停止录音失败: {}", e);
                        }
                    }
                }
            }));

            // 启动热键监听
            hotkey.start()?;
        }

        info!("===========================================");
        info!("🎤 TypechoAI 语音输入已启动");
        info!("   按住 {} 说话，松开后自动输入", self.config.hotkey.key);
        info!("   按 Ctrl+C 退出");
        info!("===========================================");

        // 保持运行
        loop {
            std::thread::sleep(Duration::from_secs(1));
        }
    }
}

fn main() -> anyhow::Result<()> {
    // 初始化日志
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into()),
        )
        .init();

    info!("TypechoAI 启动中...");

    let cli = Cli::parse();

    match cli.command {
        Commands::Run => {
            info!("运行语音输入模式");
            let mut app = AppState::new(&cli.config)?;
            app.init_modules()?;
            app.run_voice_input()?;
        }
        Commands::Test => {
            info!("运行测试模式");
            test_modules(&cli.config)?;
        }
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

fn test_modules(config_path: &str) -> Result<()> {
    println!("=== 测试各模块 ===\n");

    // 测试配置
    println!("[1/4] 测试配置模块...");
    let config = config::Config::load(config_path)?;
    println!("  ✓ 配置加载成功");
    println!("    - 热键: {}", config.hotkey.key);
    println!("    - 采样率: {} Hz", config.audio.sample_rate);
    println!("    - LLM 启用: {}", config.llm.enabled);

    // 测试音频
    println!("\n[2/4] 测试音频录制器...");
    match audio::AudioRecorder::new(config.audio.sample_rate, config.audio.channels) {
        Ok(r) => {
            println!("  ✓ 音频录制器创建成功");
            println!("    - 采样率: {} Hz", config.audio.sample_rate);
            println!("    - 声道: {}", config.audio.channels);
        }
        Err(e) => {
            println!("  ✗ 音频录制器创建失败: {}", e);
        }
    }

    // 测试 Whisper
    println!("\n[3/4] 测试 Whisper...");
    match stt::WhisperSTT::new(&config.whisper.model_path) {
        Ok(w) => {
            println!("  ✓ Whisper 模型加载成功");
        }
        Err(e) => {
            println!("  ~ Whisper 模型加载跳过: {}", e);
            println!("    (请下载模型到 models/ 目录)");
        }
    }

    // 测试键盘
    println!("\n[4/4] 测试键盘输入...");
    match input::Keyboard::new() {
        Ok(mut k) => {
            println!("  ✓ 键盘输入初始化成功");
        }
        Err(e) => {
            println!("  ✗ 键盘输入初始化失败: {}", e);
        }
    }

    println!("\n=== 测试完成 ===");
    Ok(())
}
