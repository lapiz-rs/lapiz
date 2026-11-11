use std::fmt::{self, Debug};

use anyhow::{Context as _, Result, bail, ensure};
use image::{DynamicImage, ImageBuffer, Luma, Rgb, Rgba};
use uuid::Uuid;

use crate::{cursor::Cursor, rle};

pub enum ColorMode {
    Gray,
    Indexed { table: Vec<u8> },
    Rgb,
}

impl Debug for ColorMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Gray => write!(f, "Gray"),
            Self::Indexed { table } => f
                .debug_struct("Indexed")
                .field("table", &format!("{} bytes", table.len()))
                .finish(),
            Self::Rgb => write!(f, "Rgb"),
        }
    }
}

#[derive(Debug)]
pub struct Pattern {
    pub id: Uuid,
    pub name: String,
    pub color_mode: ColorMode,
    pub height: u16,
    pub width: u16,
    pub top: u32,
    pub left: u32,
    pub bottom: u32,
    pub right: u32,
    pub channel_count: u32,
    pub channels: Vec<Option<PatternChannel>>,
}

pub struct PatternChannel {
    pub depth: u32,
    pub top: u32,
    pub left: u32,
    pub bottom: u32,
    pub right: u32,
    pub compression: u8,
    pub pixel_data: Vec<u8>,
}

impl Debug for PatternChannel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PatternChannel")
            .field("depth", &self.depth)
            .field("top", &self.top)
            .field("left", &self.left)
            .field("bottom", &self.bottom)
            .field("right", &self.right)
            .field("compression", &self.compression)
            .field("pixel_data", &format!("{} bytes", self.pixel_data.len()))
            .finish()
    }
}

impl Pattern {
    pub(crate) fn parse_patt_section(cursor: &mut Cursor<'_>) -> Result<Vec<Self>> {
        let mut patterns = Vec::new();

        while cursor.remaining() != 0 {
            let len = usize::try_from(cursor.read_u32_be()?)?;
            let mut record = cursor.take_cursor(len)?;

            let version = record.read_u32_be()?;
            if version != 1 {
                bail!("unsupported ABR pattern version {version}");
            }

            let color_mode = record.read_u32_be()?;
            let height = record.read_u16_be()?;
            let width = record.read_u16_be()?;

            let name = record.read_utf16_string()?;

            let id_len = usize::from(record.read_u8()?);
            let id = Uuid::try_parse_ascii(record.take(id_len)?)?;
            let color_mode = match color_mode {
                1 => ColorMode::Gray,
                2 => ColorMode::Indexed {
                    table: record.take(772)?.to_vec(),
                },
                3 => ColorMode::Rgb,
                mode => bail!("unsupported ABR pattern color mode {mode}"),
            };

            let array_version = record.read_u32_be()?;
            if array_version != 3 {
                bail!("unsupported ABR pattern array version {array_version}");
            }

            let array_len = usize::try_from(record.read_u32_be()?)?;
            let mut array = record.take_cursor(array_len)?;
            let top = array.read_u32_be()?;
            let left = array.read_u32_be()?;
            let bottom = array.read_u32_be()?;
            let right = array.read_u32_be()?;
            let channel_count = array.read_u32_be()?;
            let slot_count = usize::try_from(
                channel_count
                    .checked_add(2)
                    .context("ABR pattern channel count overflow")?,
            )?;
            let mut channels = Vec::with_capacity(slot_count);

            for _ in 0..slot_count {
                if array.read_u32_be()? == 0 {
                    channels.push(None);
                    continue;
                }

                let channel_len = usize::try_from(array.read_u32_be()?)?;
                if channel_len == 0 {
                    channels.push(None);
                    continue;
                }

                let mut channel = array.take_cursor(channel_len)?;
                let depth = channel.read_u32_be()?;
                let channel_top = channel.read_u32_be()?;
                let channel_left = channel.read_u32_be()?;
                let channel_bottom = channel.read_u32_be()?;
                let channel_right = channel.read_u32_be()?;
                let second_depth = channel.read_u16_be()?;
                if u32::from(second_depth) != depth {
                    bail!("inconsistent ABR pattern channel depths {depth} and {second_depth}");
                }
                let compression = channel.read_u8()?;
                let pixel_data = channel.take(channel.remaining())?.to_vec();

                channels.push(Some(PatternChannel {
                    depth,
                    top: channel_top,
                    left: channel_left,
                    bottom: channel_bottom,
                    right: channel_right,
                    compression,
                    pixel_data,
                }));
            }

            if array.remaining() != 0 || record.remaining() != 0 {
                bail!("unexpected trailing ABR pattern data");
            }

            patterns.push(Self {
                id,
                name,
                color_mode,
                height,
                width,
                top,
                left,
                bottom,
                right,
                channel_count,
                channels,
            });

            cursor.align_to(4)?;
        }

        Ok(patterns)
    }

