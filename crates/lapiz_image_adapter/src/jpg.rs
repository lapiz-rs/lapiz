use std::path::Path;

use anyhow::Result;
use iced_core::{Element, Theme};
use iced_runtime::Task;
use image::{ExtendedColorType, ImageEncoder, codecs::jpeg::JpegEncoder};
use lapiz_i18n::t;
use lapiz_runtime::{Renderer, Services};
use lapiz_widgets::{form::Form, spin_slider::SpinSlider};
use serde::{Deserialize, Serialize};

use crate::{ImageFormatAdapter, pixels};

#[derive(Serialize, Deserialize)]
pub struct JpgAdapter {
    quality: u8,
}

impl Default for JpgAdapter {
    fn default() -> Self {
        Self { quality: 90 }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum JpgExportMessage {
    QualityChanged(u8),
}

impl ImageFormatAdapter for JpgAdapter {
    type ExportDialogMessage = JpgExportMessage;

    fn extension() -> &'static str {
        "jpg"
    }

    fn aliases() -> &'static [&'static str] {
        &["jpeg"]
    }

    fn description() -> String {
        t!("jpg_image_description")
    }

    fn export_dialog_view(&self, _: &Services) -> Element<'_, JpgExportMessage, Theme, Renderer> {
        Form::new()
            .push(
                t!("quality"),
                SpinSlider::new(1..=100, self.quality)
                    .precision(0)
                    .on_confirm(JpgExportMessage::QualityChanged),
            )
            .into()
    }

    fn export_dialog_update(
        &mut self,
        message: JpgExportMessage,
        _: &mut Services,
    ) -> Task<JpgExportMessage> {
        match message {
            JpgExportMessage::QualityChanged(quality) => self.quality = quality,
        }
        Task::none()
    }

    async fn export(&self, services: &Services, path: &Path) -> Result<()> {
        let rgba = pixels::readback_root_layer(services).await?;
        let rgb = flatten_onto_white(&rgba);
        let file = std::fs::File::create(path)?;
        JpegEncoder::new_with_quality(file, self.quality).write_image(
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
fn flatten_onto_white(rgba: &image::RgbaImage) -> image::RgbImage {
    image::ImageBuffer::from_fn(rgba.width(), rgba.height(), |x, y| {
        let pixel = rgba.get_pixel(x, y);
        let alpha = pixel[3] as u32;
        let blend =
            |channel: u8| ((channel as u32 * alpha + 255 * (255 - alpha) + 127) / 255) as u8;
        image::Rgb([blend(pixel[0]), blend(pixel[1]), blend(pixel[2])])
    })
}
