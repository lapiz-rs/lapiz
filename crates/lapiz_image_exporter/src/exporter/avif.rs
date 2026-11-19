use std::{fs::File, path::Path};

use anyhow::Result;
use iced_core::{Element, Theme};
use iced_runtime::Task;
use image::{ExtendedColorType, ImageEncoder as _, codecs::avif::AvifEncoder};
use lapiz_canvas::CCanvas;
use lapiz_i18n::t;
use lapiz_image::tile::TileStorageAppExt as _;
use lapiz_render::render_context::RenderContextAppExt as _;
use lapiz_runtime::{Renderer, global::Globals};
use lapiz_widgets::{form, spin_slider};
use serde::{Deserialize, Serialize};

use super::pixels;
use crate::ImageFormatExporter;

// ravif speed, 1 (slowest, best compression) to 10 (fastest)
const AVIF_SPEED: u8 = 4;

/// Exporter for the AVIF (`.avif`) image format with configurable encoding quality.
#[derive(Serialize, Deserialize)]
pub struct AvifExporter {
    quality: u8,
}

impl Default for AvifExporter {
    fn default() -> Self {
        Self { quality: 80 }
    }
}

/// UI messages emitted by the [`AvifExporter`] options form.
#[derive(Debug, Clone, Copy)]
pub enum AvifExportMessage {
    QualityChanged(u8),
}

impl ImageFormatExporter for AvifExporter {
    type DialogMessage = AvifExportMessage;

    fn extension() -> &'static str {
        "avif"
    }

    fn description() -> String {
        t!("avif_image_description")
    }

    fn dialog_view(&self, _: &Globals) -> Element<'_, AvifExportMessage, Theme, Renderer> {
        form()
            .push(
                t!("quality"),
                spin_slider(1..=100, self.quality)
                    .precision(0)
                    .on_confirm(AvifExportMessage::QualityChanged),
            )
            .into()
    }

    fn dialog_update(
        &mut self,
        message: AvifExportMessage,
        _: &mut Globals,
    ) -> Task<AvifExportMessage> {
        match message {
            AvifExportMessage::QualityChanged(quality) => self.quality = quality,
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
        let file = File::create(path)?;
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
