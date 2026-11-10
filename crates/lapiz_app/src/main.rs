use std::env;

use lapiz_runtime::Application;
#[cfg(target_os = "android")]
use winit::platform::android::activity::AndroidApp;

#[cfg(not(target_os = "android"))]
fn main() {
    run();
}

pub(crate) fn run(#[cfg(target_os = "android")] android_app: AndroidApp) {
    #[cfg(target_os = "android")]
    {
        lapiz_dirs::set_android_data_dir(
            android_app.external_data_path().expect("Android data path"),
        );
    }

    lapiz_report::setup_panic_hook();

    tracing_subscriber::fmt()
        .with_env_filter("info,wgpu_hal=warn,iced_winit=warn,iced_wgpu=warn")
        .init();

    log::info!("Running at {}", env::current_dir().unwrap().display());

    let mut app = Application::new(
        #[cfg(target_os = "android")]
        android_app,
    );

    app.add_plugin(lapiz_assets::AssetsPlugin::new(lapiz_dirs::assets_dir()))
        .add_plugin(lapiz_abr_bridge::AbrBridgePlugin)
        .add_plugin(lapiz_undo::UndoPlugin)
        .add_plugin(lapiz_render::RenderPlugin)
        .add_plugin(lapiz_shader_graph::ShaderGraphPlugin)
        .add_plugin(lapiz_effect::EffectPlugin)
        .add_plugin(lapiz_tools::ToolsPlugin)
        .add_plugin(lapiz_image::ImagePlugin)
        .add_plugin(lapiz_canvas::CanvasPlugin)
        .add_plugin(lapiz_input::InputPlugin)
        .add_plugin(lapiz_brush::BrushPlugin)
        .add_plugin(lapiz_filter::FilterPlugin)
        .add_plugin(lapiz_bucket_tool::BucketPlugin)
        .add_plugin(lapiz_eye_dropper::EyeDropperPlugin)
        .add_plugin(lapiz_selection_tool::SelectionPlugin)
        .add_plugin(lapiz_transform_tool::FreeTransformPlugin)
        .add_plugin(lapiz_color::ColorPlugin)
        .add_plugin(lapiz_actions::ActionPlugin)
        .add_plugin(lapiz_color_selector::ColorSelectorPlugin)
        .add_plugin(lapiz_builtin_docks::BuiltinDocksPlugin)
        .add_plugin(lapiz_image_importer::ImageImporterPlugin)
        .add_plugin(lapiz_image_exporter::ImageExporterPlugin)
        .add_plugin(lapiz_about::AboutPlugin)
        .add_plugin(lapiz_new_canvas_dialog::NewCanvasDialogPlugin)
        .add_plugin(lapiz_main_view::MainViewPlugin);
    app.build_plugins();

    lapiz_i18n::init();

    app.run().unwrap();
}
