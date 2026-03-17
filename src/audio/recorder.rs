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

/// 使用立方样条插值重采样音频（比线性插值更好的质量）
fn resample_audio(audio: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
    if from_rate == to_rate {
        return audio.to_vec();
    }

    let ratio = to_rate as f64 / from_rate as f64;
    let new_len = (audio.len() as f64 * ratio) as usize;
    
    if new_len == 0 || audio.len() < 4 {
        return resample_audio_linear(audio, from_rate, to_rate);
    }

    let mut result = Vec::with_capacity(new_len);
    
    // 预计算二阶差分用于立方插值
    let n = audio.len();
    let mut y2 = vec![0.0f32; n];
    let mut y = audio.to_vec();
    
    // 设定边界条件：自然样条（假设二阶导数为零）
    let mut u = vec![0.0f32; n - 1];
    y2[0] = 0.0;
    y2[n - 1] = 0.0;
    u[0] = 0.0;
    
    for i in 1..(n - 1) {
        let sig = (y[i + 1] - y[i - 1]) / (2.0 * (y[i] - y[i + 1]).abs().max(1e-10));
        let p = sig * y2[i - 1] + 2.0;
        y2[i] = (sig - 1.0) / p.max(1e-10);
        u[i] = (y[i + 1] - y[i]) / ((y[i + 1] - y[i]).abs().max(1e-10)) 
            - (y[i] - y[i - 1]) / ((y[i] - y[i - 1]).abs().max(1e-10));
        u[i] = (6.0 * u[i] / (2.0 * ((y[i + 1] - y[i]).abs().max(1e-10) + (y[i] - y[i - 1]).abs().max(1e-10))) - sig * y2[i - 1]) / p.max(1e-10);
    }
    
    // 反向扫描以得到正确的样条系数
    for i in (1..n - 1).rev() {
        y2[i] = y2[i] * y2[i + 1] + u[i];
    }

    // 进行立方插值
    for i in 0..new_len {
        let src_idx = i as f64 / ratio;
        let idx = src_idx as usize;
        let frac = (src_idx - idx as f64) as f32;
        
        if idx + 1 < n {
            // 立方样条插值
            let h0 = (1.0 - frac).powi(3) / 6.0;
            let h1 = (3.0 * frac.powi(3) - 6.0 * frac.powi(2) + 4.0) / 6.0;
            let h2 = (-3.0 * frac.powi(3) + 3.0 * frac.powi(2) + 3.0 * frac + 1.0) / 6.0;
            let h3 = frac.powi(3) / 6.0;
            
            let idx_prev = if idx > 0 { idx - 1 } else { 0 };
            let idx_next = if idx + 1 < n { idx + 1 } else { n - 1 };
            let idx_next2 = if idx + 2 < n { idx + 2 } else { n - 1 };
            
            let sample = h0 * y[idx_prev] + h1 * y[idx] + h2 * y[idx_next] + h3 * y[idx_next2];
            result.push(sample);
        } else if idx < n {
            result.push(y[idx]);
        }
    }
    
    result
}

/// 简单的线性插值重采样（作为备选）
fn resample_audio_linear(audio: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
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

        Ok(())
    }

    /// 停止录音并返回音频数据（已重采样到目标采样率）
    pub fn stop(&self) -> Result<Vec<f32>> {
        if !self.is_recording.load(Ordering::SeqCst) {
            return Ok(Vec::new());
        }

        self.is_recording.store(false, Ordering::SeqCst);
        *self.stream.lock() = None;

        let mut buffer = self.audio_buffer.lock().clone();
        
        // 重采样到目标采样率（Whisper 需要 16000 Hz）
        let device_rate = *self.device_sample_rate.lock();
        if device_rate != 0 && device_rate != self.sample_rate {
            buffer = resample_audio(&buffer, device_rate, self.sample_rate);
        }

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
