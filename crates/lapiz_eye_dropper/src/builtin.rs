use anyhow::{Result, anyhow};
use async_trait::async_trait;
use glam::IVec2;
use lapiz_color::{
    Color,
    model::{gray::Gray, rgb::Rgb},
};
use lapiz_image::{layer::LayerId, texel::TexelFormat, tile::GpuTileStorage};
use wgpu::{Device, Queue};

use crate::{EyeDropperSampleMode, EyeDropperTarget};

#[derive(Default)]
pub struct PixelLayerEyeDropperTarget;

#[async_trait]
impl EyeDropperTarget for PixelLayerEyeDropperTarget {
    async fn sample(
        &self,
        layer_id: LayerId,
        pixel: IVec2,
        mode: EyeDropperSampleMode,
        tiles: &GpuTileStorage,
        device: &Device,
        queue: &Queue,
    ) -> Result<Color> {
        sample_pixel_storage(layer_id, pixel, mode, tiles, device, queue).await
    }
}

#[derive(Default)]
pub struct GroupLayerEyeDropperTarget;

#[async_trait]
impl EyeDropperTarget for GroupLayerEyeDropperTarget {
    async fn sample(
        &self,
        layer_id: LayerId,
        pixel: IVec2,
        mode: EyeDropperSampleMode,
        tiles: &GpuTileStorage,
        device: &Device,
        queue: &Queue,
    ) -> Result<Color> {
        sample_pixel_storage(layer_id, pixel, mode, tiles, device, queue).await
    }
}

pub async fn sample_pixel_storage(
    layer_id: LayerId,
    pixel: IVec2,
    mode: EyeDropperSampleMode,
    tiles: &GpuTileStorage,
    device: &Device,
    queue: &Queue,
) -> Result<Color> {
    let radius = match mode {
        EyeDropperSampleMode::Single => 0,
        EyeDropperSampleMode::Average { radius } => radius as i32,
    };
    let min = pixel - IVec2::splat(radius);
    let max = pixel + IVec2::splat(radius) + IVec2::ONE;
    let tile_size = IVec2::splat(GpuTileStorage::TILE_SIZE as i32);
    let min_tile = min.div_euclid(tile_size);
    let max_tile = (max - IVec2::ONE).div_euclid(tile_size);
    let requested_tiles = (min_tile.y..=max_tile.y)
        .flat_map(|y| (min_tile.x..=max_tile.x).map(move |x| IVec2::new(x, y)));

    let layer = tiles
        .get_layer(layer_id)
        .ok_or_else(|| anyhow!("Eye dropper target layer {} is unavailable", layer_id))?;
    let texel_type = layer.layer_info().texel_type;
    let buffers = layer.readback(device, queue, requested_tiles).await?;
    drop(layer);

    let mut sum = [0.0; 3];
    let mut count = 0;
    for y in min.y..max.y {
        for x in min.x..max.x {
            let position = IVec2::new(x, y);
            let tile = position.div_euclid(tile_size);
            if let Some(buffer) = buffers.get(&tile) {
                let local = position - tile * tile_size;
                let pixel_index =
                    (local.y as u32 * GpuTileStorage::TILE_SIZE + local.x as u32) as usize;
                match texel_type.format {
                    TexelFormat::Alpha => {
                        sum[0] += texel_type.get_channel_as_f32(buffer, pixel_index, 0);
                    }
                    TexelFormat::Rgba => {
                        for (channel, value) in sum.iter_mut().enumerate() {
                            *value +=
                                texel_type.get_channel_as_f32(buffer, pixel_index, channel as u32);
                        }
                    }
                }
            }
            count += 1;
        }
    }

    let scale = 1.0 / count as f32;
    Ok(match texel_type.format {
        TexelFormat::Alpha => Color::Gray(Gray::new(sum[0] * scale)),
        TexelFormat::Rgba => Color::Rgb(Rgb::new(sum[0] * scale, sum[1] * scale, sum[2] * scale)),
    })
}
