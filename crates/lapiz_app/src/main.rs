use std::{env, sync::Arc};

use self::main_view::MainView;

mod main_view;
lapiz_i18n::define_i18n!("app");

use lapiz_abr_bridge::AbrAssetBundle;
use lapiz_actions::ActionPlugin;
use lapiz_assets::{
    AssetsPlugin,
    bundle::{ErasedAssetBundle, directory::AssetDirectory, standard::StandardAssetBundle},
};
use lapiz_brush::BrushPlugin;
use lapiz_bucket_tool::BucketPlugin;
use lapiz_builtin_docks::BuiltinDocksPlugin;
use lapiz_canvas::CanvasPlugin;
use lapiz_color::ColorPlugin;
use lapiz_color_selector::ColorSelectorPlugin;
use lapiz_dirs::assets_dir;
use lapiz_eye_dropper::EyeDropperPlugin;
use lapiz_filter::FilterPlugin;
use lapiz_image::ImagePlugin;
use lapiz_image_exporter::ImageExporterPlugin;
use lapiz_image_importer::ImageImporterPlugin;
use lapiz_input::InputPlugin;
use lapiz_render::RenderPlugin;
#[cfg(target_os = "android")]
use lapiz_runtime::android;
use lapiz_runtime::{Application, renderer::global_render_context, windows::WindowCommandBuffer};
use lapiz_selection_tool::SelectionPlugin;
use lapiz_shader_graph::ShaderGraphPlugin;
use lapiz_tools::ToolsPlugin;
use lapiz_transform_tool::FreeTransformPlugin;
use lapiz_undo::UndoPlugin;
#[cfg(target_os = "android")]
use winit::platform::android::activity::AndroidApp;

#[cfg(not(target_os = "android"))]
fn main() {
    run();
}

pub(crate) fn run(#[cfg(target_os = "android")] android_app: AndroidApp) {
    #[cfg(target_os = "android")]
    {
        use std::{ffi::CString, fs, io};

        lapiz_dirs::set_android_data_dir(
            android_app.external_data_path().expect("Android data path"),
        );
        let destination = assets_dir().join("builtin_assets");
        fs::create_dir_all(&destination).unwrap();
        let manager = android_app.asset_manager();
        let entries = manager.open_dir(c"builtin_assets").expect("Builtin assets");
        for entry in entries {
            let name = entry.to_str().unwrap();
            let source = CString::new(format!("builtin_assets/{name}")).unwrap();
            let mut asset = manager.open(&source).expect("Builtin asset");
            let mut output = fs::File::create(destination.join(name)).unwrap();
            io::copy(&mut asset, &mut output).unwrap();
        }
    }

    lapiz_report::setup_panic_hook();

    tracing_subscriber::fmt()
        .with_env_filter("info,wgpu_hal=warn,iced_winit=warn,iced_wgpu=warn")
        .init();

    i18n::init();

    log::info!("Running at {}", env::current_dir().unwrap().display());

    let mut app = Application::default();
    #[cfg(target_os = "android")]
    app.runtime_mut()
        .add_service_instance(android::AndroidApp::new(android_app.clone()));
    let mut asset_bundles = Vec::<Arc<dyn ErasedAssetBundle>>::new();
    asset_bundles.push(Arc::new(
        AssetDirectory::new(assets_dir().join("builtin_assets")).unwrap(),
    ));

    {
        let (standard_bundles, errs) = StandardAssetBundle::scan_bundles(assets_dir());
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
        let (abr_bundles, errs) = AbrAssetBundle::scan_bundles(assets_dir());
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

    app.add_service_instance(global_render_context())
        .add_service::<WindowCommandBuffer>()
        .add_plugin(AssetsPlugin {
            asset_root: assets_dir().into(),
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
        .add_plugin(EyeDropperPlugin)
        .add_plugin(SelectionPlugin)
        .add_plugin(FreeTransformPlugin)
        .add_plugin(ColorPlugin)
        .add_plugin(ActionPlugin)
        .add_plugin(ColorSelectorPlugin)
        .add_plugin(BuiltinDocksPlugin)
        .add_plugin(ImageImporterPlugin)
        .add_plugin(ImageExporterPlugin);
    app.build_plugins();

    {
        let mut rt = app.runtime_mut();

        rt.window_manager_mut().set_root_view::<MainView>();
        rt.window_manager_mut().register_view::<MainView>();
    }

    lapiz_i18n::init();

    app.run(
        #[cfg(target_os = "android")]
        android_app,
    )
    .unwrap();
}
