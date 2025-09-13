//! Shared types and utilities for strim audio streaming

use anyhow::Result;
use serde::{Deserialize, Serialize};

/// Audio stream configuration sent from server to client
/// Contains all the information needed to properly decode audio data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioConfig {
    /// Sample rate in Hz (e.g., 44100, 48000)
    pub sample_rate: u32,
    /// Number of audio channels (1 = mono, 2 = stereo)
    pub channels: u16,
    /// Format of each audio sample
    pub sample_format: SampleFormat,
}

/// Sample format enumeration for audio data
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SampleFormat {
    /// 16-bit signed integer samples
    I16,
    /// 32-bit signed integer samples  
    I32,
    /// 32-bit floating point samples
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

/// Network protocol messages for communication between server and client
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Message {
    /// Raw audio data to be played by the client
    AudioData(Vec<u8>),
    /// Audio configuration sent when client connects
    Config(AudioConfig),
    /// Error message from either server or client
    Error(String),
}

impl Message {
    /// Serialize a message to bytes for network transmission with length prefix
    /// Returns the serialized message as a byte vector with 4-byte length prefix
    pub fn serialize(&self) -> Result<Vec<u8>> {
        let data = bincode::serialize(self).map_err(|e| anyhow::anyhow!("Serialization failed: {}", e))?;
        let mut result = Vec::with_capacity(4 + data.len());
        result.extend_from_slice(&(data.len() as u32).to_le_bytes());
        result.extend_from_slice(&data);
        Ok(result)
    }

    /// Deserialize bytes back into a Message from length-prefixed data
    /// Takes raw bytes from network and converts them to a Message
    /// Returns (message, remaining_bytes) if successful
    pub fn deserialize(data: &[u8]) -> Result<(Self, &[u8])> {
        if data.len() < 4 {
            return Err(anyhow::anyhow!("Not enough data for length prefix"));
        }
        
        let length = u32::from_le_bytes([data[0], data[1], data[2], data[3]]) as usize;
        
        if data.len() < 4 + length {
            return Err(anyhow::anyhow!("Not enough data for complete message"));
        }
        
        let message_data = &data[4..4 + length];
        let message = bincode::deserialize(message_data).map_err(|e| anyhow::anyhow!("Deserialization failed: {}", e))?;
        let remaining = &data[4 + length..];
        
        Ok((message, remaining))
    }
}

/// Default server port for audio streaming
pub const DEFAULT_PORT: u16 = 8080;

/// Default buffer size for audio streaming (4KB chunks)
pub const DEFAULT_BUFFER_SIZE: usize = 4096;
