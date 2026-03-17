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
    device_sample_rate: Arc<Mutex<u32>>,  // 设备的实际采样率
}

// 确保 AudioRecorder 可以安全地在线程间共享
unsafe impl Send for AudioRecorder {}
unsafe impl Sync for AudioRecorder {}

/// 重采样音频到目标采样率（简单线性插值）
fn resample_audio(audio: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
    if from_rate == to_rate {
        return audio.to_vec();
    }
    
    let ratio = to_rate as f64 / from_rate as f64;
    let new_len = (audio.len() as f64 * ratio) as usize;
    let mut result = Vec::with_capacity(new_len);
    
    for i in 0..new_len {
        let src_idx = i as f64 / ratio;
        let idx = src_idx as usize;
        let frac = (src_idx - idx as f64) as f32;
        
        if idx + 1 < audio.len() {
            // 线性插值
            let sample = audio[idx] * (1.0 - frac) + audio[idx + 1] * frac;
            result.push(sample);
        } else if idx < audio.len() {
            result.push(audio[idx]);
        }
    }
    
    result
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
            device_sample_rate: Arc::new(Mutex::new(0)),
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

        // 保存设备实际采样率
        *self.device_sample_rate.lock() = config.sample_rate().0;
        tracing::info!("[Recorder] 设备采样率: {} Hz, 目标采样率: {} Hz", 
            config.sample_rate().0, self.sample_rate);

        tracing::info!("[Recorder] 录音已开始");
        Ok(())
    }

    /// 停止录音并返回音频数据（已重采样到目标采样率）
    pub fn stop(&self) -> Result<Vec<f32>> {
        if !self.is_recording.load(Ordering::SeqCst) {
            tracing::warn!("[Recorder] 未在录音中");
            return Ok(Vec::new());
        }

        tracing::info!("[Recorder] 停止录音...");
        self.is_recording.store(false, Ordering::SeqCst);
        *self.stream.lock() = None;

        let mut buffer = self.audio_buffer.lock().clone();
        
        // 重采样到目标采样率（Whisper 需要 16000 Hz）
        let device_rate = *self.device_sample_rate.lock();
        if device_rate != 0 && device_rate != self.sample_rate {
            tracing::info!("[Recorder] 重采样: {} Hz -> {} Hz", device_rate, self.sample_rate);
            buffer = resample_audio(&buffer, device_rate, self.sample_rate);
        }
        
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
