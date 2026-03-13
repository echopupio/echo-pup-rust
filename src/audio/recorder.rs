//! 音频录制器

use anyhow::Result;

pub struct AudioRecorder;

impl AudioRecorder {
    pub fn new() -> Self {
        Self
    }

    pub fn start(&self) -> Result<()> {
        todo!()
    }

    pub fn stop(&self) -> Result<Vec<f32>> {
        todo!()
    }
}
