//! 音频录制器
#![allow(dead_code)]

use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::Stream;
use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};
use tracing::info;

/// 降噪强度配置
#[derive(Clone, Debug)]
pub struct DenoiseConfig {
    /// 降噪强度 (0.0-1.0)，0 表示不降噪，1 表示最强降噪
    pub strength: f32,
    /// 滤波器窗口大小（奇数），越大降噪越强但延迟越高
    pub window_size: usize,
}

impl Default for DenoiseConfig {
    fn default() -> Self {
        Self {
            strength: 0.3,  // 默认轻度降噪
            window_size: 5, // 默认 5 点滑动窗口
        }
    }
}

impl DenoiseConfig {
    /// 创建新的降噪配置
    pub fn new(strength: f32, window_size: usize) -> Self {
        let window_size = if window_size % 2 == 0 {
            window_size + 1
        } else {
            window_size
        };
        let strength = strength.clamp(0.0, 1.0);
        Self {
            strength,
            window_size,
        }
    }
}

/// 简单的移动平均降噪滤波器
/// 使用滑动窗口平均来平滑信号，减少高频噪声
pub struct Denoiser {
    config: DenoiseConfig,
    /// 滤波器状态，用于保持连续性
    state: Arc<Mutex<Vec<f32>>>,
}

impl Denoiser {
    /// 创建新的降噪器
    pub fn new(config: DenoiseConfig) -> Self {
        Self {
            config,
            state: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// 使用默认配置创建降噪器
    pub fn default_denoiser() -> Self {
        Self::new(DenoiseConfig::default())
    }

    /// 设置降噪强度
    pub fn set_strength(&mut self, strength: f32) {
        self.config.strength = strength.clamp(0.0, 1.0);
    }

    /// 设置滤波器窗口大小
    pub fn set_window_size(&mut self, window_size: usize) {
        self.config.window_size = if window_size % 2 == 0 {
            window_size + 1
        } else {
            window_size
        };
    }

    /// 对音频数据进行降噪处理
    /// 使用移动平均滤波 + 信号衰减的混合方法
    pub fn denoise(&self, audio: &[f32]) -> Vec<f32> {
        if self.config.strength == 0.0 || audio.is_empty() {
            return audio.to_vec();
        }

        let window_size = self.config.window_size.max(1);
        let strength = self.config.strength;

        let mut result = Vec::with_capacity(audio.len());
        let mut state = self.state.lock();

        // 保持状态连续性：将之前的状态附加到当前输入之前
        let mut input_with_state = state.clone();
        input_with_state.extend_from_slice(audio);

        for i in 0..audio.len() {
            let idx = i + state.len();

            // 计算滑动窗口平均
            let half_window = window_size / 2;
            let start = idx.saturating_sub(half_window);
            let end = (idx + half_window + 1).min(input_with_state.len());

            let window: Vec<f32> = input_with_state[start..end].to_vec();
            let avg: f32 = window.iter().sum::<f32>() / window.len() as f32;

            // 混合原始信号和平均信号
            // 强度越高，越多地使用平均信号（降噪后的信号）
            let original = audio[i];
            let denoised = original * (1.0 - strength * 0.5) + avg * (strength * 0.5);

            result.push(denoised);

            // 更新状态，保留最近的样本
            if i + window_size >= state.len() + audio.len() {
                // 到达输入末尾，保存最后 window_size 个样本到状态
                let state_start = audio.len().saturating_sub(window_size);
                *state = audio[state_start..].to_vec();
            }
        }

        // 如果音频太短，没有更新状态，则清空
        if audio.len() < window_size {
            *state = audio
                .iter()
                .rev()
                .take(window_size)
                .cloned()
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
        }

        result
    }

    /// 重置降噪器状态
    pub fn reset(&self) {
        self.state.lock().clear();
    }
}

/// 音频录制器 - 线程安全
pub struct AudioRecorder {
    sample_rate: u32,
    channels: u16,
    is_recording: Arc<AtomicBool>,
    audio_buffer: Arc<Mutex<Vec<f32>>>,
    captured_samples: Arc<AtomicU64>,
    recording_started_at: Arc<Mutex<Option<Instant>>>,
    stream: Arc<Mutex<Option<Stream>>>,
    device_sample_rate: Arc<Mutex<u32>>, // 设备的实际采样率

    // 增益控制
    gain: Arc<Mutex<f32>>, // 增益系数（默认 1.0，可配置 1.5-3.0）
    max_gain: f32,         // 最大增益，防止爆音

    // 降噪相关
    denoiser: Arc<Mutex<Denoiser>>,
    denoise_enabled: Arc<AtomicBool>,

    // 端点检测（VAD）相关
    vad_enabled: Arc<AtomicBool>,
    vad_threshold: Arc<Mutex<f32>>,
    vad_silence_duration_ms: Arc<AtomicU64>, // 持续静音多少毫秒后自动停止
    vad_callback: Arc<Mutex<Option<Box<dyn Fn() + Send + Sync>>>>,
    vad_thread_handle: Arc<Mutex<Option<thread::JoinHandle<()>>>>,
}

// 确保 AudioRecorder 可以安全地在线程间共享
unsafe impl Send for AudioRecorder {}
unsafe impl Sync for AudioRecorder {}

/// 将交错多声道 PCM downmix 为单声道
fn downmix_to_mono(audio: &[f32], channels: usize) -> Vec<f32> {
    if channels <= 1 {
        return audio.to_vec();
    }

    let mut mono = Vec::with_capacity(audio.len() / channels);
    for frame in audio.chunks_exact(channels) {
        let sum: f32 = frame.iter().copied().sum();
        mono.push(sum / channels as f32);
    }
    mono
}

/// 重采样音频
/// 为保证稳定性，统一使用线性插值（避免非标准插值导致音素失真）
fn resample_audio(audio: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
    if from_rate == to_rate {
        return audio.to_vec();
    }
    resample_audio_linear(audio, from_rate, to_rate)
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
            captured_samples: Arc::new(AtomicU64::new(0)),
            recording_started_at: Arc::new(Mutex::new(None)),
            stream: Arc::new(Mutex::new(None)),
            device_sample_rate: Arc::new(Mutex::new(0)),
            gain: Arc::new(Mutex::new(1.0)), // 默认不增益
            max_gain: 3.0,                   // 最大增益 3.0，防止爆音
            denoiser: Arc::new(Mutex::new(Denoiser::default_denoiser())),
            denoise_enabled: Arc::new(AtomicBool::new(false)),
            vad_enabled: Arc::new(AtomicBool::new(false)),
            vad_threshold: Arc::new(Mutex::new(0.01)),
            vad_silence_duration_ms: Arc::new(AtomicU64::new(1500)), // 默认 1.5 秒
            vad_callback: Arc::new(Mutex::new(None)),
            vad_thread_handle: Arc::new(Mutex::new(None)),
        })
    }

    /// 设置麦克风增益系数
    /// - gain: 增益系数，范围 1.0-3.0（默认 1.0）
    ///   - 1.0: 不增益
    ///   - 1.5-2.0: 轻度增益，适合一般场景
    ///   - 2.0-3.0: 强力增益，适合麦克风音量很小的场景
    pub fn set_gain(&self, gain: f32) {
        let gain = gain.clamp(1.0, self.max_gain);
        *self.gain.lock() = gain;
        info!("麦克风增益设置为: {}", gain);
    }

    /// 获取当前增益系数
    pub fn get_gain(&self) -> f32 {
        *self.gain.lock()
    }

    /// 设置降噪参数
    /// - strength: 降噪强度，范围 0.0-1.0（默认 0.3）
    ///   - 0.0: 不降噪
    ///   - 0.1-0.3: 轻度降噪，适合一般环境
    ///   - 0.4-0.7: 中度降噪，适合有背景噪音的环境
    ///   - 0.8-1.0: 强力降噪，可能会影响语音质量
    /// - window_size: 滤波器窗口大小，越大降噪越强但延迟越高（默认 5）
    pub fn set_denoise_params(&self, strength: f32, window_size: usize) {
        let mut denoiser = self.denoiser.lock();
        denoiser.set_strength(strength);
        denoiser.set_window_size(window_size);
        info!(
            "降噪参数设置: strength={}, window_size={}",
            strength, denoiser.config.window_size
        );
    }

    /// 启用降噪
    pub fn enable_denoise(&self) {
        self.denoise_enabled.store(true, Ordering::SeqCst);
        info!("降噪已启用");
    }

    /// 禁用降噪
    pub fn disable_denoise(&self) {
        self.denoise_enabled.store(false, Ordering::SeqCst);
        info!("降噪已禁用");
    }

    /// 检查降噪是否启用
    pub fn is_denoise_enabled(&self) -> bool {
        self.denoise_enabled.load(Ordering::SeqCst)
    }

    /// 重置降噪器状态（用于新的录音会话）
    pub fn reset_denoiser(&self) {
        self.denoiser.lock().reset();
    }

    /// 设置端点检测参数
    /// - silence_duration_ms: 持续静音多少毫秒后自动停止录音（默认 1500ms）
    /// - threshold: 能量阈值，低于此值认为是静音（默认 0.01）
    pub fn set_vad_params(&self, silence_duration_ms: u64, threshold: f32) {
        *self.vad_threshold.lock() = threshold;
        self.vad_silence_duration_ms
            .store(silence_duration_ms, Ordering::SeqCst);
    }

    /// 设置端点检测回调 - 当检测到语音结束时自动调用
    pub fn set_vad_callback<F>(&self, callback: F)
    where
        F: Fn() + Send + Sync + 'static,
    {
        *self.vad_callback.lock() = Some(Box::new(callback));
    }

    /// 启用端点检测（自动结束录音）
    pub fn enable_vad(&self) {
        self.vad_enabled.store(true, Ordering::SeqCst);
    }

    /// 禁用端点检测
    pub fn disable_vad(&self) {
        self.vad_enabled.store(false, Ordering::SeqCst);
    }

    /// 检查端点检测是否已触发（语音已结束）
    pub fn is_vad_triggered(&self) -> bool {
        // 检查回调是否存在，如果存在说明 VAD 已触发
        let callback = self.vad_callback.lock();
        callback.is_some()
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
        let device_name = device
            .name()
            .unwrap_or_else(|_| "unknown-input-device".to_string());

        let config = device.default_input_config().context("无法获取音频配置")?;
        let sample_format = config.sample_format();

        let _sample_rate = self.sample_rate;
        let is_recording = self.is_recording.clone();
        let audio_buffer = self.audio_buffer.clone();
        let captured_samples = self.captured_samples.clone();

        let config_clone = config.clone();
        let input_channels = config.channels() as usize;

        is_recording.store(true, Ordering::SeqCst);
        audio_buffer.lock().clear();
        captured_samples.store(0, Ordering::SeqCst);
        *self.recording_started_at.lock() = Some(Instant::now());

        info!(
            "开始录音: 设备={}, 设备采样率={}Hz, 声道={}, 格式={:?}",
            device_name,
            config.sample_rate().0,
            config.channels(),
            sample_format
        );

        let err_fn = |err| eprintln!("音频流错误: {}", err);

        // 直接使用设备原始采样率，不做降采样
        let stream = match sample_format {
            cpal::SampleFormat::F32 => {
                let audio_buffer = audio_buffer.clone();
                let is_recording = is_recording.clone();
                let captured_samples = captured_samples.clone();
                let gain = self.gain.clone();
                let denoiser = self.denoiser.clone();
                let denoise_enabled = self.denoise_enabled.clone();
                device.build_input_stream(
                    &config_clone.into(),
                    move |data: &[f32], _: &cpal::InputCallbackInfo| {
                        if is_recording.load(Ordering::SeqCst) {
                            let mut buffer = audio_buffer.lock();
                            let gain_val = *gain.lock();

                            // 先 downmix 到单声道，避免交错多声道数据直接送入 ASR
                            let mono = downmix_to_mono(data, input_channels);

                            // 应用增益并限制最大值防止爆音
                            let mut processed: Vec<f32> = mono
                                .iter()
                                .map(|&sample| (sample * gain_val).clamp(-1.0, 1.0))
                                .collect();

                            // 应用降噪（如果启用）
                            if denoise_enabled.load(Ordering::SeqCst) {
                                let denoiser = denoiser.lock();
                                processed = denoiser.denoise(&processed);
                            }

                            captured_samples.fetch_add(processed.len() as u64, Ordering::SeqCst);
                            buffer.extend(processed);
                        }
                    },
                    err_fn,
                    None,
                )?
            }
            cpal::SampleFormat::I16 => {
                let audio_buffer = audio_buffer.clone();
                let is_recording = is_recording.clone();
                let captured_samples = captured_samples.clone();
                let gain = self.gain.clone();
                let denoiser = self.denoiser.clone();
                let denoise_enabled = self.denoise_enabled.clone();
                device.build_input_stream(
                    &config_clone.into(),
                    move |data: &[i16], _: &cpal::InputCallbackInfo| {
                        if is_recording.load(Ordering::SeqCst) {
                            let mut buffer = audio_buffer.lock();
                            let gain_val = *gain.lock();

                            // 先转为 f32 并 downmix 到单声道
                            let interleaved_f32: Vec<f32> =
                                data.iter().map(|&sample| sample as f32 / 32768.0).collect();
                            let mono = downmix_to_mono(&interleaved_f32, input_channels);

                            // 应用增益并限制最大值防止爆音
                            let mut processed: Vec<f32> = mono
                                .iter()
                                .map(|&sample| (sample * gain_val).clamp(-1.0, 1.0))
                                .collect();

                            // 应用降噪（如果启用）
                            if denoise_enabled.load(Ordering::SeqCst) {
                                let denoiser = denoiser.lock();
                                processed = denoiser.denoise(&processed);
                            }

                            captured_samples.fetch_add(processed.len() as u64, Ordering::SeqCst);
                            buffer.extend(processed);
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

        // 重置降噪器状态，确保新录音从头开始
        self.reset_denoiser();

        // 启动端点检测线程（如果启用）
        self.start_vad_thread();

        Ok(())
    }

    /// 启动 VAD 检测线程
    fn start_vad_thread(&self) {
        if !self.vad_enabled.load(Ordering::SeqCst) {
            return;
        }

        let is_recording = self.is_recording.clone();
        let audio_buffer = self.audio_buffer.clone();
        let vad_enabled = self.vad_enabled.clone();
        let vad_threshold = self.vad_threshold.clone();
        let vad_silence_duration_ms = self.vad_silence_duration_ms.clone();
        let vad_callback = self.vad_callback.clone();
        let device_sample_rate = self.device_sample_rate.clone();

        let handle = thread::spawn(move || {
            let mut silence_start: Option<Instant> = None;
            let check_interval = Duration::from_millis(100); // 每 100ms 检查一次

            while is_recording.load(Ordering::SeqCst) {
                thread::sleep(check_interval);

                if !vad_enabled.load(Ordering::SeqCst) {
                    continue;
                }

                // 计算最近一小段音频的能量
                let buffer = audio_buffer.lock();
                let threshold = *vad_threshold.lock();

                // 获取最近 200ms 的音频进行能量计算
                let sample_rate = *device_sample_rate.lock();
                let lookback_samples = (sample_rate as usize * 200) / 1000; // 200ms
                let start_idx = buffer.len().saturating_sub(lookback_samples);
                let recent_audio = &buffer[start_idx..];

                if recent_audio.is_empty() {
                    continue;
                }

                // 计算 RMS 能量
                let sum: f32 = recent_audio.iter().map(|&s| s * s).sum();
                let energy = (sum / recent_audio.len() as f32).sqrt();

                if energy > threshold {
                    // 有声音，重置静音计时
                    silence_start = None;
                } else {
                    // 静音
                    if silence_start.is_none() {
                        silence_start = Some(Instant::now());
                    } else if let Some(start) = silence_start {
                        let silence_duration = start.elapsed().as_millis() as u64;
                        if silence_duration >= vad_silence_duration_ms.load(Ordering::SeqCst) {
                            // 持续静音达到阈值，触发 VAD 回调
                            info!(
                                "端点检测：检测到 {} ms 静音，自动结束录音",
                                silence_duration
                            );

                            // 调用回调
                            if let Some(ref callback) = *vad_callback.lock() {
                                callback();
                            } else {
                                // 无回调时兜底停止
                                is_recording.store(false, Ordering::SeqCst);
                            }
                            break;
                        }
                    }
                }
            }
        });

        *self.vad_thread_handle.lock() = Some(handle);
    }

    /// 停止录音并返回音频数据（已重采样到目标采样率）
    pub fn stop(&self) -> Result<Vec<f32>> {
        if !self.is_recording.load(Ordering::SeqCst) {
            return Ok(Vec::new());
        }

        self.is_recording.store(false, Ordering::SeqCst);
        let elapsed_ms = self
            .recording_started_at
            .lock()
            .take()
            .map(|t| t.elapsed().as_millis() as u64)
            .unwrap_or(0);

        // 停止 VAD 线程
        if let Some(handle) = self.vad_thread_handle.lock().take() {
            if handle.thread().id() != thread::current().id() {
                let _ = handle.join();
            }
        }

        *self.stream.lock() = None;

        let mut buffer = self.audio_buffer.lock().clone();

        // 重采样到目标采样率（Whisper 需要 16000 Hz）
        let device_rate = *self.device_sample_rate.lock();
        if device_rate != 0 && device_rate != self.sample_rate {
            buffer = resample_audio(&buffer, device_rate, self.sample_rate);
        }

        let captured_samples = self.captured_samples.load(Ordering::SeqCst);
        info!(
            "停止录音: 时长={}ms, 捕获采样点={}, 重采样后采样点={}",
            elapsed_ms,
            captured_samples,
            buffer.len()
        );

        if buffer.is_empty() && elapsed_ms >= 300 {
            return Err(anyhow::anyhow!(
                "录音 {} ms 但未收到麦克风数据，请检查系统麦克风权限和输入设备",
                elapsed_ms
            ));
        }

        Ok(buffer)
    }

    /// 获取当前录音快照（已重采样到目标采样率）
    pub fn get_snapshot(&self) -> Vec<f32> {
        let mut buffer = self.audio_buffer.lock().clone();
        let device_rate = *self.device_sample_rate.lock();
        if device_rate != 0 && device_rate != self.sample_rate {
            buffer = resample_audio(&buffer, device_rate, self.sample_rate);
        }
        buffer
    }

    /// 获取目标采样率
    pub fn target_sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// 获取音频缓冲区的副本（用于实时分析）
    pub fn get_audio_buffer(&self) -> Vec<f32> {
        self.get_snapshot()
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
