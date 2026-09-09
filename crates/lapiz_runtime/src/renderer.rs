use std::sync::{Arc, LazyLock};

use futures::executor::block_on;
use iced_core::backend;
use iced_graphics::compositor;
use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, WindowHandle,
};

use crate::service::Service;

static RENDER_CONTEXT: LazyLock<RenderContext> =
    LazyLock::new(|| block_on(RenderContext::request()));

// Many plugins relies on wgpu devices, but the device doesn't exist before window creation.
// We are using a workaround to first create the render context, then inject it into compositor
// and renderer.
pub fn global_render_context() -> RenderContext {
    RENDER_CONTEXT.clone()
}

#[derive(Debug)]
struct DisplayAndWindow {
    display: Arc<dyn compositor::Display>,
    window: Box<dyn compositor::Window>,
}

impl HasDisplayHandle for DisplayAndWindow {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        self.display.display_handle()
    }
}

impl HasWindowHandle for DisplayAndWindow {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        self.window.window_handle()
    }
}

#[derive(Clone)]
pub struct RenderContext {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
}

impl Service for RenderContext {}

impl RenderContext {
    pub async fn request() -> Self {
        let instance = wgpu::util::new_instance_with_webgpu_detection(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::from_env().unwrap_or(wgpu::Backends::PRIMARY),
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        })
        .await;

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: None,
                force_fallback_adapter: false,
            })
            .await
            .expect("Failed to request a render adapter");

        log::info!("Adapter limits: {:#?}", adapter.limits());
        log::info!("Adapter features: {:#?}", adapter.features());

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("lapiz render device descriptor"),
                required_features: wgpu::Features::SHADER_F16
                    | wgpu::Features::CLEAR_TEXTURE
                    | wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES,
                required_limits: adapter.limits(),
                memory_hints: wgpu::MemoryHints::MemoryUsage,
                trace: wgpu::Trace::Off,
                experimental_features: wgpu::ExperimentalFeatures::disabled(),
            })
            .await
            .expect("Failed to request the render device");

        device.on_uncaptured_error(Arc::new(|err| {
            log::error!("WGPU device error:\n{err}");
        }));
        device.set_device_lost_callback(|reason, err| {
            log::error!("WGPU device lost: {reason:?} {err}");
        });

        Self {
            instance,
            adapter,
            device,
            queue,
        }
    }
}

impl Default for RenderContext {
    fn default() -> Self {
        block_on(Self::request())
    }
}

pub struct Renderer {
    inner: iced_wgpu::Renderer,
}

impl iced_core::Renderer for Renderer {
    #[inline]
    fn start_layer(&mut self, bounds: iced_core::Rectangle) {
        self.inner.start_layer(bounds);
    }

    #[inline]
    fn end_layer(&mut self) {
        self.inner.end_layer();
    }

    #[inline]
    fn start_transformation(&mut self, transformation: iced_core::Transformation) {
        self.inner.start_transformation(transformation);
    }

    #[inline]
    fn end_transformation(&mut self) {
        self.inner.end_transformation();
    }

    #[inline]
    fn fill_quad(
        &mut self,
        quad: iced_core::renderer::Quad,
        background: impl Into<iced_core::Background>,
    ) {
        self.inner.fill_quad(quad, background);
    }

    #[inline]
    fn allocate_image(
        &self,
        handle: &iced_core::image::Handle,
        callback: impl FnOnce(Result<iced_core::image::Allocation, iced_core::image::Error>)
        + Send
        + 'static,
    ) {
        self.inner.allocate_image(handle, callback);
    }

    #[inline]
    fn hint(&mut self, scale: iced_core::renderer::Scale) {
        self.inner.hint(scale);
    }

    #[inline]
    fn scale(&self) -> Option<iced_core::renderer::Scale> {
        self.inner.scale()
    }

    #[inline]
    fn reset(&mut self, new_bounds: iced_core::Rectangle) {
        self.inner.reset(new_bounds);
    }

