use std::path::Path;

use anyhow::Result;
use iced_core::{Element, Theme};
use iced_runtime::Task;
use lapiz_canvas::CCanvas;
use lapiz_i18n::t;
use lapiz_lazuli::LazuliArchive;
use lapiz_runtime::{Renderer, Services};

use crate::ImageFormatAdapter;

#[derive(Default)]
pub struct LazuliAdapter;

impl ImageFormatAdapter for LazuliAdapter {
    type ExportDialogMessage = ();

    fn extension() -> &'static str {
        lapiz_lazuli::EXTENSION
    }

    fn description() -> String {
        t!("lazuli_image_description")
    }

    fn has_export_options() -> bool {
        false
    }

    fn export_dialog_view(&self, _: &Services) -> Element<'_, (), Theme, Renderer> {
        iced_widget::Column::new().into()
    }

    fn export_dialog_update(&mut self, _: (), _: &mut Services) -> Task<()> {
        Task::none()
    }

    #[tracing::instrument(skip_all)]
    async fn export(&self, services: &Services, canvas: &CCanvas, path: &Path) -> Result<()> {
        let archive = LazuliArchive::new(path)?;
        canvas.image.write_archive(&archive, services).await
    }

    fn to_toml(&self) -> Result<toml::Value> {
        Ok(toml::Value::Table(Default::default()))
    }

    fn from_toml(&mut self, _: toml::Value) -> Result<()> {
        Ok(())
    }
}
