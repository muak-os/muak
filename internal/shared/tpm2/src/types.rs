//! TPM2 type definitions and constants.

pub const TPM2_ST_NO_SESSIONS: u16 = 0x8001;
pub const TPM2_ST_SESSIONS: u16 = 0x8002;

pub const TPM2_CC_EVICT_CONTROL: u32 = 0x00000120;
pub const TPM2_CC_CREATE_PRIMARY: u32 = 0x00000131;
pub const TPM2_CC_CREATE: u32 = 0x00000153;
pub const TPM2_CC_LOAD: u32 = 0x00000157;
pub const TPM2_CC_UNSEAL: u32 = 0x0000015E;
pub const TPM2_CC_FLUSH_CONTEXT: u32 = 0x00000165;
pub const TPM2_CC_START_AUTH_SESSION: u32 = 0x00000176;
pub const TPM2_CC_POLICY_PCR: u32 = 0x0000017F;
pub const TPM2_CC_GET_CAPABILITY: u32 = 0x0000017A;

pub const TPM2_RH_OWNER: u32 = 0x40000001;
pub const TPM2_RH_NULL: u32 = 0x40000007;
pub const TPM2_RS_PW: u32 = 0x40000009;

pub const TPM2_ALG_SHA256: u16 = 0x000B;
pub const TPM2_ALG_KEYEDHASH: u16 = 0x0008;
pub const TPM2_ALG_NULL: u16 = 0x0010;
pub const TPM2_ALG_AES: u16 = 0x0006;
pub const TPM2_ALG_CFB: u16 = 0x0043;
pub const TPM2_ALG_ECC: u16 = 0x0023;

pub const TPM2_ECC_NIST_P256: u16 = 0x0003;

pub const TPM2_SE_POLICY: u8 = 0x01;

pub const TPM2_CAP_HANDLES: u32 = 0x00000001;

pub const SRK_HANDLE: u32 = 0x81000001;

pub const PCR_INDEX: u32 = 11;
pub const SHA256_DIGEST_SIZE: usize = 32;

/// Marshalling buffer for building TPM2 commands.
pub struct CommandBuffer {
    buf: Vec<u8>,
}

impl CommandBuffer {
    pub fn new(tag: u16, command_code: u32) -> Self {
        let mut buf = Vec::with_capacity(256);
        buf.extend_from_slice(&tag.to_be_bytes());
        buf.extend_from_slice(&0u32.to_be_bytes());
        buf.extend_from_slice(&command_code.to_be_bytes());
        Self { buf }
    }

    pub fn write_u8(&mut self, v: u8) {
        self.buf.push(v);
    }

    pub fn write_u16(&mut self, v: u16) {
        self.buf.extend_from_slice(&v.to_be_bytes());
    }

    pub fn write_u32(&mut self, v: u32) {
        self.buf.extend_from_slice(&v.to_be_bytes());
    }

    pub fn write_bytes(&mut self, data: &[u8]) {
        self.buf.extend_from_slice(data);
    }

    pub fn write_sized(&mut self, data: &[u8]) {
        self.write_u16(data.len() as u16);
        self.buf.extend_from_slice(data);
    }

    pub fn finalize(mut self) -> Vec<u8> {
        let len = self.buf.len() as u32;
        self.buf[2..6].copy_from_slice(&len.to_be_bytes());
        self.buf
    }
}

/// Reader for parsing TPM2 response buffers.
pub struct ResponseReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> ResponseReader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    pub fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.pos)
    }

    pub fn read_u8(&mut self) -> crate::errors::Result<u8> {
        if self.remaining() < 1 {
            return Err(crate::errors::Error::ResponseTooShort {
                expected: self.pos + 1,
                actual: self.data.len(),
            });
        }
        let v = self.data[self.pos];
        self.pos += 1;
        Ok(v)
    }

    pub fn read_u16(&mut self) -> crate::errors::Result<u16> {
        if self.remaining() < 2 {
            return Err(crate::errors::Error::ResponseTooShort {
                expected: self.pos + 2,
                actual: self.data.len(),
            });
        }
        let v = u16::from_be_bytes([self.data[self.pos], self.data[self.pos + 1]]);
        self.pos += 2;
        Ok(v)
    }

    pub fn read_u32(&mut self) -> crate::errors::Result<u32> {
        if self.remaining() < 4 {
            return Err(crate::errors::Error::ResponseTooShort {
                expected: self.pos + 4,
                actual: self.data.len(),
            });
        }
        let v = u32::from_be_bytes([
            self.data[self.pos],
            self.data[self.pos + 1],
            self.data[self.pos + 2],
            self.data[self.pos + 3],
        ]);
        self.pos += 4;
        Ok(v)
    }

    pub fn read_bytes(&mut self, len: usize) -> crate::errors::Result<&'a [u8]> {
        if self.remaining() < len {
            return Err(crate::errors::Error::ResponseTooShort {
                expected: self.pos + len,
                actual: self.data.len(),
            });
        }
        let slice = &self.data[self.pos..self.pos + len];
        self.pos += len;
        Ok(slice)
    }

    pub fn read_sized(&mut self) -> crate::errors::Result<&'a [u8]> {
        let len = self.read_u16()? as usize;
        self.read_bytes(len)
    }
}
