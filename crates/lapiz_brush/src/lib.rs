use std::sync::Arc;

use futures::StreamExt;
use iced_futures::{Executor, backend::default};
use iced_runtime::{Action, Task, task::into_stream};
use lapiz_assets::AssetAppExt;
use lapiz_render::texture::Image;
use lapiz_runtime::{Application, plugin::Plugin};
use lapiz_shader_graph::{
    graph::{
        function::{ASSET_GRAPH_FUNCTION_STORAGE, GraphFunctionStorage},
        texture::{ASSET_GRAPH_TEXTURE_STORAGE, GraphTextureStorage},
    },
    save::SerializableGraphFunction,
};
use lapiz_tools::ToolsAppExt;

use crate::{
    asset::{BrushPreset, BrushPresetSerializer},
    render::stroke_preview::load_cached_stroke_preview_or_generate,
    tool::BrushTool,
};

pub mod asset;
pub mod editor;
pub mod input_processing;
pub mod instance;
pub mod render;
pub mod tool;
pub mod widget;

lapiz_i18n::define_i18n!("brush");

pub struct BrushPlugin;

impl Plugin for BrushPlugin {
    fn build(&self, app: &mut Application) {
        crate::i18n::init();
        let mut runtime = app.runtime_mut();
        let services = runtime.services_mut();
        services.add_asset_serializer::<BrushPresetSerializer>();
        services.add_tool_function::<BrushTool>();
    }

    fn finish(&self, app: &mut Application) {
        let runtime = app.runtime();
        let services = runtime.services();
        let assets = services.assets();

        ASSET_GRAPH_TEXTURE_STORAGE.swap(Arc::new(GraphTextureStorage::new(
            assets.all_handles_of::<Image>().unwrap(),
        )));
        ASSET_GRAPH_FUNCTION_STORAGE.swap(Arc::new(GraphFunctionStorage::new(
            ASSET_GRAPH_TEXTURE_STORAGE.clone(),
            ASSET_GRAPH_FUNCTION_STORAGE.clone(),
            assets
                .all_handles_of::<SerializableGraphFunction>()
                .unwrap(),
        )));

        let brushes = assets
            .all_handles_of::<BrushPreset>()
            .expect("Failed to enumerate brush presets");

        let preview_tasks = brushes.into_iter().filter_map(|brush| {
            let brush_id = brush.id();
            match load_cached_stroke_preview_or_generate(&brush, services) {
                Ok(task) => Some(task.map(move |result| (brush_id, result))),
                Err(error) => {
                    log::error!("Failed to prepare preview for brush {brush_id}: {error:#}");
                    None
                }
            }
        });
        let task = Task::batch(preview_tasks);

        let executor = default::Executor::new().expect("Failed to create preview task executor");
        if let Some(stream) = into_stream(task) {
            executor.spawn(stream.for_each(|action| async move {
                if let Action::Output((brush_id, result)) = action
                    && let Err(error) = result
                {
                    log::error!("Failed to generate preview for brush {brush_id}: {error:#}");
                }
            }));
        }
    }
}
