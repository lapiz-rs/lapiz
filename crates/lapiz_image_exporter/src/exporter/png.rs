use std::{fs::File, path::Path};

use anyhow::Result;
use iced_core::{Element, Theme};
use iced_runtime::Task;
use image::{
    ExtendedColorType, ImageEncoder as _,
    codecs::png::{CompressionType, FilterType, PngEncoder},
};
use lapiz_canvas::CCanvas;
use lapiz_i18n::{Translated, t};
use lapiz_image::tile::TileStorageAppExt as _;
use lapiz_render::render_context::RenderContextAppExt as _;
use lapiz_runtime::{Renderer, global::Globals};
use lapiz_widgets::{checkbox, combo_box, form};
use parse_display::Display;
use serde::{Deserialize, Serialize};

use super::pixels;
use crate::ImageFormatExporter;

const COMPRESSIONS: [PngCompression; 4] = [
    PngCompression::Default,
    PngCompression::Fast,
    PngCompression::Best,
    PngCompression::Uncompressed,
];

const FILTERS: [PngFilter; 6] = [
    PngFilter::Adaptive,
    PngFilter::NoFilter,
    PngFilter::Sub,
    PngFilter::Up,
    PngFilter::Avg,
    PngFilter::Paeth,
];

/// Exporter for the PNG (`.png`) image format with compression, filter, and ICC profile options.
#[derive(Serialize, Deserialize)]
pub struct PngExporter {
    compression: PngCompression,
    filter: PngFilter,
    #[serde(default = "crate::default_embed_profile")]
    embed_profile: bool,
}

impl Default for PngExporter {
    fn default() -> Self {
        Self {
            compression: PngCompression::Default,
            filter: PngFilter::Adaptive,
            embed_profile: true,
        }
    }
}

/// Compression level presets available when exporting a PNG image with [`PngExporter`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Display, Serialize, Deserialize)]
pub enum PngCompression {
    #[display("png_compression_default")]
    Default,
    #[display("png_compression_fast")]
    Fast,
    #[display("png_compression_best")]
    Best,
    #[display("png_compression_uncompressed")]
    Uncompressed,
}

impl From<PngCompression> for CompressionType {
    fn from(value: PngCompression) -> Self {
        match value {
            PngCompression::Default => CompressionType::Default,
            PngCompression::Fast => CompressionType::Fast,
            PngCompression::Best => CompressionType::Best,
            PngCompression::Uncompressed => CompressionType::Uncompressed,
        }
    }
}

/// Row filter strategies available when exporting a PNG image with [`PngExporter`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Display, Serialize, Deserialize)]
pub enum PngFilter {
    #[display("png_filter_adaptive")]
    Adaptive,
    #[display("png_filter_no_filter")]
    NoFilter,
    #[display("png_filter_sub")]
    Sub,
    #[display("png_filter_up")]
    Up,
    #[display("png_filter_avg")]
    Avg,
    #[display("png_filter_paeth")]
    Paeth,
}

impl From<PngFilter> for FilterType {
    fn from(value: PngFilter) -> Self {
        match value {
            PngFilter::Adaptive => FilterType::Adaptive,
            PngFilter::NoFilter => FilterType::NoFilter,
            PngFilter::Sub => FilterType::Sub,
            PngFilter::Up => FilterType::Up,
            PngFilter::Avg => FilterType::Avg,
            PngFilter::Paeth => FilterType::Paeth,
        }
    }
}

/// UI messages emitted by the [`PngExporter`] options form.
#[derive(Debug, Clone, Copy)]
pub enum PngExportMessage {
    CompressionChanged(PngCompression),
    FilterChanged(PngFilter),
    EmbedProfileToggled(bool),
}

impl ImageFormatExporter for PngExporter {
    type DialogMessage = PngExportMessage;

    fn extension() -> &'static str {
        "png"
    }

    fn description() -> String {
        t!("png_image_description")
    }

    fn dialog_view(&self, _: &Globals) -> Element<'_, PngExportMessage, Theme, Renderer> {
        form()
            .push(
                t!("compression"),
                combo_box(
                    COMPRESSIONS.map(Translated).to_vec(),
                    Some(Translated(self.compression)),
                    |option| PngExportMessage::CompressionChanged(option.into_inner()),
                ),
            )
            .push(
                t!("filter"),
                combo_box(
                    FILTERS.map(Translated).to_vec(),
                    Some(Translated(self.filter)),
                    |option| PngExportMessage::FilterChanged(option.into_inner()),
                ),
            )
            .push(
                t!("embed_icc_profile"),
                checkbox(self.embed_profile).on_toggle(PngExportMessage::EmbedProfileToggled),
            )
            .into()
    }

    fn dialog_update(
        &mut self,
        message: PngExportMessage,
        _: &mut Globals,
    ) -> Task<PngExportMessage> {
        match message {
            PngExportMessage::CompressionChanged(compression) => self.compression = compression,
            PngExportMessage::FilterChanged(filter) => self.filter = filter,
            PngExportMessage::EmbedProfileToggled(checked) => self.embed_profile = checked,
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
        let mut encoder =
            PngEncoder::new_with_quality(file, self.compression.into(), self.filter.into());
        if self.embed_profile {
            encoder.set_icc_profile(canvas.image.profile().encode()?)?;
        }
        encoder.write_image(
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
