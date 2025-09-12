//! Shared types and utilities for strim audio streaming

use anyhow::Result;

/// Audio stream configuration
#[derive(Debug, Clone)]
pub struct AudioConfig {
    pub sample_rate: u32,
    pub channels: u16,
    pub sample_format: SampleFormat,
}

/// Sample format enumeration
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SampleFormat {
    I16,
    I32,
    F32,
}

impl From<cpal::SampleFormat> for SampleFormat {
    fn from(format: cpal::SampleFormat) -> Self {
        match format {
            cpal::SampleFormat::I16 => SampleFormat::I16,
            cpal::SampleFormat::I32 => SampleFormat::I32,
            cpal::SampleFormat::F32 => SampleFormat::F32,
            _ => SampleFormat::F32, // Default fallback
        }
    }
}

impl From<SampleFormat> for cpal::SampleFormat {
    fn from(format: SampleFormat) -> Self {
        match format {
            SampleFormat::I16 => cpal::SampleFormat::I16,
            SampleFormat::I32 => cpal::SampleFormat::I32,
            SampleFormat::F32 => cpal::SampleFormat::F32,
        }
    }
}

/// Network protocol messages
#[derive(Debug, Clone)]
pub enum Message {
    AudioData(Vec<u8>),
    Config(AudioConfig),
    Error(String),
}

/// Default server port
pub const DEFAULT_PORT: u16 = 8080;

/// Default buffer size for audio streaming
pub const DEFAULT_BUFFER_SIZE: usize = 4096;
