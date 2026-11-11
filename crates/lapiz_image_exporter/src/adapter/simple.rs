use std::{io::BufWriter, path::Path};

use anyhow::Result;
use iced_core::{Element, Theme};
use iced_runtime::Task;
use image::{DynamicImage, ImageFormat};
use lapiz_canvas::CCanvas;
use lapiz_image::tile::TileStorageAppExt as _;
use lapiz_render::render_context::RenderContextAppExt as _;
use lapiz_runtime::{Renderer, global::Globals};
use lapiz_widgets::column;

use super::pixels;
use crate::ImageFormatExporter;

macro_rules! simple_exporters {
    ($($name:ident($extension:literal, [$($alias:literal),*], $format:expr, $description:literal))*) => {
        $(
            /// Stateless image exporter backed by the [`image`] crate.
            #[derive(Default)]
            pub struct $name;

            impl ImageFormatExporter for $name {
                type DialogMessage = ();

                fn extension() -> &'static str {
                    $extension
                }

                fn aliases() -> &'static [&'static str] {
                    &[$($alias),*]
                }

                fn description() -> String {
                    lapiz_i18n::t!($description)
                }

                fn has_options() -> bool {
                    false
                }

                fn dialog_view(&self, _: &Globals) -> Element<'_, (), Theme, Renderer> {
                    column!().into()
                }

                fn dialog_update(&mut self, _: (), _: &mut Globals) -> Task<()> {
                    Task::none()
                }

                #[tracing::instrument(skip_all)]
                async fn export(
                    &self,
                    globals: &Globals,
                    canvas: &CCanvas,
                    path: &Path,
                ) -> Result<()> {
                    let rgba = pixels::readback_root_layer(
                        canvas,
                        globals.tile_storage(),
                        globals.render_device(),
                        globals.render_queue(),
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

simple_exporters! {
    WebPExporter("webp", [], ImageFormat::WebP, "webp_image_description")
    GifExporter("gif", [], ImageFormat::Gif, "gif_image_description")
    BmpExporter("bmp", [], ImageFormat::Bmp, "bmp_image_description")
    TiffExporter("tiff", ["tif"], ImageFormat::Tiff, "tiff_image_description")
    TgaExporter("tga", [], ImageFormat::Tga, "tga_image_description")
    QoiExporter("qoi", [], ImageFormat::Qoi, "qoi_image_description")
    FarbfeldExporter("ff", [], ImageFormat::Farbfeld, "farbfeld_image_description")
    IcoExporter("ico", [], ImageFormat::Ico, "ico_image_description")
    HdrExporter("hdr", [], ImageFormat::Hdr, "hdr_image_description")
    OpenExrExporter("exr", [], ImageFormat::OpenExr, "openexr_image_description")
    PnmExporter("pnm", ["pam"], ImageFormat::Pnm, "pnm_image_description")
}
