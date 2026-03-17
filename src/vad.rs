//! VAD (Voice Activity Detection) 模块
//! 使用能量检测算法识别音频中的语音段
//!
//! 这种方法通过计算音频的短时能量来检测语音：
//! - 能量高于阈值的部分认为是语音
//! - 能量低于阈值的部分认为是静音
//!
//! 优点：无需外部依赖，计算简单，速度快

/// VAD 配置参数
#[derive(Debug, Clone)]
pub struct VadConfig {
    /// 采样率 (默认 16000 Hz)
    pub sample_rate: u32,
    /// 帧大小（采样点数，默认 400 = 25ms @ 16kHz）
    pub frame_size: usize,
    /// 帧移（采样点数，默认 160 = 10ms @ 16kHz）
    pub frame_shift: usize,
    /// 能量阈值 (默认 0.01)
    pub threshold: f32,
    /// 最小语音持续帧数（用于过滤短暂噪声）
    pub min_speech_frames: usize,
    /// 静音前缀帧数（从静音开始计算）
    pub min_silence_frames: usize,
}

impl Default for VadConfig {
    fn default() -> Self {
        Self {
            sample_rate: 16000,
            frame_size: 400,    // 25ms 帧
            frame_shift: 160,   // 10ms 帧移
            threshold: 0.01,    // 能量阈值
            min_speech_frames: 3,   // 至少 30ms 的语音
            min_silence_frames: 15, // 150ms 静音才认为语音结束
        }
    }
}

/// VAD 语音活动检测器（基于能量）
pub struct VadDetector {
    config: VadConfig,
}

impl VadDetector {
    /// 创建新的 VAD 检测器（使用默认配置）
    pub fn new() -> Self {
        Self {
            config: VadConfig::default(),
        }
    }

    /// 创建指定采样率的 VAD 检测器
    pub fn with_sample_rate(sample_rate: u32) -> Self {
        let mut config = VadConfig::default();
        config.sample_rate = sample_rate;
        // 根据采样率调整帧大小
        config.frame_size = (sample_rate as f32 * 0.025) as usize;
        config.frame_shift = (sample_rate as f32 * 0.010) as usize;
        Self { config }
    }

    /// 创建自定义配置的 VAD 检测器
    pub fn with_config(config: VadConfig) -> Self {
        Self { config }
    }

    /// 设置能量阈值
    pub fn set_threshold(&mut self, threshold: f32) {
        self.config.threshold = threshold;
    }

    /// 计算音频的短时能量
    fn compute_frame_energy(&self, audio: &[f32], frame_start: usize) -> f32 {
        let end = (frame_start + self.config.frame_size).min(audio.len());
        if frame_start >= end {
            return 0.0;
        }

        let frame = &audio[frame_start..end];
        // 计算均方根 (RMS)
        let sum: f32 = frame.iter().map(|&s| s * s).sum();
        (sum / frame.len() as f32).sqrt()
    }

    /// 检测音频中的语音段
    /// 返回语音段的起止位置（采样点索引）
    pub fn detect_speech_segments(&self, audio: &[f32]) -> Vec<(usize, usize)> {
        if audio.is_empty() {
            return Vec::new();
        }

        let num_frames = (audio.len().saturating_sub(self.config.frame_size))
            / self.config.frame_shift
            + 1;

        if num_frames == 0 {
            return Vec::new();
        }

        // 标记每帧是否为语音
        let mut speech_frames: Vec<bool> = Vec::with_capacity(num_frames);

        for i in 0..num_frames {
            let frame_start = i * self.config.frame_shift;
            let energy = self.compute_frame_energy(audio, frame_start);
            speech_frames.push(energy > self.config.threshold);
        }

        // 合并连续的语音帧
        let mut segments: Vec<(usize, usize)> = Vec::new();
        let mut in_speech = false;
        let mut speech_start = 0;
        let mut silence_count = 0;

        for (i, &is_speech) in speech_frames.iter().enumerate() {
            if is_speech {
                if !in_speech {
                    // 开始新的语音段
                    speech_start = i;
                    in_speech = true;
                    silence_count = 0;
                }
            } else {
                if in_speech {
                    silence_count += 1;
                    // 连续静音达到阈值，结束当前语音段
                    if silence_count >= self.config.min_silence_frames {
                        let start_sample = speech_start * self.config.frame_shift;
                        let end_sample = ((i - self.config.min_silence_frames)
                            * self.config.frame_shift
                            + self.config.frame_size)
                            .min(audio.len());

                        // 检查是否满足最小语音长度
                        if end_sample - start_sample
                            >= self.config.min_speech_frames * self.config.frame_shift
                        {
                            segments.push((start_sample, end_sample));
                        }
                        in_speech = false;
                    }
                }
            }
        }

        // 处理最后一个语音段
        if in_speech {
            let start_sample = speech_start * self.config.frame_shift;
            let end_sample = audio.len();
            if end_sample - start_sample
                >= self.config.min_speech_frames * self.config.frame_shift
            {
                segments.push((start_sample, end_sample));
            }
        }

        segments
    }

