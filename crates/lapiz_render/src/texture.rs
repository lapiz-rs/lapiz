use std::{
    fs,
    io::{self, Cursor, Read, Write},
    path::PathBuf,
};

use anyhow::Result;
use iced_core::{Element, image::Handle};
use image::{DynamicImage, ImageFormat, imageops::FilterType};
use lapiz_assets::{
    asset::{Asset, AssetHandle, AssetId},
    loader::AssetSerializer,
};
use lapiz_dirs::cache_dir;
use lapiz_runtime::{Renderer, Theme};
use lapiz_widgets::{label, row};
use serde::{Deserialize, Serialize};
use toml::de;
use wgpu::{
    Device, Extent3d, Queue, Texture, TextureDimension, TextureFormat, TextureUsages,
    util::DeviceExt as _,
    wgt::{TextureDataOrder, TextureDescriptor},
};
use zip::{ZipArchive, result::ZipError};

pub fn image_thumbnail_path(id: &AssetId<Image>) -> PathBuf {
    cache_dir()
        .join("image_thumbnail")
        .join(format!("{}.png", id))
}

pub fn read_or_create_cached_thumbnail(handle: &AssetHandle<Image>) -> Result<PathBuf> {
    let path = image_thumbnail_path(&handle.id());
    if path.exists() {
        return Ok(path);
    }

    let img = handle.get()?;
    let thumbnail = img.image.resize_to_fill(64, 64, FilterType::Lanczos3);
    fs::create_dir_all(
        path.parent()
            .expect("thumbnail path has a parent directory"),
    )?;
    thumbnail.save_with_format(&path, ImageFormat::Png)?;

    Ok(path)
}

pub struct ImageListItem {
    handle: AssetHandle<Image>,
}

impl ImageListItem {
    pub fn new(handle: AssetHandle<Image>) -> Self {
        Self { handle }
    }
}

impl<'a, Message: 'a> From<ImageListItem> for Element<'a, Message, Theme, Renderer> {
    fn from(value: ImageListItem) -> Self {
        let path = read_or_create_cached_thumbnail(&value.handle).unwrap_or_else(|error| {
            log::warn!(
                "Failed to create thumbnail for image {}: {error}",
                value.handle.id()
            );
            image_thumbnail_path(&value.handle.id())
        });
        row![
            lapiz_widgets::image(Handle::from_path(path))
                .width(32)
                .height(32),
            label(
                value
                    .handle
                    .get()
                    .map(|h| h.metadata.name.clone())
                    .unwrap_or_default()
            )
        ]
        .gap(4.0)
        .into()
    }
}

pub struct Image {
    pub metadata: ImageMetadata,
    pub image: DynamicImage,
    pub format: ImageFormat,
}

#[derive(Serialize, Deserialize)]
pub struct ImageMetadata {
    pub name: String,
}

impl Asset for Image {
    const TYPE_NAME: &'static str = "texture";
}

impl Image {
    pub fn from_buffer(metadata: ImageMetadata, buffer: &[u8]) -> Result<Self, image::ImageError> {
        let format = image::guess_format(buffer)?;
        let image = image::load_from_memory_with_format(buffer, format)?;
        Ok(Self {
            metadata,
            image,
            format,
        })
    }
}

#[derive(Default)]
pub struct ImageSerializer;

