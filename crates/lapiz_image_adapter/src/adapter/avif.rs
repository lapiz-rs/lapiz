use std::path::Path;

use anyhow::Result;
use iced_core::{Element, Theme};
use iced_runtime::Task;
use image::{ExtendedColorType, ImageEncoder, codecs::avif::AvifEncoder};
use lapiz_canvas::CCanvas;
use lapiz_i18n::t;
use lapiz_image::tile::TileStorageAppExt;
use lapiz_render::render_context::RenderContextAppExt;
use lapiz_runtime::{Renderer, Services};
use lapiz_widgets::{form::Form, spin_slider::SpinSlider};
use serde::{Deserialize, Serialize};

use super::pixels;
use crate::ImageFormatAdapter;

// ravif speed, 1 (slowest, best compression) to 10 (fastest)
const AVIF_SPEED: u8 = 4;

#[derive(Serialize, Deserialize)]
pub struct AvifAdapter {
    quality: u8,
}

impl Default for AvifAdapter {
    fn default() -> Self {
        Self { quality: 80 }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum AvifExportMessage {
    QualityChanged(u8),
}

impl ImageFormatAdapter for AvifAdapter {
    type ExportDialogMessage = AvifExportMessage;

    fn extension() -> &'static str {
        "avif"
    }

    fn description() -> String {
        t!("avif_image_description")
    }

    fn export_dialog_view(&self, _: &Services) -> Element<'_, AvifExportMessage, Theme, Renderer> {
        Form::new()
            .push(
                t!("quality"),
                SpinSlider::new(1..=100, self.quality)
                    .precision(0)
                    .on_confirm(AvifExportMessage::QualityChanged),
            )
            .into()
    }

    fn export_dialog_update(
        &mut self,
        message: AvifExportMessage,
        _: &mut Services,
    ) -> Task<AvifExportMessage> {
        match message {
            AvifExportMessage::QualityChanged(quality) => self.quality = quality,
        }
        Task::none()
    }

    #[tracing::instrument(skip_all)]
    async fn export(&self, services: &Services, canvas: &CCanvas, path: &Path) -> Result<()> {
        let rgba = pixels::readback_root_layer(
            canvas,
            services.tile_storage(),
            services.render_device(),
            services.render_queue(),
        )
        .await?;
        let file = std::fs::File::create(path)?;
        AvifEncoder::new_with_speed_quality(file, AVIF_SPEED, self.quality).write_image(
            rgba.as_raw(),
            rgba.width(),
            rgba.height(),
            ExtendedColorType::Rgba8,
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
