use std::fs;

use chrono::Utc;
use iced_runtime::Task;
use lapiz_dirs::reports_dir;
use lapiz_render::render_context::RenderContextAppExt as _;
use lapiz_runtime::{
    global::Globals,
    windows::{OpenWindowViewCommand, WindowCommandBuffer, WindowViewId},
};
use lapiz_utils::log_err::LogErr as _;

use crate::{ActionFunction, ActionId};

/// Action that opens the application "About" window view.
#[derive(Default)]
pub struct OpenAboutAction;

impl ActionFunction for OpenAboutAction {
    type Message = ();

    fn id(&self) -> ActionId {
        ActionId::new("open_about_action".into())
    }

    fn trigger(&self, globals: &mut Globals) -> Task<Self::Message> {
        globals
            .global_mut::<WindowCommandBuffer>()
            .push(OpenWindowViewCommand::new(WindowViewId::new(
                lapiz_about::ABOUT_VIEW_ID,
            )));
        Task::none()
    }
}

/// Action that generates a system and WGPU allocator diagnostic report and writes it to disk.
#[derive(Default)]
pub struct GenerateReportAction;

impl ActionFunction for GenerateReportAction {
    type Message = ();

    fn id(&self) -> ActionId {
        ActionId("generate_report_action".into())
    }

    fn trigger(&self, globals: &mut Globals) -> Task<Self::Message> {
        let Ok(report) = lapiz_report::report(globals.render_device()).logged_err() else {
            return Task::none();
        };

        log::info!("{}", report);

        let report_path = reports_dir().join(format!(
            "report-{}.txt",
            Utc::now().to_rfc3339().replace(':', "-")
        ));
        if fs::write(&report_path, report).logged_err().is_err() {
            return Task::none();
        }

        log::info!("Report saved to {}", report_path.display());

        Task::none()
    }
}

/// Action that intentionally triggers a panic to test crash reporting.
#[derive(Default)]
pub struct DebugManualPanicAction;

impl ActionFunction for DebugManualPanicAction {
    type Message = ();

    fn id(&self) -> ActionId {
        ActionId("debug_manual_panic_action".into())
    }

    fn trigger(&self, _globals: &mut Globals) -> Task<Self::Message> {
        panic!("Debug panic");
    }
}
