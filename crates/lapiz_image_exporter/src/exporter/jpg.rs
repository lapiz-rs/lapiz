use std::{fs::File, path::Path};

use anyhow::Result;
use iced_core::{Element, Theme};
use iced_runtime::Task;
use image::{ExtendedColorType, ImageEncoder as _, codecs::jpeg::JpegEncoder};
use lapiz_canvas::CCanvas;
use lapiz_i18n::t;
use lapiz_image::tile::TileStorageAppExt as _;
use lapiz_render::render_context::RenderContextAppExt as _;
use lapiz_runtime::{Renderer, global::Globals};
use lapiz_widgets::{checkbox, form, spin_slider};
use serde::{Deserialize, Serialize};

use super::pixels;
use crate::ImageFormatExporter;

/// Exporter for the JPEG (`.jpg` / `.jpeg`) image format with quality and ICC profile embedding options.
#[derive(Serialize, Deserialize)]
pub struct JpgExporter {
    quality: u8,
    #[serde(default = "crate::default_embed_profile")]
    embed_profile: bool,
}

impl Default for JpgExporter {
    fn default() -> Self {
        Self {
            quality: 90,
            embed_profile: true,
        }
    }
}

/// UI messages emitted by the [`JpgExporter`] options form.
#[derive(Debug, Clone, Copy)]
pub enum JpgExportMessage {
    QualityChanged(u8),
    EmbedProfileToggled(bool),
}

impl ImageFormatExporter for JpgExporter {
    type DialogMessage = JpgExportMessage;

    fn extension() -> &'static str {
        "jpg"
    }

    fn aliases() -> &'static [&'static str] {
        &["jpeg"]
    }

    fn description() -> String {
        t!("jpg_image_description")
    }

    fn dialog_view(&self, _: &Globals) -> Element<'_, JpgExportMessage, Theme, Renderer> {
        form()
            .push(
                t!("quality"),
                spin_slider(1..=100, self.quality)
                    .precision(0)
                    .on_confirm(JpgExportMessage::QualityChanged),
            )
            .push(
                t!("embed_icc_profile"),
                checkbox(self.embed_profile).on_toggle(JpgExportMessage::EmbedProfileToggled),
            )
            .into()
    }

    fn dialog_update(
        &mut self,
        message: JpgExportMessage,
        _: &mut Globals,
    ) -> Task<JpgExportMessage> {
        match message {
            JpgExportMessage::QualityChanged(quality) => self.quality = quality,
            JpgExportMessage::EmbedProfileToggled(checked) => self.embed_profile = checked,
        }
        Task::none()
    }

    #[tracing::instrument(skip_all)]
    async fn export(&self, globals: &Globals, canvas: &CCanvas, path: &Path) -> Result<()> {
        let rgba = pixels::readback_root_layer(
            canvas,
            globals.tile_storage(),
            globals.render_device(),
            globals.render_queue(),
        )
        .await?;
        let rgb = flatten_onto_white(&rgba);
        let file = File::create(path)?;
        let mut encoder = JpegEncoder::new_with_quality(file, self.quality);
        if self.embed_profile {
            encoder.set_icc_profile(canvas.image.profile().encode()?)?;
        }
        encoder.write_image(
            rgb.as_raw(),
            rgb.width(),
            rgb.height(),
            ExtendedColorType::Rgb8,
        )?;
        Ok(())
    }

    fn to_toml(&self) -> Result<toml::Value> {
        Ok(toml::Value::try_from(self)?)
    }

    fn from_toml(&mut self, value: toml::Value) -> Result<()> {
        *self = value.try_into()?;
        Ok(())
    }
}

// jpeg has no alpha, composite the flattened image over white
// TODO: make background color configurable
fn flatten_onto_white(rgba: &image::RgbaImage) -> image::RgbImage {
    image::ImageBuffer::from_fn(rgba.width(), rgba.height(), |x, y| {
        let pixel = rgba.get_pixel(x, y);
        let alpha = pixel[3] as u32;
        let blend =
            |channel: u8| ((channel as u32 * alpha + 255 * (255 - alpha) + 127) / 255) as u8;
        image::Rgb([blend(pixel[0]), blend(pixel[1]), blend(pixel[2])])
    })
}