    #[inline]
    fn settings(&self) -> iced_core::renderer::Settings {
        self.inner.settings()
    }

    #[inline]
    fn tick(&mut self) {
        self.inner.tick();
    }
}

impl iced_core::text::Renderer for Renderer {
    type Font = <iced_wgpu::Renderer as iced_core::text::Renderer>::Font;
    type Paragraph = <iced_wgpu::Renderer as iced_core::text::Renderer>::Paragraph;
    type Editor = <iced_wgpu::Renderer as iced_core::text::Renderer>::Editor;

    const ICON_FONT: Self::Font = <iced_wgpu::Renderer as iced_core::text::Renderer>::ICON_FONT;
    const CHECKMARK_ICON: char = <iced_wgpu::Renderer as iced_core::text::Renderer>::CHECKMARK_ICON;
    const ARROW_DOWN_ICON: char =
        <iced_wgpu::Renderer as iced_core::text::Renderer>::ARROW_DOWN_ICON;
    const SCROLL_UP_ICON: char = <iced_wgpu::Renderer as iced_core::text::Renderer>::SCROLL_UP_ICON;
    const SCROLL_DOWN_ICON: char =
        <iced_wgpu::Renderer as iced_core::text::Renderer>::SCROLL_DOWN_ICON;
    const SCROLL_LEFT_ICON: char =
        <iced_wgpu::Renderer as iced_core::text::Renderer>::SCROLL_LEFT_ICON;
    const SCROLL_RIGHT_ICON: char =
        <iced_wgpu::Renderer as iced_core::text::Renderer>::SCROLL_RIGHT_ICON;
    const ICED_LOGO: char = <iced_wgpu::Renderer as iced_core::text::Renderer>::ICED_LOGO;

    #[inline]
    fn default_font(&self) -> Self::Font {
        self.inner.default_font()
    }

    #[inline]
    fn default_size(&self) -> iced_core::Pixels {
        self.inner.default_size()
    }

    #[inline]
    fn fill_paragraph(
        &mut self,
        text: &Self::Paragraph,
        position: iced_core::Point,
        color: iced_core::Color,
        clip_bounds: iced_core::Rectangle,
    ) {
        self.inner
            .fill_paragraph(text, position, color, clip_bounds)
    }

    #[inline]
    fn fill_editor(
        &mut self,
        editor: &Self::Editor,
        position: iced_core::Point,
        color: iced_core::Color,
        clip_bounds: iced_core::Rectangle,
    ) {
        self.inner.fill_editor(editor, position, color, clip_bounds)
    }

    #[inline]
    fn fill_text(
        &mut self,
        text: iced_core::text::Text<String, Self::Font>,
        position: iced_core::Point,
        color: iced_core::Color,
        clip_bounds: iced_core::Rectangle,
    ) {
        self.inner.fill_text(text, position, color, clip_bounds)
    }
}

impl iced_graphics::text::Renderer for Renderer {
    #[inline]
    fn fill_raw(&mut self, raw: iced_graphics::text::Raw) {
        self.inner.fill_raw(raw)
    }
}

impl iced_core::svg::Renderer for Renderer {
    #[inline]
    fn measure_svg(&self, handle: &iced_core::svg::Handle) -> iced_core::Size<u32> {
        self.inner.measure_svg(handle)
    }

    #[inline]
    fn draw_svg(
        &mut self,
        svg: iced_core::Svg,
        bounds: iced_core::Rectangle,
        clip_bounds: iced_core::Rectangle,
    ) {
        self.inner.draw_svg(svg, bounds, clip_bounds)
    }
}

impl iced_graphics::mesh::Renderer for Renderer {
    #[inline]
    fn draw_mesh(&mut self, mesh: iced_graphics::Mesh) {
        self.inner.draw_mesh(mesh)
    }

    #[inline]
    fn draw_mesh_cache(&mut self, cache: iced_graphics::mesh::Cache) {
        self.inner.draw_mesh_cache(cache)
    }
}

