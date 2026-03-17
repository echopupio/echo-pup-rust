//! 音频录制器

use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::Stream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use parking_lot::Mutex;

/// 音频录制器
pub struct AudioRecorder {
    sample_rate: u32,
    channels: u16,
    is_recording: Arc<AtomicBool>,
    audio_buffer: Arc<Mutex<Vec<f32>>>,
    // 使用 Arc 存储 stream 以正确管理其生命周期
    stream: Arc<Mutex<Option<Stream>>>,
}

impl AudioRecorder {
    /// 创建新的录音器
    pub fn new(sample_rate: u32, channels: u16) -> Result<Self> {
        Ok(Self {
            sample_rate,
            channels,
            is_recording: Arc::new(AtomicBool::new(false)),
            audio_buffer: Arc::new(Mutex::new(Vec::new())),
            stream: Arc::new(Mutex::new(None)),
        })
    }

    /// 开始录音
    pub fn start(&self) -> Result<()> {
        if self.is_recording.load(Ordering::SeqCst) {
            return Ok(());
        }

        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .context("无法获取音频输入设备")?;

        let config = device
            .default_input_config()
            .context("无法获取音频配置")?;

        let sample_rate = self.sample_rate;
        let is_recording = self.is_recording.clone();
        let audio_buffer = self.audio_buffer.clone();

        // 克隆 config 以便在闭包中使用
        let config_clone = config.clone();
        let config_sample_rate = config.sample_rate();

        is_recording.store(true, Ordering::SeqCst);
        audio_buffer.lock().clear();

        let err_fn = |err| eprintln!("音频流错误: {}", err);

        let stream = match config.sample_format() {
            cpal::SampleFormat::F32 => {
                let audio_buffer = audio_buffer.clone();
                let is_recording = is_recording.clone();
                device.build_input_stream(
                    &config_clone.into(),
                    move |data: &[f32], _: &cpal::InputCallbackInfo| {
                        if is_recording.load(Ordering::SeqCst) {
                            let mut buffer = audio_buffer.lock();
                            // 降采样处理
                            let ratio = config_sample_rate.0 as f32 / sample_rate as f32;
                            if ratio > 1.0 {
                                for (i, &sample) in data.iter().enumerate() {
                                    if i % ratio as usize == 0 {
                                        buffer.push(sample);
                                    }
                                }
                            } else {
                                buffer.extend_from_slice(data);
                            }
                        }
                    },
                    err_fn,
                    None,
                )?
            }
            _ => {
                is_recording.store(false, Ordering::SeqCst);
                return Err(anyhow::anyhow!("不支持的音频格式"));
            }
        };

        stream.play()?;

        // 保存 stream 以便后续停止
        *self.stream.lock() = Some(stream);

        tracing::info!("录音已开始");
        Ok(())
    }

    /// 停止录音并返回音频数据
    pub fn stop(&self) -> Result<Vec<f32>> {
        if !self.is_recording.load(Ordering::SeqCst) {
            return Ok(Vec::new());
        }

        self.is_recording.store(false, Ordering::SeqCst);

        // 清理 stream - 将其置为 None 以释放资源
        *self.stream.lock() = None;

        let buffer = self.audio_buffer.lock().clone();
        tracing::info!("录音已停止，采样点数: {}", buffer.len());

        Ok(buffer)
    }

    /// 检查是否正在录音
    pub fn is_recording(&self) -> bool {
        self.is_recording.load(Ordering::SeqCst)
    }
}

impl Default for AudioRecorder {
    fn default() -> Self {
        Self::new(16000, 1).unwrap()
    }
}
