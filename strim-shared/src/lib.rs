//! Shared types and utilities for strim audio streaming

use anyhow::Result;
use serde::{Deserialize, Serialize};
mod audiosample;

pub use audiosample::AudioSample;

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
    /// 16-bit unsigned integer samples
    U16,
}

impl TryFrom<cpal::SampleFormat> for SampleFormat {
    
    type Error = anyhow::Error;

    fn try_from(format: cpal::SampleFormat) -> Result<Self> {
        match format {
            cpal::SampleFormat::I16 => Ok(SampleFormat::I16),
            cpal::SampleFormat::I32 => Ok(SampleFormat::I32),
            cpal::SampleFormat::F32 => Ok(SampleFormat::F32),
            cpal::SampleFormat::U16 => Ok(SampleFormat::U16),
            _ => Err(anyhow::anyhow!("Unsupported sample format")),
        }
    }
}

impl From<SampleFormat> for cpal::SampleFormat {
    fn from(format: SampleFormat) -> Self {
        match format {
            SampleFormat::I16 => cpal::SampleFormat::I16,
            SampleFormat::I32 => cpal::SampleFormat::I32,
            SampleFormat::F32 => cpal::SampleFormat::F32,
            SampleFormat::U16 => cpal::SampleFormat::U16
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sample_format_conversion_from_cpal() {
        assert_eq!(SampleFormat::try_from(cpal::SampleFormat::I16).unwrap(), SampleFormat::I16);
        assert_eq!(SampleFormat::try_from(cpal::SampleFormat::I32).unwrap(), SampleFormat::I32);
        assert_eq!(SampleFormat::try_from(cpal::SampleFormat::F32).unwrap(), SampleFormat::F32);
        assert_eq!(SampleFormat::try_from(cpal::SampleFormat::U16).unwrap(), SampleFormat::U16);
    }

    #[test]
    fn test_sample_format_conversion_to_cpal() {
        assert_eq!(cpal::SampleFormat::from(SampleFormat::I16), cpal::SampleFormat::I16);
        assert_eq!(cpal::SampleFormat::from(SampleFormat::I32), cpal::SampleFormat::I32);
        assert_eq!(cpal::SampleFormat::from(SampleFormat::F32), cpal::SampleFormat::F32);
        assert_eq!(cpal::SampleFormat::from(SampleFormat::U16), cpal::SampleFormat::U16);
    }

    #[test]
    fn test_audio_config_creation() {
        let config = AudioConfig {
            sample_rate: 44100,
            channels: 2,
            sample_format: SampleFormat::F32,
        };
        
        assert_eq!(config.sample_rate, 44100);
        assert_eq!(config.channels, 2);
        assert_eq!(config.sample_format, SampleFormat::F32);
    }

    #[test]
    fn test_message_serialization_roundtrip() {
        let test_cases = vec![
            Message::AudioData(vec![1, 2, 3, 4, 5]),
            Message::Config(AudioConfig {
                sample_rate: 48000,
                channels: 1,
                sample_format: SampleFormat::I16,
            }),
            Message::Error("Test error message".to_string()),
        ];

        for original_msg in test_cases {
            // Serialize the message
            let serialized = original_msg.serialize().expect("Serialization should succeed");
            
            // Check that serialized data has proper length prefix
            assert!(serialized.len() >= 4, "Serialized data should have at least 4 bytes for length prefix");
            
            // Extract length from prefix
            let expected_length = u32::from_le_bytes([serialized[0], serialized[1], serialized[2], serialized[3]]) as usize;
            assert_eq!(serialized.len() - 4, expected_length, "Length prefix should match data length");
            
            // Deserialize back
            let (deserialized_msg, remaining) = Message::deserialize(&serialized)
                .expect("Deserialization should succeed");
            
            // Check that no bytes remain
            assert!(remaining.is_empty(), "No bytes should remain after deserialization");
            
            // Compare messages (we need to implement comparison logic)
            match (&original_msg, &deserialized_msg) {
                (Message::AudioData(orig), Message::AudioData(deser)) => {
                    assert_eq!(orig, deser, "AudioData should match");
                }
                (Message::Config(orig), Message::Config(deser)) => {
                    assert_eq!(orig.sample_rate, deser.sample_rate);
                    assert_eq!(orig.channels, deser.channels);
                    assert_eq!(orig.sample_format, deser.sample_format);
                }
                (Message::Error(orig), Message::Error(deser)) => {
                    assert_eq!(orig, deser, "Error messages should match");
                }
                _ => panic!("Message types should match"),
            }
        }
    }

    #[test]
    fn test_message_deserialization_insufficient_data() {
        // Test with less than 4 bytes (no length prefix)
        let insufficient_data = vec![1, 2, 3];
        let result = Message::deserialize(&insufficient_data);
        assert!(result.is_err(), "Should fail with insufficient data for length prefix");
        
        // Test with length prefix but insufficient message data
        let mut partial_data = vec![];
        partial_data.extend_from_slice(&(10u32).to_le_bytes()); // Claims 10 bytes of data
        partial_data.extend_from_slice(&[1, 2, 3]); // But only has 3 bytes
        
        let result = Message::deserialize(&partial_data);
        assert!(result.is_err(), "Should fail with insufficient data for complete message");
    }

    #[test]
    fn test_message_deserialization_with_remaining_bytes() {
        let msg = Message::AudioData(vec![1, 2, 3]);
        let mut serialized = msg.serialize().unwrap();
        
        // Add some extra bytes
        serialized.extend_from_slice(&[99, 100, 101]);
        
        let (deserialized, remaining) = Message::deserialize(&serialized)
            .expect("Deserialization should succeed");
        
        assert_eq!(remaining.len(), 3, "Should have 3 remaining bytes");
        assert_eq!(remaining, &[99, 100, 101], "Remaining bytes should match");
        
        // Check the deserialized message
        if let Message::AudioData(data) = deserialized {
            assert_eq!(data, vec![1, 2, 3]);
        } else {
            panic!("Should deserialize as AudioData");
        }
    }

    #[test]
    fn test_constants() {
        assert_eq!(DEFAULT_PORT, 8080);
        assert_eq!(DEFAULT_BUFFER_SIZE, 4096);
    }
}
