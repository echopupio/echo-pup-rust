//! 音频缓冲区
#![allow(dead_code)]

use std::collections::VecDeque;

pub struct AudioBuffer {
    samples: Vec<f32>,
}

impl AudioBuffer {
    pub fn new() -> Self {
        Self {
            samples: Vec::new(),
        }
    }

    pub fn push(&mut self, sample: f32) {
        self.samples.push(sample);
    }

    pub fn clear(&mut self) {
        self.samples.clear();
    }

    pub fn as_slice(&self) -> &[f32] {
        &self.samples
    }
}

/// 固定容量音频环形缓冲区。
///
/// 当前阶段先作为后续采集热路径改造的基础设施落地，
/// 后续再逐步接入 `AudioRecorder`。
#[derive(Debug, Clone)]
pub struct AudioRingBuffer {
    capacity: usize,
    samples: VecDeque<f32>,
}

impl AudioRingBuffer {
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            samples: VecDeque::with_capacity(capacity.max(1)),
        }
    }

    pub fn len(&self) -> usize {
        self.samples.len()
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    pub fn clear(&mut self) {
        self.samples.clear();
    }

    pub fn push_sample(&mut self, sample: f32) {
        if self.samples.len() == self.capacity {
            self.samples.pop_front();
        }
        self.samples.push_back(sample);
    }

    pub fn push_samples(&mut self, samples: &[f32]) {
        if samples.len() >= self.capacity {
            self.samples.clear();
            self.samples.extend(
                samples[samples.len() - self.capacity..]
                    .iter()
                    .copied(),
            );
            return;
        }

        let overflow = self
            .samples
            .len()
            .saturating_add(samples.len())
            .saturating_sub(self.capacity);
        for _ in 0..overflow {
            self.samples.pop_front();
        }
        self.samples.extend(samples.iter().copied());
    }

    pub fn snapshot(&self) -> Vec<f32> {
        self.samples.iter().copied().collect()
    }

    pub fn tail(&self, count: usize) -> Vec<f32> {
        if count >= self.samples.len() {
            return self.snapshot();
        }
        self.samples
            .iter()
            .skip(self.samples.len() - count)
            .copied()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::AudioRingBuffer;

    #[test]
    fn push_samples_keeps_latest_capacity_window() {
        let mut buffer = AudioRingBuffer::with_capacity(4);
        buffer.push_samples(&[1.0, 2.0, 3.0]);
        buffer.push_samples(&[4.0, 5.0]);
        assert_eq!(buffer.snapshot(), vec![2.0, 3.0, 4.0, 5.0]);
    }

    #[test]
    fn push_samples_over_capacity_replaces_with_latest_tail() {
        let mut buffer = AudioRingBuffer::with_capacity(3);
        buffer.push_samples(&[1.0, 2.0, 3.0, 4.0, 5.0]);
        assert_eq!(buffer.snapshot(), vec![3.0, 4.0, 5.0]);
    }

    #[test]
    fn tail_returns_latest_requested_samples() {
        let mut buffer = AudioRingBuffer::with_capacity(5);
        buffer.push_samples(&[1.0, 2.0, 3.0, 4.0, 5.0]);
        assert_eq!(buffer.tail(2), vec![4.0, 5.0]);
        assert_eq!(buffer.tail(10), vec![1.0, 2.0, 3.0, 4.0, 5.0]);
    }
}
