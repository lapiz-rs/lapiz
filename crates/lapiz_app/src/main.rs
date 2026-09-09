use std::sync::Arc;

use crate::main_view::MainView;

mod dock;
mod main_view;
lapiz_i18n::define_i18n!("app");

use lapiz_abr_bridge::AbrAssetBundle;
use lapiz_actions::ActionPlugin;
use lapiz_assets::{
    AssetsPlugin,
    bundle::{ErasedAssetBundle, directory::AssetDirectory, standard::StandardAssetBundle},
};
use lapiz_brush::{BrushPlugin, editor::BrushEditor};
use lapiz_bucket_tool::BucketPlugin;
use lapiz_canvas::CanvasPlugin;
use lapiz_color::ColorPlugin;
use lapiz_color_selector::ColorSelectorPlugin;
use lapiz_filter::{FilterPlugin, editor::FilterEditor, panel::FilterPanel};
use lapiz_image::ImagePlugin;
use lapiz_input::InputPlugin;
use lapiz_render::RenderPlugin;
use lapiz_runtime::{Application, windows::WindowCommandBuffer};
use lapiz_selection_tool::SelectionPlugin;
use lapiz_shader_graph::ShaderGraphPlugin;
use lapiz_tools::ToolsPlugin;
use lapiz_transform_tool::FreeTransformPlugin;
use lapiz_undo::UndoPlugin;

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter("info,wgpu_hal=warn,iced_winit=warn,iced_wgpu=warn")
        .init();

    i18n::init();

    log::info!("Running at {}", std::env::current_dir().unwrap().display());

    let mut app = Application::default();
    let mut asset_bundles = Vec::<Arc<dyn ErasedAssetBundle>>::new();
    asset_bundles.push(Arc::new(
        AssetDirectory::new("assets/builtin_assets").unwrap(),
    ));

    {
        let (standard_bundles, errs) = StandardAssetBundle::scan_bundles("assets");
        log::info!(
            "Loaded {} lazurite bundles with {} errors",
            standard_bundles.len(),
            errs.len()
        );
        for err in errs {
            log::error!("Error loading asset bundle: {}", err);
        }
        for bundle in &standard_bundles {
            log::info!("Loaded asset bundle: {}", bundle.path().display());
        }
        asset_bundles.extend(
            standard_bundles
                .into_iter()
                .map(|b| Arc::new(b) as Arc<dyn ErasedAssetBundle>),
        );
    }

    {
        let (abr_bundles, errs) = AbrAssetBundle::scan_bundles("assets");
        log::info!(
            "Loaded {} abr bundles with {} errors",
            abr_bundles.len(),
            errs.len()
        );
        for err in errs {
            log::error!("Error loading ABR asset bundle: {}", err);
        }
        for bundle in &abr_bundles {
            log::info!("Loaded ABR asset bundle: {}", bundle.path().display());
        }
        asset_bundles.extend(
            abr_bundles
                .into_iter()
                .map(|b| Arc::new(b) as Arc<dyn ErasedAssetBundle>),
        );
    }

    app.add_service_instance(lapiz_runtime::renderer::global_render_context())
        .add_service::<WindowCommandBuffer>()
        .add_plugin(AssetsPlugin {
            asset_root: "assets".into(),
            bundles: asset_bundles,
        })
        .add_plugin(UndoPlugin)
        .add_plugin(RenderPlugin)
        .add_plugin(ShaderGraphPlugin)
        .add_plugin(ToolsPlugin)
        .add_plugin(ImagePlugin)
        .add_plugin(CanvasPlugin)
        .add_plugin(InputPlugin)
        .add_plugin(BrushPlugin)
        .add_plugin(FilterPlugin)
        .add_plugin(BucketPlugin)
        .add_plugin(SelectionPlugin)
        .add_plugin(FreeTransformPlugin)
        .add_plugin(ColorPlugin)
        .add_plugin(ActionPlugin)
        .add_plugin(ColorSelectorPlugin);
    app.build_plugins();

    {
        let mut rt = app.runtime_mut();

        rt.window_manager_mut().set_root_view::<MainView>();
        rt.window_manager_mut().register_view::<MainView>();
        rt.window_manager_mut().register_view::<BrushEditor>();
        rt.window_manager_mut().register_view::<FilterPanel>();
        rt.window_manager_mut().register_view::<FilterEditor>();
    }

    lapiz_i18n::init();

    app.run().unwrap();
}
