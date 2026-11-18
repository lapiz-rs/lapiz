use std::time::Duration;

use ::image::RgbaImage;
use anyhow::Result;
use futures::{
    channel::oneshot,
    future::{AbortHandle, Abortable, BoxFuture},
};
use iced_core::{
    ContentFit, Element, Event, Layout, Length, Point, Rectangle, Shell, Size, Theme, Widget,
    image::{self, Handle, Renderer as _},
    layout,
    pointer::mouse,
    renderer,
    time::Instant,
    widget::{self, tree},
    window::RedrawRequest,
};
use iced_futures::{Executor as _, backend::native::smol::Executor};
use lapiz_assets::asset::{AssetHandle, UntypedAssetId};
use lapiz_runtime::{Renderer, global::Globals};

use crate::{
    asset::BrushPreset,
    engine::BrushEngine,
    render::stroke_preview::{
        CACHED_STROKE_PREVIEW_SIZE, generate_stroke_preview, generate_stroke_preview_compiled,
    },
};

pub struct BrushPreview<'a, E: BrushEngine> {
    handle: AssetHandle<E::Brush>,
    globals: &'a Globals,
    generation: u64,
}

impl<'a, E: BrushEngine> BrushPreview<'a, E> {
    pub fn new(handle: AssetHandle<E::Brush>, globals: &'a Globals) -> Self {
        let generation = handle
            .metadata()
            .map_or(0, |metadata| metadata.revision.into());
        Self {
            handle,
            globals,
            generation,
        }
    }

    pub fn generation(mut self, generation: u64) -> Self {
        self.generation = generation;
        self
    }
}

impl<'a, Message, E: BrushEngine> From<BrushPreview<'a, E>>
    for Element<'a, Message, Theme, Renderer>
{
    fn from(value: BrushPreview<'a, E>) -> Self {
        Element::new(Preview {
            key: (Some(value.handle.id().into_untyped()), value.generation),
            generate: Box::new(move || generate_stroke_preview::<E>(&value.handle, value.globals)),
        })
    }
}

pub struct BrushPreviewCompiled<'a> {
    brush: BrushPreset,
    asset_id: Option<UntypedAssetId>,
    globals: &'a Globals,
    generation: u64,
}

impl<'a> BrushPreviewCompiled<'a> {
    pub fn new(brush: BrushPreset, asset_id: Option<UntypedAssetId>, globals: &'a Globals) -> Self {
        Self {
            brush,
            asset_id,
            globals,
            generation: 0,
        }
    }

    pub fn generation(mut self, generation: u64) -> Self {
        self.generation = generation;
        self
    }
}

impl<'a, Message> From<BrushPreviewCompiled<'a>> for Element<'a, Message, Theme, Renderer> {
    fn from(value: BrushPreviewCompiled<'a>) -> Self {
        Element::new(Preview {
            key: (value.asset_id, value.generation),
            generate: Box::new(move || {
                generate_stroke_preview_compiled(&value.brush, value.asset_id, value.globals)
            }),
        })
    }
}

type PreviewKey = (Option<UntypedAssetId>, u64);

struct Preview<'a> {
    key: PreviewKey,
    generate: Box<dyn Fn() -> BoxFuture<'static, Result<RgbaImage>> + 'a>,
}

#[derive(Default)]
struct State {
    key: Option<PreviewKey>,
    pending: Option<oneshot::Receiver<Result<Handle>>>,
    abort: Option<AbortHandle>,
    image: Option<Handle>,
}

impl Drop for State {
    fn drop(&mut self) {
        if let Some(abort) = &self.abort {
            abort.abort();
        }
    }
}

impl<Message> Widget<Message, Theme, Renderer> for Preview<'_> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }

    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Fixed(180.0))
    }

    fn layout(
        &mut self,
        _: &mut widget::Tree,
        _: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        layout::atomic(limits, Length::Fill, Length::Fixed(180.0))
    }

    fn update(
        &mut self,
        tree: &mut widget::Tree,
        _: &Event,
        _: Layout<'_>,
        _: mouse::Cursor,
        _: &Renderer,
        shell: &mut Shell<'_, Message>,
        _: &Rectangle,
    ) {
        let state = tree.state.downcast_mut::<State>();
        if state.key != Some(self.key) {
            *state = State::default();
            state.key = Some(self.key);
            let future = (self.generate)();
            let (sender, receiver) = oneshot::channel();
            let (abort, registration) = AbortHandle::new_pair();
            Executor.spawn(async move {
                let _ = Abortable::new(
                    async move {
                        let result = future.await.map(|image| {
                            Handle::from_rgba(image.width(), image.height(), image.into_raw())
                        });
                        let _ = sender.send(result);
                    },
                    registration,
                )
                .await;
            });
            state.pending = Some(receiver);
            state.abort = Some(abort);
        }

        if let Some(receiver) = &mut state.pending {
            match receiver.try_recv() {
                Ok(Some(result)) => {
                    match result {
                        Ok(image) => state.image = Some(image),
                        Err(error) => tracing::warn!("Failed to generate brush preview: {error:#}"),
                    }
                    state.pending = None;
                    state.abort = None;
                    shell.request_redraw();
                }
                Ok(None) => shell.request_redraw_at(RedrawRequest::At(
                    Instant::now() + Duration::from_millis(16),
                )),
                Err(error) => {
                    tracing::warn!("Brush preview task was canceled: {error}");
                    state.pending = None;
                    state.abort = None;
                }
            }
        }
    }

    fn draw(
        &self,
        tree: &widget::Tree,
        renderer: &mut Renderer,
        _: &Theme,
        _: &renderer::Style,
        layout: Layout<'_>,
        _: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_ref::<State>();
        if state.key != Some(self.key) {
            return;
        }
        if let Some(handle) = &state.image {
            let bounds = layout.bounds();
            let size = ContentFit::Contain.fit(
                Size::new(
                    CACHED_STROKE_PREVIEW_SIZE.0 as f32,
                    CACHED_STROKE_PREVIEW_SIZE.1 as f32,
                ),
                bounds.size(),
            );
            let image_bounds = Rectangle::new(
                Point::new(
                    bounds.center_x() - size.width / 2.0,
                    bounds.center_y() - size.height / 2.0,
                ),
                size,
            );
            if let Some(clip) = bounds.intersection(viewport) {
                renderer.draw_image(image::Image::new(handle.clone()), image_bounds, clip);
            }
        }
    }
}