    pub fn as_image(&self) -> Result<DynamicImage> {
        let width = self
            .right
            .checked_sub(self.left)
            .context("pattern width underflows u32")?;
        let height = self
            .bottom
            .checked_sub(self.top)
            .context("pattern height underflows u32")?;
        ensure!(
            width > 0 && height > 0,
            "invalid pattern bounds ({}, {}, {}, {})",
            self.top,
            self.left,
            self.bottom,
            self.right
        );
        ensure!(
            width == u32::from(self.width) && height == u32::from(self.height),
            "pattern size {}x{} does not match bounds {}x{}",
            self.width,
            self.height,
            width,
            height
        );

        let channel_count = match &self.color_mode {
            ColorMode::Gray | ColorMode::Indexed { .. } => 1,
            ColorMode::Rgb => 3,
        };
        let mut channels = Vec::with_capacity(channel_count);
        for index in 0..channel_count {
            let channel = self
                .channels
                .get(index)
                .and_then(Option::as_ref)
                .with_context(|| format!("pattern color channel {index} is missing"))?;
            ensure!(
                channel.top == self.top
                    && channel.left == self.left
                    && channel.bottom == self.bottom
                    && channel.right == self.right,
                "pattern color channel {index} bounds do not match the pattern bounds"
            );
            channels.push((channel.depth, channel.decode()?));
        }

        let depth = channels[0].0;
        ensure!(
            channels.iter().all(|channel| channel.0 == depth),
            "pattern color channel depths do not match"
        );
        let pixel_count = usize::try_from(width)?
            .checked_mul(usize::try_from(height)?)
            .context("pattern pixel count overflows usize")?;

        match (&self.color_mode, depth) {
            (ColorMode::Gray, 8) => Ok(DynamicImage::ImageLuma8(
                ImageBuffer::<Luma<u8>, _>::from_raw(width, height, channels.remove(0).1)
                    .context("failed to construct 8-bit grayscale pattern image")?,
            )),
            (ColorMode::Gray, 16) => {
                let pixels = channels[0]
                    .1
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|bytes| u16::from_be_bytes(*bytes))
                    .collect();
                Ok(DynamicImage::ImageLuma16(
                    ImageBuffer::<Luma<u16>, _>::from_raw(width, height, pixels)
                        .context("failed to construct 16-bit grayscale pattern image")?,
                ))
            }
            (ColorMode::Indexed { table }, 8) => {
                ensure!(table.len() == 772, "invalid indexed pattern color table");
                let transparent = usize::from(u16::from_be_bytes([table[770], table[771]]));
                let mut pixels = Vec::with_capacity(
                    pixel_count
                        .checked_mul(4)
                        .context("indexed pattern image size overflows usize")?,
                );
                for index in &channels[0].1 {
                    let index = usize::from(*index);
                    let color = index * 3;
                    pixels.extend_from_slice(&[
                        table[color],
                        table[color + 1],
                        table[color + 2],
                        if index == transparent { 0 } else { 255 },
                    ]);
                }
                Ok(DynamicImage::ImageRgba8(
                    ImageBuffer::<Rgba<u8>, _>::from_raw(width, height, pixels)
                        .context("failed to construct indexed pattern image")?,
                ))
            }
            (ColorMode::Rgb, 8) => {
                let mut pixels = Vec::with_capacity(
                    pixel_count
                        .checked_mul(3)
                        .context("RGB pattern image size overflows usize")?,
                );
                for index in 0..pixel_count {
                    pixels.extend_from_slice(&[
                        channels[0].1[index],
                        channels[1].1[index],
                        channels[2].1[index],
                    ]);
                }
                Ok(DynamicImage::ImageRgb8(
                    ImageBuffer::<Rgb<u8>, _>::from_raw(width, height, pixels)
                        .context("failed to construct 8-bit RGB pattern image")?,
                ))
            }
            (ColorMode::Rgb, 16) => {
                let mut pixels = Vec::with_capacity(
                    pixel_count
                        .checked_mul(3)
                        .context("RGB pattern image size overflows usize")?,
                );
                for index in 0..pixel_count {
                    let offset = index * 2;
                    for channel in &channels {
                        pixels.push(u16::from_be_bytes([
                            channel.1[offset],
                            channel.1[offset + 1],
                        ]));
                    }
                }
                Ok(DynamicImage::ImageRgb16(
                    ImageBuffer::<Rgb<u16>, _>::from_raw(width, height, pixels)
                        .context("failed to construct 16-bit RGB pattern image")?,
                ))
            }
            (_, depth) => bail!("unsupported pattern depth {depth}"),
        }
    }
}

impl PatternChannel {
    fn decode(&self) -> Result<Vec<u8>> {
        let width = self
            .right
            .checked_sub(self.left)
            .context("pattern channel width underflows u32")?;
        let height = self
            .bottom
            .checked_sub(self.top)
            .context("pattern channel height underflows u32")?;
        ensure!(
            width > 0 && height > 0,
            "invalid pattern channel bounds ({}, {}, {}, {})",
            self.top,
            self.left,
            self.bottom,
            self.right
        );

        let bytes_per_pixel = match self.depth {
            8 => 1,
            16 => 2,
            depth => bail!("unsupported pattern channel depth {depth}"),
        };
        let row_bytes = usize::try_from(width)?
            .checked_mul(bytes_per_pixel)
            .context("pattern channel row size overflows usize")?;
        let expected_len = row_bytes
            .checked_mul(usize::try_from(height)?)
            .context("pattern channel image size overflows usize")?;

        match self.compression {
            0 => {
                ensure!(
                    self.pixel_data.len() == expected_len,
                    "raw pattern channel has {} bytes, expected {expected_len}",
                    self.pixel_data.len()
                );
                Ok(self.pixel_data.clone())
            }
            1 => {
                let mut cursor = Cursor::new(&self.pixel_data);
                let decoded = rle::decode(&mut cursor, usize::try_from(height)?, row_bytes)?;
                ensure!(
                    cursor.remaining() == 0,
                    "compressed pattern channel has {} trailing bytes",
                    cursor.remaining()
                );
                Ok(decoded)
            }
            compression => bail!("unsupported pattern channel compression {compression}"),
        }
    }
}
