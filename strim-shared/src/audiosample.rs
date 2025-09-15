

pub trait AudioSample: Default + Copy {
    const BYTE_SIZE: usize;
    type ByteArray: AsRef<[u8]>;
    fn from_le_bytes(bytes: &[u8]) -> Self;
    fn to_le_bytes(self) -> Self::ByteArray;
}

impl AudioSample for f32 {
    const BYTE_SIZE: usize = 4;
    type ByteArray = [u8; 4];
    fn from_le_bytes(bytes: &[u8]) -> Self {
        f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
    }
    fn to_le_bytes(self) -> Self::ByteArray {
        f32::to_le_bytes(self)
    }
}

impl AudioSample for i32 {
    const BYTE_SIZE: usize = 4;
    type ByteArray = [u8; 4];
    fn from_le_bytes(bytes: &[u8]) -> Self {
        i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
    }
    fn to_le_bytes(self) -> Self::ByteArray {
        i32::to_le_bytes(self)
    }
}

impl AudioSample for i16 {
    const BYTE_SIZE: usize = 2;    
    type ByteArray = [u8; 2];
    fn from_le_bytes(bytes: &[u8]) -> Self {
        i16::from_le_bytes([bytes[0], bytes[1]])
    }
    fn to_le_bytes(self) -> Self::ByteArray {
        i16::to_le_bytes(self)
    }
}

impl AudioSample for u16 {
    const BYTE_SIZE: usize = 2;    
    type ByteArray = [u8; 2];
    fn from_le_bytes(bytes: &[u8]) -> Self {
        u16::from_le_bytes([bytes[0], bytes[1]])
    }
    fn to_le_bytes(self) -> Self::ByteArray {
        u16::to_le_bytes(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_f32_audio_sample() {
        let original: f32 = 0.5;
        let bytes = original.to_le_bytes();
        let restored = f32::from_le_bytes(bytes);
        
        assert_eq!(f32::BYTE_SIZE, 4);
        assert_eq!(bytes.len(), 4);
        assert_eq!(original, restored);
        
        // Test edge cases
        let test_values = [0.0, 1.0, -1.0, f32::MAX, f32::MIN, f32::EPSILON];
        for &value in &test_values {
            let bytes = value.to_le_bytes();
            let restored = f32::from_le_bytes(bytes);
            assert_eq!(value, restored, "f32 roundtrip failed for {}", value);
        }
    }

    #[test]
    fn test_i32_audio_sample() {
        let original: i32 = 42000;
        let bytes = original.to_le_bytes();
        let restored = i32::from_le_bytes(bytes);
        
        assert_eq!(i32::BYTE_SIZE, 4);
        assert_eq!(bytes.len(), 4);
        assert_eq!(original, restored);
        
        // Test edge cases
        let test_values = [0, 1, -1, i32::MAX, i32::MIN, 32767, -32768];
        for &value in &test_values {
            let bytes = value.to_le_bytes();
            let restored = i32::from_le_bytes(bytes);
            assert_eq!(value, restored, "i32 roundtrip failed for {}", value);
        }
    }

    #[test]
    fn test_i16_audio_sample() {
        let original: i16 = 12345;
        let bytes = original.to_le_bytes();
        let restored = i16::from_le_bytes(bytes);
        
        assert_eq!(i16::BYTE_SIZE, 2);
        assert_eq!(bytes.len(), 2);
        assert_eq!(original, restored);
        
        // Test edge cases
        let test_values = [0, 1, -1, i16::MAX, i16::MIN];
        for &value in &test_values {
            let bytes = value.to_le_bytes();
            let restored = i16::from_le_bytes(bytes);
            assert_eq!(value, restored, "i16 roundtrip failed for {}", value);
        }
    }

    #[test]
    fn test_u16_audio_sample() {
        let original: u16 = 54321;
        let bytes = original.to_le_bytes();
        let restored = u16::from_le_bytes(bytes);
        
        assert_eq!(u16::BYTE_SIZE, 2);
        assert_eq!(bytes.len(), 2);
        assert_eq!(original, restored);
        
        // Test edge cases
        let test_values = [0, 1, u16::MAX, u16::MIN, 32767, 65535];
        for &value in &test_values {
            let bytes = value.to_le_bytes();
            let restored = u16::from_le_bytes(bytes);
            assert_eq!(value, restored, "u16 roundtrip failed for {}", value);
        }
    }

    #[test]
    fn test_audio_sample_byte_sizes() {
        assert_eq!(f32::BYTE_SIZE, std::mem::size_of::<f32>());
        assert_eq!(i32::BYTE_SIZE, std::mem::size_of::<i32>());
        assert_eq!(i16::BYTE_SIZE, std::mem::size_of::<i16>());
        assert_eq!(u16::BYTE_SIZE, std::mem::size_of::<u16>());
    }

    #[test]
    fn test_audio_sample_default_values() {
        assert_eq!(f32::default(), 0.0);
        assert_eq!(i32::default(), 0);
        assert_eq!(i16::default(), 0);
        assert_eq!(u16::default(), 0);
    }
}
