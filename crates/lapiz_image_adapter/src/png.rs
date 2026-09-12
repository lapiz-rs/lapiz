use std::path::Path;

use anyhow::Result;
use iced_core::{Element, Theme};
use iced_runtime::Task;
use image::{
    ExtendedColorType, ImageEncoder,
    codecs::png::{CompressionType, FilterType, PngEncoder},
};
use lapiz_i18n::{Translated, t};
use lapiz_runtime::{Renderer, Services};
use lapiz_widgets::{combo_box::ComboBox, form::Form};
use parse_display::Display;
use serde::{Deserialize, Serialize};

use crate::{ImageFormatAdapter, pixels};

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

#[derive(Serialize, Deserialize)]
pub struct PngAdapter {
    compression: PngCompression,
    filter: PngFilter,
}

impl Default for PngAdapter {
    fn default() -> Self {
        Self {
            compression: PngCompression::Default,
            filter: PngFilter::Adaptive,
        }
    }
}

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

#[derive(Debug, Clone, Copy)]
pub enum PngExportMessage {
    CompressionChanged(PngCompression),
    FilterChanged(PngFilter),
}

impl ImageFormatAdapter for PngAdapter {
    type ExportDialogMessage = PngExportMessage;

    fn extension() -> &'static str {
        "png"
    }

    fn description() -> String {
        t!("png_image_description")
    }

    fn export_dialog_view(&self, _: &Services) -> Element<'_, PngExportMessage, Theme, Renderer> {
        Form::new()
            .push(
                t!("compression"),
                ComboBox::new(
                    COMPRESSIONS.map(Translated).to_vec(),
                    Some(Translated(self.compression)),
                    |option| PngExportMessage::CompressionChanged(option.into_inner()),
                ),
            )
            .push(
                t!("filter"),
                ComboBox::new(
                    FILTERS.map(Translated).to_vec(),
                    Some(Translated(self.filter)),
                    |option| PngExportMessage::FilterChanged(option.into_inner()),
                ),
            )
            .into()
    }

    fn export_dialog_update(
        &mut self,
        message: PngExportMessage,
        _: &mut Services,
    ) -> Task<PngExportMessage> {
        match message {
            PngExportMessage::CompressionChanged(compression) => self.compression = compression,
            PngExportMessage::FilterChanged(filter) => self.filter = filter,
        }
        Task::none()
    }

    async fn export(&self, services: &Services, path: &Path) -> Result<()> {
        let rgba = pixels::readback_root_layer(services).await?;
        let file = std::fs::File::create(path)?;
        PngEncoder::new_with_quality(file, self.compression.into(), self.filter.into())
            .write_image(
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
