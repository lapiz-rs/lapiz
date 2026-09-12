use anyhow::{Result, anyhow};
use lapiz_canvas::{CCanvas, CanvasAppExt};
use lapiz_image::{
    texel::TexelType,
    tile::{GpuTileStorage, TileStorageAppExt},
};
use lapiz_render::render_context::RenderContextAppExt;
use lapiz_runtime::Services;
use wgpu::{Device, Queue};

pub(crate) async fn readback_root_layer(
    canvas: &CCanvas,
    tile_storage: &GpuTileStorage,
    device: &Device,
    queue: &Queue,
) -> Result<image::RgbaImage> {
    let image = &canvas.image;
    let root_layer = *image.layer_stack().root_id();
    let layer = tile_storage
        .get_layer(root_layer)
        .ok_or_else(|| anyhow!("Root layer is missing from tile storage"))?;

    if layer.layer_info().texel_type != TexelType::RGBA8 {
        anyhow::bail!("Unsupported root layer texel type for image export");
    }

    let size = image.size();
    let tile_data = layer
        .readback(device, queue, layer.iter_tile_indices().collect::<Vec<_>>())
        .await?;
    drop(layer);

    let mut result = image::RgbaImage::new(size.x, size.y);
    let tile = GpuTileStorage::TILE_SIZE as i32;
    let stride = GpuTileStorage::TILE_SIZE as usize * 4;
    for (index, data) in tile_data {
        let origin = index * tile;
        let width = (size.x as i32 - origin.x).min(tile) as usize;
        let height = (size.y as i32 - origin.y).min(tile) as usize;
        for row in 0..height {
            let src = &data[row * stride..row * stride + width * 4];
            let offset =
                ((origin.y + row as i32) as usize * size.x as usize + origin.x as usize) * 4;
            result.as_mut()[offset..offset + width * 4].copy_from_slice(src);
        }
    }
    Ok(result)
}
