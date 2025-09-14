

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