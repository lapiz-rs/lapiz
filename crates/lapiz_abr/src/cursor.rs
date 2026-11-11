use anyhow::{Context as _, Result, bail};

pub struct Cursor<'a> {
    input: &'a [u8],
    position: usize,
}

impl<'a> Cursor<'a> {
    pub fn new(input: &'a [u8]) -> Self {
        Self { input, position: 0 }
    }

    pub fn remaining(&self) -> usize {
        self.input.len() - self.position
    }

    pub fn position(&self) -> usize {
        self.position
    }

    pub fn read_u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    pub fn read_u16_be(&mut self) -> Result<u16> {
        let bytes = self.take(2)?;

        Ok(u16::from_be_bytes([bytes[0], bytes[1]]))
    }

    pub fn read_i32_be(&mut self) -> Result<i32> {
        let bytes = self.take(4)?;

        Ok(i32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    pub fn read_u32_be(&mut self) -> Result<u32> {
        let bytes = self.take(4)?;

        Ok(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    pub fn read_f64_be(&mut self) -> Result<f64> {
        let bytes = self.take(8)?;

        Ok(f64::from_be_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ]))
    }

    pub fn read_ostype(&mut self) -> Result<[u8; 4]> {
        let bytes = self.take(4)?;
        Ok([bytes[0], bytes[1], bytes[2], bytes[3]])
    }

    pub fn read_descriptor_id(&mut self) -> Result<String> {
        let len = usize::try_from(self.read_u32_be()?)?;
        let len = if len == 0 { 4 } else { len };

        Ok(String::from_utf8(self.take(len)?.to_vec())?)
    }

    pub fn read_utf16_string(&mut self) -> Result<String> {
        let len = usize::try_from(self.read_u32_be()?)?;
        let bytes = self.take(
            len.checked_mul(2)
                .context("ABR UTF-16 string length overflow")?,
        )?;
        let mut units = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|bytes| u16::from_be_bytes(*bytes))
            .collect::<Vec<_>>();
        if units.last() == Some(&0) {
            units.pop();
        }

        Ok(String::from_utf16(&units)?)
    }

    pub fn take(&mut self, len: usize) -> Result<&'a [u8]> {
        let end = self
            .position
            .checked_add(len)
            .context("ABR data length overflow")?;
        let bytes = self
            .input
            .get(self.position..end)
            .context("unexpected end of ABR data")?;
        self.position = end;

        Ok(bytes)
    }

    pub fn take_cursor(&mut self, len: usize) -> Result<Self> {
        Ok(Self::new(self.take(len)?))
    }

    pub fn skip(&mut self, len: usize) -> Result<()> {
        self.take(len)?;
        Ok(())
    }

    pub fn align_to(&mut self, n: usize) -> Result<()> {
        if n == 0 {
            bail!("ABR alignment must be greater than zero");
        }

        let remainder = self.position % n;
        if remainder != 0 {
            self.skip(n - remainder)?;
        }

        Ok(())
    }
}
