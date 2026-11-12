use std::{fs::File, io::BufReader, path::Path};

use anyhow::Result;
use iced_core::{Element, Theme};
use iced_runtime::Task;
use lapiz_image::CImage;
use lapiz_lazuli::LazuliArchive;
use lapiz_runtime::{Renderer, global::Globals};
use lapiz_widgets::column;

use crate::ImageFormatImporter;

macro_rules! simple_importers {
    ($($name:ident($extension:literal, [$($alias:literal),*], $description:literal))*) => {
        $(
            /// Stateless image importer backed by [`CImage::load_image_with_profile`].
            #[derive(Default)]
            pub struct $name;

            impl ImageFormatImporter for $name {
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
                async fn import(&self, _: &Globals, path: &Path) -> Result<LazuliArchive> {
                    let (image, profile) = CImage::load_image_with_profile(BufReader::new(
                        File::open(path)?,
                    ))?;
                    CImage::image_to_lazuli(image, profile)
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

simple_importers! {
    PngImporter("png", [], "png_image_description")
    JpgImporter("jpg", ["jpeg"], "jpg_image_description")
    WebPImporter("webp", [], "webp_image_description")
    AvifImporter("avif", [], "avif_image_description")
    GifImporter("gif", [], "gif_image_description")
    BmpImporter("bmp", [], "bmp_image_description")
    TiffImporter("tiff", ["tif"], "tiff_image_description")
    TgaImporter("tga", [], "tga_image_description")
    QoiImporter("qoi", [], "qoi_image_description")
    FarbfeldImporter("ff", [], "farbfeld_image_description")
    IcoImporter("ico", [], "ico_image_description")
    HdrImporter("hdr", [], "hdr_image_description")
    OpenExrImporter("exr", [], "openexr_image_description")
    PnmImporter("pnm", ["pam"], "pnm_image_description")
}
