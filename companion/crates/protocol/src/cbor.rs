use core::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CborError {
    Eof,
    InvalidType,
    NonCanonical,
    Overflow,
    InvalidUtf8,
    Nesting,
    TrailingData,
}

impl fmt::Display for CborError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for CborError {}

#[derive(Debug, Default)]
pub struct Encoder {
    bytes: Vec<u8>,
}

impl Encoder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }

    fn argument(&mut self, major: u8, value: u64) {
        let prefix = major << 5;
        match value {
            0..=23 => self.bytes.push(prefix | u8::try_from(value).unwrap()),
            24..=0xff => {
                self.bytes.push(prefix | 0x18);
                self.bytes.push(u8::try_from(value).unwrap());
            }
            0x100..=0xffff => {
                self.bytes.push(prefix | 0x19);
                self.bytes
                    .extend_from_slice(&u16::try_from(value).unwrap().to_be_bytes());
            }
            0x1_0000..=0xffff_ffff => {
                self.bytes.push(prefix | 0x1a);
                self.bytes
                    .extend_from_slice(&u32::try_from(value).unwrap().to_be_bytes());
            }
            _ => {
                self.bytes.push(prefix | 0x1b);
                self.bytes.extend_from_slice(&value.to_be_bytes());
            }
        }
    }

    pub fn map(&mut self, len: usize) {
        self.argument(5, u64::try_from(len).unwrap());
    }

    pub fn array(&mut self, len: usize) {
        self.argument(4, u64::try_from(len).unwrap());
    }

    pub fn unsigned(&mut self, value: u64) {
        self.argument(0, value);
    }

    pub fn signed(&mut self, value: i64) {
        if value >= 0 {
            self.unsigned(value.unsigned_abs());
        } else {
            self.argument(1, value.unsigned_abs() - 1);
        }
    }

    pub fn text(&mut self, value: &str) {
        self.argument(3, u64::try_from(value.len()).unwrap());
        self.bytes.extend_from_slice(value.as_bytes());
    }

    pub fn boolean(&mut self, value: bool) {
        self.bytes.push(if value { 0xf5 } else { 0xf4 });
    }
}

