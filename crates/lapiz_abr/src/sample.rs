use std::fmt::{self, Debug};

use anyhow::{Context as _, Result, bail, ensure};
use image::{DynamicImage, ImageBuffer, Luma};
use uuid::Uuid;

use crate::{cursor::Cursor, rle};

/// Decoded grayscale alpha bitmap of a [`Sample`], preserving its 8-bit or 16-bit channel depth.
pub enum SampleImage {
    Bit8(ImageBuffer<Luma<u8>, Vec<u8>>),
    Bit16(ImageBuffer<Luma<u16>, Vec<u16>>),
}

impl SampleImage {
    pub fn into_dynamic(self) -> DynamicImage {
        match self {
            SampleImage::Bit8(b) => DynamicImage::ImageLuma8(b),
            SampleImage::Bit16(b) => DynamicImage::ImageLuma16(b),
        }
    }
}

/// A sampled brush tip mask decoded from the `samp` section of an [`Abr`](crate::Abr) file.
///
/// Stores the raw or PackBits RLE-compressed grayscale alpha bitmap for a custom brush tip,
/// along with its [`Uuid`], pixel bounding box, and bit depth (`8` or `16`). Sampled brush tips
/// ([`SampledBrushTip`](crate::descriptor::class::SampledBrushTip)) reference their corresponding
/// `Sample` by [`id`](Self::id), and [`as_image`](Self::as_image) decodes the payload into a [`SampleImage`].
pub struct Sample {
    pub id: Uuid,
    pub top: u32,
    pub left: u32,
    pub bottom: u32,
    pub right: u32,
    pub depth: u16,
    pub compression: u8,
    pub pixel_data: Vec<u8>,
}

impl Debug for Sample {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Sample")
            .field("id", &self.id)
            .field("top", &self.top)
            .field("left", &self.left)
            .field("bottom", &self.bottom)
            .field("right", &self.right)
            .field("depth", &self.depth)
            .field("compression", &self.compression)
            .field("pixel_data", &format!("{} bytes", self.pixel_data.len()))
            .finish()
    }
}

impl Sample {
    pub(crate) fn parse_samp_section(cursor: &mut Cursor<'_>) -> Result<Vec<Self>> {
        let mut samples = Vec::new();

        while cursor.remaining() != 0 {
            let len = usize::try_from(cursor.read_u32_be()?)?;
            let mut record = cursor.take_cursor(len)?;
            let id_len = usize::from(record.read_u8()?);
            let id = Uuid::try_parse_ascii(record.take(id_len)?)?;

            // unknown
            record.skip(264)?;

            let top = record.read_u32_be()?;
            let left = record.read_u32_be()?;
            let bottom = record.read_u32_be()?;
            let right = record.read_u32_be()?;
            let depth = record.read_u16_be()?;
            let compression = record.read_u8()?;
            let pixel_data = record.take(record.remaining())?.to_vec();

            samples.push(Self {
                id,
                top,
                left,
                bottom,
                right,
                depth,
                compression,
                pixel_data,
            });

            cursor.align_to(4)?;
        }

        Ok(samples)
    }

    pub fn as_image(&self) -> Result<SampleImage> {
        let width = self
            .right
            .checked_sub(self.left)
            .context("sample width underflows u32")?;
        let height = self
            .bottom
            .checked_sub(self.top)
            .context("sample height underflows u32")?;
        ensure!(
            width > 0 && height > 0,
            "invalid sample bounds ({}, {}, {}, {})",
            self.top,
            self.left,
            self.bottom,
            self.right
        );

        let width_usize = usize::try_from(width)?;
        let height_usize = usize::try_from(height)?;
        let bytes_per_pixel = match self.depth {
            8 => 1,
            16 => 2,
            depth => bail!("unsupported sample depth {depth}"),
        };
        let row_bytes = width_usize
            .checked_mul(bytes_per_pixel)
            .context("sample row size overflows usize")?;
        let expected_len = row_bytes
            .checked_mul(height_usize)
            .context("sample image size overflows usize")?;

        let decoded = match self.compression {
            0 => {
                ensure!(
                    self.pixel_data.len() >= expected_len,
                    "raw sample data has {} bytes, expected at least {expected_len}",
                    self.pixel_data.len()
                );
                self.pixel_data[..expected_len].to_vec()
            }
            1 => {
                let mut data = Cursor::new(&self.pixel_data);
                rle::decode(&mut data, height_usize, row_bytes)?
            }
            compression => bail!("unsupported sample compression {compression}"),
        };

        match self.depth {
            8 => Ok(SampleImage::Bit8(
                ImageBuffer::from_raw(width, height, decoded)
                    .context("failed to construct 8-bit sample image")?,
            )),
            16 => {
                let pixels = decoded
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|bytes| u16::from_be_bytes(*bytes))
                    .collect();
                Ok(SampleImage::Bit16(
                    ImageBuffer::from_raw(width, height, pixels)
                        .context("failed to construct 16-bit sample image")?,
                ))
            }
            _ => unreachable!(),
        }
    }
}
