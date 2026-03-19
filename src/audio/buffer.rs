//! 音频缓冲区
#![allow(dead_code)]

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
