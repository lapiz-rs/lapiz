use anyhow::Result;
use glam::Vec4;
use iced_core::image::Handle;
use iced_runtime::Task;
use lapiz_assets::AssetAppExt as _;
use lapiz_brush::{
    instance::BrushPresetInstance,
    render::{
        graph::CanvasResources,
        stroke_preview::{create_stroke_preview, predefined_curve_samples},
    },
};
use lapiz_render::{
    readback::{
        create_readback_buffer_and_schedule_copy_texture, readback_buffer_raw_on_submit_async,
    },
    render_context::RenderContextAppExt as _,
    util::DevicePollExt as _,
};
use lapiz_runtime::global::Globals;

use crate::parameter::PhotoshopBrush;

pub(super) fn render(brush: &PhotoshopBrush, globals: &Globals) -> Result<Task<Result<Handle>>> {
    let preset = brush.compile()?;
    let instance = BrushPresetInstance::new(&preset, globals.assets().clone())?;
    let (width, height) = (512, 128);
    let task = create_stroke_preview(
        &instance,
        &predefined_curve_samples(width, height),
        width,
        height,
        globals,
        &CanvasResources {
            foreground_color: Vec4::ONE,
            background_color: Vec4::ZERO,
        },
    )?;
    let device = globals.render_device().clone();
    let queue = globals.render_queue().clone();

    Ok(task.then(move |texture| {
        let mut encoder = device.create_command_encoder(&Default::default());
        let staging =
            create_readback_buffer_and_schedule_copy_texture(&device, &mut encoder, &texture);
        let mut readback = Some(readback_buffer_raw_on_submit_async(
            &mut encoder,
            &staging,
            ..,
        ));
        let submission = queue.submit([encoder.finish()]);
        let device = device.clone();
        let width = texture.width();
        let height = texture.height();

        Task::future(async move { device.poll_indefinitely_for(submission) }).then(move |result| {
            if let Err(error) = result {
                return Task::done(Err(error.into()));
            }
            let readback = readback.take().expect("preview readback completes once");
            Task::future(async move {
                let bytes = readback.into_inner().await??;
                Ok(Handle::from_rgba(width, height, bytes))
            })
        })
    }))
}
