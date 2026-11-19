use std::path::Path;

use anyhow::Result;
use iced_core::{Element, Theme};
use iced_runtime::Task;
use lapiz_canvas::CCanvas;
use lapiz_i18n::t;
use lapiz_lazuli::LazuliArchive;
use lapiz_runtime::{Renderer, global::Globals};
use lapiz_widgets::column;

use crate::ImageFormatExporter;

/// Exporter that saves a canvas into a `.lazuli` project archive via [`LazuliArchive`].
#[derive(Default)]
pub struct LazuliExporter;

impl ImageFormatExporter for LazuliExporter {
    type DialogMessage = ();

    fn extension() -> &'static str {
        lapiz_lazuli::EXTENSION
    }

    fn description() -> String {
        t!("lazuli_image_description")
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
    async fn export(&self, globals: &Globals, canvas: &CCanvas, path: &Path) -> Result<()> {
        let archive = LazuliArchive::new(path)?;
        canvas.image.write_archive(&archive, globals).await
    }

    fn to_toml(&self) -> Result<toml::Value> {
        Ok(toml::Value::Table(Default::default()))
    }

    fn from_toml(&mut self, _: toml::Value) -> Result<()> {
        Ok(())
    }
}
