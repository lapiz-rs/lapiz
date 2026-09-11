use anyhow::Result;
use imagers::DynamicImage;
use moxcms::Layout;
use num_enum::TryFromPrimitive;
use serde::{Deserialize, Serialize};
use wgpu::{TextureFormat, TextureSampleType};

pub const A8_FORMAT: TextureFormat = TextureFormat::R8Unorm;
pub const RGBA8_FORMAT: TextureFormat = TextureFormat::R32Uint;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, TryFromPrimitive)]
#[repr(u8)]
pub enum TexelFormat {
    Alpha = 0,
    Rgba = 1,
}

impl TexelFormat {
    pub fn moxcms_layout(&self) -> Layout {
        match self {
            TexelFormat::Alpha => Layout::Gray,
            TexelFormat::Rgba => Layout::Rgba,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, TryFromPrimitive)]
#[repr(u8)]
pub enum TexelDepth {
    Bit8 = 0,
}

impl TexelDepth {
    pub fn max_value(&self) -> Option<f32> {
        match self {
            TexelDepth::Bit8 => Some(255.0),
        }
    }

    pub fn get_value_as_f32(&self, tile_bytes: &[u8], index: usize) -> f32 {
        match self {
            TexelDepth::Bit8 => tile_bytes[index] as f32,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TexelType {
    pub format: TexelFormat,
    pub depth: TexelDepth,
}

impl TexelType {
    pub const RGBA8: Self = Self {
        format: TexelFormat::Rgba,
        depth: TexelDepth::Bit8,
    };

    pub const A8: Self = Self {
        format: TexelFormat::Alpha,
        depth: TexelDepth::Bit8,
    };

    pub const ALL_POSSIBLE_FORMATS: [Self; 2] = [Self::RGBA8, Self::A8];

    pub fn wgpu_format(&self) -> TextureFormat {
        match (self.format, self.depth) {
            (TexelFormat::Rgba, TexelDepth::Bit8) => RGBA8_FORMAT,
            (TexelFormat::Alpha, TexelDepth::Bit8) => A8_FORMAT,
        }
    }

    pub fn moxcms_layout(&self) -> Layout {
        self.format.moxcms_layout()
    }

    pub fn shader_def(&self) -> &'static str {
        match (self.format, self.depth) {
            (TexelFormat::Rgba, TexelDepth::Bit8) => "RGBA8",
            (TexelFormat::Alpha, TexelDepth::Bit8) => "A8",
        }
    }

    pub fn sample_type(&self) -> TextureSampleType {
        match (self.format, self.depth) {
            (TexelFormat::Rgba, TexelDepth::Bit8) => TextureSampleType::Uint,
            (TexelFormat::Alpha, TexelDepth::Bit8) => TextureSampleType::Float { filterable: true },
        }
    }

    pub fn alpha_channel_index(&self) -> u32 {
        match self.format {
            TexelFormat::Rgba => 3,
            TexelFormat::Alpha => 0,
        }
    }

    pub fn convert_image_to_wgpu(&self, img: DynamicImage) -> Vec<u8> {
        match (self.format, self.depth) {
            (TexelFormat::Rgba, TexelDepth::Bit8) => {
                let rgba8 = img.to_rgba8();
                rgba8.into_raw()
            }
            (TexelFormat::Alpha, TexelDepth::Bit8) => {
                let alpha8 = img.to_luma8();
                alpha8.into_raw()
            }
        }
    }

    pub fn get_channel_as_f32(&self, tile_bytes: &[u8], pixel_index: usize, channel: u32) -> f32 {
        let channel_count = match self.format {
            TexelFormat::Alpha => 1,
            TexelFormat::Rgba => 4,
        };
        assert!(channel < channel_count, "Texel channel should exist");

        let index = pixel_index * channel_count as usize + channel as usize;
        let value = self.depth.get_value_as_f32(tile_bytes, index);
        self.depth
            .max_value()
            .map_or(value, |max_value| value / max_value)
    }

    pub fn encode(&self) -> u8 {
        let format = self.format as u8;
        let depth = self.depth as u8;
        (format << 4) | depth
    }

    pub fn decode(value: u8) -> Result<Self> {
        let format = TexelFormat::try_from(value >> 4)?;
        let depth = TexelDepth::try_from(value & 0x0F)?;
        Ok(Self { format, depth })
    }
}

impl Serialize for TexelType {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let value = self.encode();
        serializer.serialize_u8(value)
    }
}

impl<'de> Deserialize<'de> for TexelType {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = u8::deserialize(deserializer)?;
        Self::decode(value).map_err(|e| <D::Error as serde::de::Error>::custom(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::TexelType;

    #[test]
    fn reads_normalized_rgba_channels() {
        let pixels = [0, 64, 128, 255, 255, 128, 64, 0];

        assert_eq!(TexelType::RGBA8.get_channel_as_f32(&pixels, 0, 0), 0.0);
        assert_eq!(TexelType::RGBA8.get_channel_as_f32(&pixels, 0, 3), 1.0);
        assert_eq!(
            TexelType::RGBA8.get_channel_as_f32(&pixels, 1, 2),
            64.0 / 255.0
        );
    }

    #[test]
    fn reads_normalized_alpha_channels() {
        let pixels = [0, 128, 255];

        assert_eq!(TexelType::A8.get_channel_as_f32(&pixels, 0, 0), 0.0);
        assert_eq!(
            TexelType::A8.get_channel_as_f32(&pixels, 1, 0),
            128.0 / 255.0
        );
        assert_eq!(TexelType::A8.get_channel_as_f32(&pixels, 2, 0), 1.0);
    }
}
