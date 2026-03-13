//! TypechoAI - AI Voice Dictation Tool
//! 
//! 核心流程: 按住 F12 → 说话 → Whisper 转写 → LLM 整理 → 自动输入

mod audio;
mod config;
mod hotkey;
mod input;
mod llm;
mod stt;

use clap::{Parser, Subcommand};
use tracing::{info, error};

#[derive(Parser)]
#[command(name = "typechoai")]
#[command(about = "AI Voice Dictation Tool", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// 配置文件路径
    #[arg(short, long, default_value = "~/.typechoai/config.toml")]
    config: String,
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
    },
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
            // TODO: 实现运行逻辑
        }
        Commands::Test => {
            info!("运行测试模式");
            // TODO: 实现测试逻辑
        }
        Commands::Config { show } => {
            if show {
                info!("显示当前配置");
                // TODO: 显示配置
            }
        }
    }

    Ok(())
}
