use std::path::Path;

use anyhow::Result;
use iced_core::{Element, Theme};
use iced_runtime::Task;
use lapiz_i18n::t;
use lapiz_lazuli::LazuliArchive;
use lapiz_runtime::{Renderer, global::Globals};
use lapiz_widgets::column;

use crate::ImageFormatImporter;

/// Importer that opens an existing `.lazuli` project archive via [`LazuliArchive::open`].
#[derive(Default)]
pub struct LazuliImporter;

impl ImageFormatImporter for LazuliImporter {
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
    async fn import(&self, _: &Globals, path: &Path) -> Result<LazuliArchive> {
        LazuliArchive::open(path)
    }

    fn to_toml(&self) -> Result<toml::Value> {
        Ok(toml::Value::Table(Default::default()))
    }

    fn from_toml(&mut self, _: toml::Value) -> Result<()> {
        Ok(())
    }
}
