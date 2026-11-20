use anyhow::{Result, anyhow};
use lapiz_canvas::{CanvasAppExt as _, CanvasId};
use lapiz_image::{
    texel::TexelType,
    tile::{GpuTileStorage, TileStorageAppExt as _},
};
use lapiz_render::render_context::RenderContextAppExt as _;
use lapiz_runtime::global::GlobalHandle;

pub(crate) async fn readback_root_layer(
    globals: GlobalHandle,
    canvas: CanvasId,
    embed_profile: bool,
) -> Result<(image::RgbaImage, Option<Vec<u8>>)> {
    let (size, root_layer, profile, tile_storage, device, queue) = globals
        .update(move |globals| {
            let image = &globals
                .canvas(&canvas)
                .ok_or_else(|| anyhow!("Canvas is missing"))?
                .image;
            let profile = embed_profile
                .then(|| image.profile().encode())
                .transpose()?;
            Ok::<_, anyhow::Error>((
                image.size(),
                *image.layer_stack().root_id(),
                profile,
                globals.tile_storage().clone(),
                globals.render_device().clone(),
                globals.render_queue().clone(),
            ))
        })
        .await?;

    let layer = tile_storage
        .get_layer(root_layer)
        .ok_or_else(|| anyhow!("Root layer is missing from tile storage"))?;

    if layer.layer_info().texel_type != TexelType::RGBA8 {
        anyhow::bail!("Unsupported root layer texel type for image export");
    }

    let tile_data = layer
        .readback(&device, &queue, layer.iter_tile_indices())
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
            (&mut *result)[offset..offset + width * 4].copy_from_slice(src);
        }
    }
    Ok((result, profile))
}