impl iced_wgpu::primitive::Renderer for Renderer {
    #[inline]
    fn draw_primitive(
        &mut self,
        bounds: iced_core::Rectangle,
        primitive: impl iced_wgpu::Primitive,
    ) {
        self.inner.draw_primitive(bounds, primitive)
    }
}

impl iced_graphics::geometry::Renderer for Renderer {
    type Geometry = <iced_wgpu::Renderer as iced_graphics::geometry::Renderer>::Geometry;
    type Frame = <iced_wgpu::Renderer as iced_graphics::geometry::Renderer>::Frame;

    #[inline]
    fn new_frame(&self, bounds: iced_core::Rectangle) -> Self::Frame {
        self.inner.new_frame(bounds)
    }

    #[inline]
    fn draw_geometry(&mut self, geometry: Self::Geometry) {
        self.inner.draw_geometry(geometry)
    }
}

impl iced_core::renderer::Headless for Renderer {
    #[inline]
    async fn new(settings: iced_core::renderer::Settings, backend: Option<&str>) -> Option<Self> {
        let inner =
            <iced_wgpu::Renderer as iced_core::renderer::Headless>::new(settings, backend).await?;
        Some(Self { inner })
    }

    #[inline]
    fn name(&self) -> String {
        self.inner.name()
    }

    #[inline]
    fn screenshot(
        &mut self,
        size: iced_core::Size<u32>,
        scale_factor: f32,
        background_color: iced_core::Color,
    ) -> Vec<u8> {
        <iced_wgpu::Renderer as iced_core::renderer::Headless>::screenshot(
            &mut self.inner,
            size,
            scale_factor,
            background_color,
        )
    }
}

pub struct Compositor {
    render_context: RenderContext,
    display: Arc<dyn compositor::Display>,
    engine: iced_wgpu::Engine,

    format: wgpu::TextureFormat,
    alpha_mode: wgpu::CompositeAlphaMode,
    settings: iced_wgpu::window::compositor::Settings,
}

impl Compositor {
    pub async fn request(
        settings: iced_wgpu::window::compositor::Settings,
        display: impl compositor::Display,
        compatible_window: impl compositor::Window,
        shell: iced_graphics::Shell,
    ) -> Result<Self, iced_wgpu::window::compositor::Error> {
        let render_context = global_render_context();
        let display: Arc<dyn compositor::Display> = Arc::new(display);

        log::info!("{settings:#?}");
        log::info!("Selected: {:#?}", render_context.adapter.get_info());

        #[allow(unsafe_code)]
        let compatible_surface = render_context
            .instance
            .create_surface(wgpu::SurfaceTarget::DisplayAndWindow(Box::new(
                DisplayAndWindow {
                    display: display.clone(),
                    window: Box::new(compatible_window),
                },
            )))
            .map_err(|_| iced_wgpu::window::compositor::Error::IncompatibleSurface)?;

        let capabilities = compatible_surface.get_capabilities(&render_context.adapter);
        let formats = capabilities.formats.iter().copied();

        log::info!("Available formats: {formats:#?}");

        const BLACKLIST: &[wgpu::TextureFormat] = &[
            wgpu::TextureFormat::Rgb10a2Unorm,
            wgpu::TextureFormat::Rgb10a2Uint,
        ];

        let mut formats = formats.filter(|format| {
            format.required_features() == wgpu::Features::empty() && !BLACKLIST.contains(format)
        });

        let format = if iced_graphics::color::GAMMA_CORRECTION {
            formats.find(wgpu::TextureFormat::is_srgb)
        } else {
            formats.find(|format| !wgpu::TextureFormat::is_srgb(format))
        }
        .or_else(|| {
            log::warn!("No preferred surface format found");
            capabilities.formats.first().copied()
        })
        .ok_or(iced_wgpu::window::compositor::Error::IncompatibleSurface)?;

        log::info!("Available alpha modes: {:#?}", capabilities.alpha_modes);

        let alpha_mode = if capabilities
            .alpha_modes
            .contains(&wgpu::CompositeAlphaMode::PreMultiplied)
        {
            wgpu::CompositeAlphaMode::PreMultiplied
        } else {
            wgpu::CompositeAlphaMode::Auto
        };

        log::info!("Selected format: {format:?} with alpha mode: {alpha_mode:?}");

        log::info!("Creating iced_wgpu engine");
        let engine = iced_wgpu::Engine::new(
            &render_context.adapter,
            render_context.device.clone(),
            render_context.queue.clone(),
            format,
            settings.antialiasing,
            shell,
        );
        log::info!("Created iced_wgpu engine");

        Ok(Self {
            render_context,
            display,
            engine,
            format,
            alpha_mode,
            settings,
        })
    }
}

