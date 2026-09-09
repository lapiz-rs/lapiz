use lapiz_runtime::Services;
use lapiz_runtime::renderer::RenderContext;
use wgpu::{Device, Queue};

pub trait RenderContextAppExt {
    fn render_context(&self) -> &RenderContext;
    fn render_device(&self) -> &Device;
    fn render_queue(&self) -> &Queue;
}

impl RenderContextAppExt for Services {
    fn render_context(&self) -> &RenderContext {
        self.service::<RenderContext>()
    }

    fn render_device(&self) -> &Device {
        &self.render_context().device
    }

    fn render_queue(&self) -> &Queue {
        &self.render_context().queue
    }
}