    /// 检测音频是否包含语音
    pub fn contains_speech(&self, audio: &[f32]) -> bool {
        let segments = self.detect_speech_segments(audio);
        !segments.is_empty()
    }

    /// 过滤音频，只保留有语音的部分
    pub fn filter_speech(&self, audio: &[f32]) -> Vec<f32> {
        let segments = self.detect_speech_segments(audio);

        if segments.is_empty() {
            return Vec::new();
        }

        let mut result = Vec::new();
        for (start, end) in segments {
            // 添加一小段静音作为间隔（可选）
            if !result.is_empty() {
                let silence_len = (self.config.sample_rate as f32 * 0.1) as usize;
                result.extend(vec![0.0f32; silence_len.min(self.config.frame_shift)]);
            }
            result.extend_from_slice(&audio[start..end]);
        }

        result
    }

    /// 获取音频中最长的语音段
    pub fn get_longest_segment(&self, audio: &[f32]) -> Option<(usize, usize)> {
        let segments = self.detect_speech_segments(audio);
        segments.into_iter().max_by_key(|(start, end)| end - start)
    }

    /// 获取音频的平均能量
    pub fn get_average_energy(&self, audio: &[f32]) -> f32 {
        if audio.is_empty() {
            return 0.0;
        }
        let sum: f32 = audio.iter().map(|&s| s * s).sum();
        (sum / audio.len() as f32).sqrt()
    }

    /// 自动调整阈值（基于音频最大能量）
    pub fn auto_threshold(&mut self, audio: &[f32]) {
        if audio.is_empty() {
            return;
        }

        // 计算最大能量
        let max_energy = audio
            .iter()
            .map(|&s| s * s)
            .fold(0.0_f32, |a, b| a.max(b));

        if max_energy > 0.0 {
            // 阈值设为最大能量的 5%
            self.config.threshold = max_energy * 0.05;
        }
    }
}

impl Default for VadDetector {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vad_empty_audio() {
        let vad = VadDetector::new();
        let segments = vad.detect_speech_segments(&[]);
        assert!(segments.is_empty());
    }

    #[test]
    fn test_vad_silence_only() {
        let vad = VadDetector::new();
        // 1秒的静音 (16kHz)
        let silence = vec![0.0f32; 16000];
        let segments = vad.detect_speech_segments(&silence);
        assert!(segments.is_empty());
    }

    #[test]
    fn test_vad_with_speech() {
        let mut vad = VadDetector::new();
        // 生成测试信号：0.1秒静音 + 0.2秒语音 + 0.1秒静音
        let mut audio = Vec::new();
        audio.extend(vec![0.0f32; 1600]); // 0.1秒静音
        // 0.2秒语音 (正弦波)
        for i in 0..3200 {
            let t = i as f32 / 16000.0;
            audio.push((2.0 * std::f32::consts::PI * 440.0 * t).sin() * 0.5);
        }
        audio.extend(vec![0.0f32; 1600]); // 0.1秒静音

        // 自动调整阈值
        vad.auto_threshold(&audio);

        let segments = vad.detect_speech_segments(&audio);
        // 应该检测到语音
        assert!(!segments.is_empty());
    }

    #[test]
    fn test_filter_speech() {
        let mut vad = VadDetector::new();
        let mut audio = Vec::new();
        audio.extend(vec![0.0f32; 1600]);
        audio.extend(vec![0.5f32; 3200]); // 语音段
        audio.extend(vec![0.0f32; 1600]);

        vad.auto_threshold(&audio);
        let filtered = vad.filter_speech(&audio);
        assert!(!filtered.is_empty());
        assert!(filtered.len() < audio.len());
    }
}