pub struct Decoder<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Decoder<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    pub fn position(&self) -> usize {
        self.pos
    }

    pub fn slice(&self, start: usize, end: usize) -> &'a [u8] {
        &self.bytes[start..end]
    }

    fn byte(&mut self) -> Result<u8, CborError> {
        let value = *self.bytes.get(self.pos).ok_or(CborError::Eof)?;
        self.pos += 1;
        Ok(value)
    }

    fn take<const N: usize>(&mut self) -> Result<[u8; N], CborError> {
        let end = self.pos.checked_add(N).ok_or(CborError::Overflow)?;
        let src = self.bytes.get(self.pos..end).ok_or(CborError::Eof)?;
        self.pos = end;
        Ok(src.try_into().unwrap())
    }

    fn head(&mut self) -> Result<(u8, u64), CborError> {
        let first = self.byte()?;
        let major = first >> 5;
        let additional = first & 0x1f;
        let value = match additional {
            0..=23 => u64::from(additional),
            24 => {
                let value = u64::from(self.byte()?);
                if value < 24 {
                    return Err(CborError::NonCanonical);
                }
                value
            }
            25 => {
                let value = u64::from(u16::from_be_bytes(self.take()?));
                if value <= 0xff {
                    return Err(CborError::NonCanonical);
                }
                value
            }
            26 => {
                let value = u64::from(u32::from_be_bytes(self.take()?));
                if value <= 0xffff {
                    return Err(CborError::NonCanonical);
                }
                value
            }
            27 => {
                let value = u64::from_be_bytes(self.take()?);
                if value <= 0xffff_ffff {
                    return Err(CborError::NonCanonical);
                }
                value
            }
            _ => return Err(CborError::NonCanonical),
        };
        Ok((major, value))
    }

    pub fn map_len(&mut self) -> Result<usize, CborError> {
        let (major, value) = self.head()?;
        if major != 5 {
            return Err(CborError::InvalidType);
        }
        usize::try_from(value).map_err(|_| CborError::Overflow)
    }

    pub fn array_len(&mut self) -> Result<usize, CborError> {
        let (major, value) = self.head()?;
        if major != 4 {
            return Err(CborError::InvalidType);
        }
        usize::try_from(value).map_err(|_| CborError::Overflow)
    }

    pub fn unsigned(&mut self) -> Result<u64, CborError> {
        let (major, value) = self.head()?;
        if major != 0 {
            return Err(CborError::InvalidType);
        }
        Ok(value)
    }

    pub fn signed(&mut self) -> Result<i64, CborError> {
        let (major, value) = self.head()?;
        match major {
            0 => i64::try_from(value).map_err(|_| CborError::Overflow),
            1 if i64::try_from(value).is_ok() => Ok(-1 - i64::try_from(value).unwrap()),
            _ => Err(CborError::InvalidType),
        }
    }

    pub fn text(&mut self) -> Result<&'a str, CborError> {
        let (major, value) = self.head()?;
        if major != 3 {
            return Err(CborError::InvalidType);
        }
        let len = usize::try_from(value).map_err(|_| CborError::Overflow)?;
        let end = self.pos.checked_add(len).ok_or(CborError::Overflow)?;
        let raw = self.bytes.get(self.pos..end).ok_or(CborError::Eof)?;
        self.pos = end;
        std::str::from_utf8(raw).map_err(|_| CborError::InvalidUtf8)
    }

    pub fn boolean(&mut self) -> Result<bool, CborError> {
        match self.byte()? {
            0xf4 => Ok(false),
            0xf5 => Ok(true),
            _ => Err(CborError::InvalidType),
        }
    }

    pub fn peek_major(&self) -> Result<u8, CborError> {
        Ok(*self.bytes.get(self.pos).ok_or(CborError::Eof)? >> 5)
    }

    pub fn skip(&mut self) -> Result<(), CborError> {
        self.skip_at_depth(0)
    }

    fn skip_at_depth(&mut self, depth: u8) -> Result<(), CborError> {
        if depth >= 8 {
            return Err(CborError::Nesting);
        }
        let start = self.pos;
        let (major, value) = self.head()?;
        match major {
            0 | 1 => Ok(()),
            2 | 3 => {
                let len = usize::try_from(value).map_err(|_| CborError::Overflow)?;
                self.pos = self.pos.checked_add(len).ok_or(CborError::Overflow)?;
                if self.pos > self.bytes.len() {
                    self.pos = start;
                    return Err(CborError::Eof);
                }
                if major == 3 {
                    std::str::from_utf8(&self.bytes[self.pos - len..self.pos])
                        .map_err(|_| CborError::InvalidUtf8)?;
                }
                Ok(())
            }
            4 => {
                for _ in 0..value {
                    self.skip_at_depth(depth + 1)?;
                }
                Ok(())
            }
            5 => {
                for _ in 0..value {
                    self.skip_at_depth(depth + 1)?;
                    self.skip_at_depth(depth + 1)?;
                }
                Ok(())
            }
            7 if value == 20 || value == 21 || value == 22 => Ok(()),
            _ => Err(CborError::InvalidType),
        }
    }

    pub fn finish(self) -> Result<(), CborError> {
        if self.pos == self.bytes.len() {
            Ok(())
        } else {
            Err(CborError::TrailingData)
        }
    }
}

pub fn deterministic_key_before(previous: &[u8], current: &[u8]) -> bool {
    previous.len() < current.len() || (previous.len() == current.len() && previous < current)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortest_integer_forms_round_trip() {
        for value in [0, 23, 24, 255, 256, 65_535, 65_536, u64::MAX] {
            let mut encoder = Encoder::new();
            encoder.unsigned(value);
            let bytes = encoder.into_bytes();
            let mut decoder = Decoder::new(&bytes);
            assert_eq!(decoder.unsigned().unwrap(), value);
            decoder.finish().unwrap();
        }
    }

    #[test]
    fn rejects_noncanonical_integer() {
        let mut decoder = Decoder::new(&[0x18, 0x01]);
        assert_eq!(decoder.unsigned(), Err(CborError::NonCanonical));
    }
}
