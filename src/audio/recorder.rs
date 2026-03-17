//! 音频录制器

use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::Stream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use parking_lot::Mutex;

/// 音频录制器 - 线程安全
pub struct AudioRecorder {
    sample_rate: u32,
    channels: u16,
    is_recording: Arc<AtomicBool>,
    audio_buffer: Arc<Mutex<Vec<f32>>>,
    stream: Arc<Mutex<Option<Stream>>>,
}

// 确保 AudioRecorder 可以安全地在线程间共享
unsafe impl Send for AudioRecorder {}
unsafe impl Sync for AudioRecorder {}

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
            tracing::warn!("[Recorder] 已经在录音中，跳过");
            return Ok(());
        }

        tracing::info!("[Recorder] 开始录音...");

        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .context("无法获取音频输入设备")?;

        tracing::debug!("[Recorder] 音频设备: {:?}", device.name());

        let config = device
            .default_input_config()
            .context("无法获取音频配置")?;

        tracing::debug!("[Recorder] 音频配置: {:?}", config);

        let sample_rate = self.sample_rate;
        let is_recording = self.is_recording.clone();
        let audio_buffer = self.audio_buffer.clone();

        let config_clone = config.clone();
        let config_sample_rate = config.sample_rate();

        is_recording.store(true, Ordering::SeqCst);
        audio_buffer.lock().clear();

        let err_fn = |err| eprintln!("音频流错误: {}", err);

        // 直接使用设备原始采样率，不做降采样
        let stream = match config.sample_format() {
            cpal::SampleFormat::F32 => {
                let audio_buffer = audio_buffer.clone();
                let is_recording = is_recording.clone();
                device.build_input_stream(
                    &config_clone.into(),
                    move |data: &[f32], _: &cpal::InputCallbackInfo| {
                        if is_recording.load(Ordering::SeqCst) {
                            let mut buffer = audio_buffer.lock();
                            // 直接复制原始数据，不做降采样
                            buffer.extend_from_slice(data);
                        }
                    },
                    err_fn,
                    None,
                )?
            }
            cpal::SampleFormat::I16 => {
                let audio_buffer = audio_buffer.clone();
                let is_recording = is_recording.clone();
                device.build_input_stream(
                    &config_clone.into(),
                    move |data: &[i16], _: &cpal::InputCallbackInfo| {
                        if is_recording.load(Ordering::SeqCst) {
                            let mut buffer = audio_buffer.lock();
                            // 转换为 f32
                            for &sample in data.iter() {
                                buffer.push(sample as f32 / 32768.0);
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
        *self.stream.lock() = Some(stream);

        tracing::info!("[Recorder] 录音已开始");
        Ok(())
    }

    /// 停止录音并返回音频数据
    pub fn stop(&self) -> Result<Vec<f32>> {
        if !self.is_recording.load(Ordering::SeqCst) {
            tracing::warn!("[Recorder] 未在录音中");
            return Ok(Vec::new());
        }

        tracing::info!("[Recorder] 停止录音...");
        self.is_recording.store(false, Ordering::SeqCst);
        *self.stream.lock() = None;

        let buffer = self.audio_buffer.lock().clone();
        tracing::info!("[Recorder] 录音已停止，采样点数: {}", buffer.len());

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