#[derive(Debug, thiserror::Error)]
pub enum ImageSerializerError {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Image(#[from] image::ImageError),
    #[error(transparent)]
    Zip(#[from] ZipError),
    #[error(transparent)]
    Toml(#[from] de::Error),
}

impl AssetSerializer for ImageSerializer {
    type Asset = Image;

    fn file_extension() -> &'static str {
        "lig"
    }

    fn read(&self, reader: &mut dyn Read) -> Result<Self::Asset> {
        let mut buf = Vec::new();
        reader.read_to_end(&mut buf)?;
        let mut archive = ZipArchive::new(Cursor::new(buf))?;

        let metadata = {
            let mut buf = Vec::new();
            archive.by_name("metadata.toml")?.read_to_end(&mut buf)?;
            toml::from_slice::<ImageMetadata>(&buf)?
        };

        let (format, image) = {
            let mut buf = Vec::new();
            archive.by_name("image")?.read_to_end(&mut buf)?;
            let format = image::guess_format(&buf)?;
            let image = image::load_from_memory_with_format(&buf, format)?;
            (format, image)
        };

        Ok(Image {
            metadata,
            image,
            format,
        })
    }

    fn write(&self, _: &Self::Asset, _: &mut dyn Write) -> Result<()> {
        todo!()
    }
}

#[derive(Debug, Clone)]
pub struct GpuImage {
    pub texture: Texture,
}

impl GpuImage {
    pub fn from_asset(device: &Device, queue: &Queue, asset: &Image, usage: TextureUsages) -> Self {
        use bytemuck::cast_slice;
        let width;
        let height;

        let data: Vec<u8>;
        let format: TextureFormat;
        // TODO: This can be incorrect sometimes, but mostly we are loading srgb textures.
        let is_srgb = true;
        let dyn_img = asset.image.clone();

        // Copied from Bevy project
        match dyn_img {
            DynamicImage::ImageLuma8(image) => {
                let i = DynamicImage::ImageLuma8(image).into_rgba8();
                width = i.width();
                height = i.height();
                format = if is_srgb {
                    TextureFormat::Rgba8UnormSrgb
                } else {
                    TextureFormat::Rgba8Unorm
                };

                data = i.into_raw();
            }
            DynamicImage::ImageLumaA8(image) => {
                let i = DynamicImage::ImageLumaA8(image).into_rgba8();
                width = i.width();
                height = i.height();
                format = if is_srgb {
                    TextureFormat::Rgba8UnormSrgb
                } else {
                    TextureFormat::Rgba8Unorm
                };

                data = i.into_raw();
            }
            DynamicImage::ImageRgb8(image) => {
                let i = DynamicImage::ImageRgb8(image).into_rgba8();
                width = i.width();
                height = i.height();
                format = if is_srgb {
                    TextureFormat::Rgba8UnormSrgb
                } else {
                    TextureFormat::Rgba8Unorm
                };

                data = i.into_raw();
            }
            DynamicImage::ImageRgba8(image) => {
                width = image.width();
                height = image.height();
                format = if is_srgb {
                    TextureFormat::Rgba8UnormSrgb
                } else {
                    TextureFormat::Rgba8Unorm
                };

                data = image.into_raw();
            }
            DynamicImage::ImageLuma16(image) => {
                width = image.width();
                height = image.height();
                format = TextureFormat::R16Uint;

                let raw_data = image.into_raw();

                data = cast_slice(&raw_data).to_owned();
            }
            DynamicImage::ImageLumaA16(image) => {
                width = image.width();
                height = image.height();
                format = TextureFormat::Rg16Uint;

                let raw_data = image.into_raw();

                data = cast_slice(&raw_data).to_owned();
            }
            DynamicImage::ImageRgb16(image) => {
                let i = DynamicImage::ImageRgb16(image).into_rgba16();
                width = i.width();
                height = i.height();
                format = TextureFormat::Rgba16Unorm;

                let raw_data = i.into_raw();

                data = cast_slice(&raw_data).to_owned();
            }
            DynamicImage::ImageRgba16(image) => {
                width = image.width();
                height = image.height();
                format = TextureFormat::Rgba16Unorm;

                let raw_data = image.into_raw();

                data = cast_slice(&raw_data).to_owned();
            }
            DynamicImage::ImageRgb32F(image) => {
                width = image.width();
                height = image.height();
                format = TextureFormat::Rgba32Float;
                let pixel_size = format.block_copy_size(None).unwrap() as usize;

                let mut local_data =
                    Vec::with_capacity(width as usize * height as usize * pixel_size);

                for pixel in image.into_raw().as_chunks::<3>().0 {
                    let r = pixel[0];
                    let g = pixel[1];
                    let b = pixel[2];
                    let a = 1f32;

                    local_data.extend_from_slice(&r.to_le_bytes());
                    local_data.extend_from_slice(&g.to_le_bytes());
                    local_data.extend_from_slice(&b.to_le_bytes());
                    local_data.extend_from_slice(&a.to_le_bytes());
                }

                data = local_data;
            }
            DynamicImage::ImageRgba32F(image) => {
                width = image.width();
                height = image.height();
                format = TextureFormat::Rgba32Float;

                let raw_data = image.into_raw();

                data = cast_slice(&raw_data).to_owned();
            }
            // DynamicImage is now non exhaustive, catch future variants and convert them
            _ => {
                let image = dyn_img.into_rgba8();
                width = image.width();
                height = image.height();
                format = TextureFormat::Rgba8UnormSrgb;

                data = image.into_raw();
            }
        }

        let texture = device.create_texture_with_data(
            queue,
            &TextureDescriptor {
                label: None,
                size: Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format,
                usage: TextureUsages::COPY_DST | usage,
                view_formats: &[],
            },
            TextureDataOrder::default(),
            &data,
        );

        Self { texture }
    }
}
