use std::{io::BufWriter, path::Path};

use anyhow::Result;
use iced_core::{Element, Theme};
use iced_runtime::Task;
use image::{DynamicImage, ImageFormat};
use lapiz_canvas::CCanvas;
use lapiz_image::tile::TileStorageAppExt;
use lapiz_render::render_context::RenderContextAppExt;
use lapiz_runtime::{Renderer, Services};

use super::pixels;
use crate::ImageFormatAdapter;

macro_rules! simple_adapters {
    ($($name:ident($extension:literal, [$($alias:literal),*], $format:expr, $description:literal))*) => {
        $(
            #[derive(Default)]
            pub struct $name;

            impl ImageFormatAdapter for $name {
                type ExportDialogMessage = ();

                fn extension() -> &'static str {
                    $extension
                }

                fn aliases() -> &'static [&'static str] {
                    &[$($alias),*]
                }

                fn description() -> String {
                    lapiz_i18n::t!($description)
                }

                fn has_export_options() -> bool {
                    false
                }

                fn export_dialog_view(&self, _: &Services) -> Element<'_, (), Theme, Renderer> {
                    iced_widget::Column::new().into()
                }

                fn export_dialog_update(&mut self, _: (), _: &mut Services) -> Task<()> {
                    Task::none()
                }

                #[tracing::instrument(skip_all)]
                async fn export(
                    &self,
                    services: &Services,
                    canvas: &CCanvas,
                    path: &Path,
                ) -> Result<()> {
                    let rgba = pixels::readback_root_layer(
                        canvas,
                        services.tile_storage(),
                        services.render_device(),
                        services.render_queue(),
                    )
                    .await?;
                    DynamicImage::ImageRgba8(rgba)
                        .write_to(&mut BufWriter::new(std::fs::File::create(path)?), $format)?;
                    Ok(())
                }

                fn to_toml(&self) -> Result<toml::Value> {
                    Ok(toml::Value::Table(Default::default()))
                }

                fn from_toml(&mut self, _: toml::Value) -> Result<()> {
                    Ok(())
                }
            }
        )*
    };
}

simple_adapters! {
    WebPAdapter("webp", [], ImageFormat::WebP, "webp_image_description")
    GifAdapter("gif", [], ImageFormat::Gif, "gif_image_description")
    BmpAdapter("bmp", [], ImageFormat::Bmp, "bmp_image_description")
    TiffAdapter("tiff", ["tif"], ImageFormat::Tiff, "tiff_image_description")
    TgaAdapter("tga", [], ImageFormat::Tga, "tga_image_description")
    QoiAdapter("qoi", [], ImageFormat::Qoi, "qoi_image_description")
    FarbfeldAdapter("ff", [], ImageFormat::Farbfeld, "farbfeld_image_description")
    IcoAdapter("ico", [], ImageFormat::Ico, "ico_image_description")
    HdrAdapter("hdr", [], ImageFormat::Hdr, "hdr_image_description")
    OpenExrAdapter("exr", [], ImageFormat::OpenExr, "openexr_image_description")
    PnmAdapter("pnm", ["pam"], ImageFormat::Pnm, "pnm_image_description")
}