impl compositor::Default for Renderer {
    type Compositor = Compositor;
}

impl iced_graphics::Compositor for Compositor {
    type Renderer = Renderer;
    type Surface = wgpu::Surface<'static>;

    async fn new(
        settings: backend::Settings,
        display: impl compositor::Display + Clone,
        compatible_window: impl compositor::Window + Clone,
        shell: iced_graphics::Shell,
    ) -> Result<Self, backend::Error> {
        if settings.backend.hardware().is_none() && !settings.backend.matches("wgpu") {
            return Err(backend::Error::GraphicsAdapterNotFound {
                backend: "wgpu",
                reason: backend::Reason::DidNotMatch {
                    preferred_backend: settings.backend,
                },
            });
        }

        let mut settings = iced_wgpu::window::compositor::Settings::from(settings);

        if let Some(backends) = wgpu::Backends::from_env() {
            settings.backends = backends;
        }

        if let Some(present_mode) = iced_wgpu::window::compositor::present_mode_from_env() {
            settings.present_mode = present_mode;
        }

        Ok(Self::request(settings, display, compatible_window, shell).await?)
    }

    fn create_renderer(&self, settings: iced_core::renderer::Settings) -> Self::Renderer {
        log::info!("Creating Lapiz renderer");
        Renderer {
            inner: iced_wgpu::Renderer::new(self.engine.clone(), settings),
        }
    }

    fn create_surface(
        &mut self,
        window: impl compositor::Window + Clone,
        width: u32,
        height: u32,
    ) -> Self::Surface {
        log::info!("Creating window surface ({width}x{height})");
        let mut surface = self
            .render_context
            .instance
            .create_surface(wgpu::SurfaceTarget::DisplayAndWindow(Box::new(
                DisplayAndWindow {
                    display: self.display.clone(),
                    window: Box::new(window),
                },
            )))
            .expect("Create surface");

        if width > 0 && height > 0 {
            self.configure_surface(&mut surface, width, height);
        }

        surface
    }

    fn configure_surface(&mut self, surface: &mut Self::Surface, width: u32, height: u32) {
        log::info!("Configuring window surface ({width}x{height})");
        surface.configure(
            &self.render_context.device,
            &wgpu::SurfaceConfiguration {
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                format: self.format,
                present_mode: self.settings.present_mode,
                width,
                height,
                alpha_mode: self.alpha_mode,
                view_formats: vec![],
                desired_maximum_frame_latency: 1,
            },
        );
    }

    fn information(&self) -> compositor::Information {
        let information = self.render_context.adapter.get_info();

        compositor::Information {
            adapter: information.name,
            backend: format!("{:?}", information.backend),
        }
    }

    fn present(
        &mut self,
        renderer: &mut Self::Renderer,
        surface: &mut Self::Surface,
        viewport: &iced_graphics::Viewport,
        background_color: iced_core::Color,
        on_pre_present: impl FnOnce(),
    ) -> Result<(), compositor::SurfaceError> {
        iced_wgpu::window::compositor::present(
            &mut renderer.inner,
            surface,
            viewport,
            background_color,
            on_pre_present,
        )
    }

    fn screenshot(
        &mut self,
        renderer: &mut Self::Renderer,
        viewport: &iced_graphics::Viewport,
        background_color: iced_core::Color,
    ) -> Vec<u8> {
        renderer.inner.screenshot(viewport, background_color)
    }
}
