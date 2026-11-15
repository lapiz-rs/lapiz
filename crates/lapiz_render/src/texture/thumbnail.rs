use std::{
    slice,
    sync::Arc,
    task::{Context, Poll, Wake, Waker},
};

use ::image::open;
use anyhow::Result;
use futures::{FutureExt as _, future::BoxFuture};
use iced_core::{
    Element, Event, Layout, Length, Rectangle, Shell, Size, Widget,
    image::Handle,
    layout,
    pointer::mouse,
    renderer,
    widget::{Operation, Tree, tree},
};
use lapiz_assets::asset::{AssetHandle, AssetId};
use lapiz_runtime::{Renderer, Theme};
use lapiz_widgets::{image, label, row, space};

use super::{Image, read_or_create_cached_thumbnail};

type ThumbnailElement<'a, Message> = Element<'a, Message, Theme, Renderer>;

/// Displays an image asset's thumbnail and name, loading them in the background when visible.
pub struct ImageThumbnail<'a, Message> {
    handle: AssetHandle<Image>,
    content: ThumbnailElement<'a, Message>,
}

impl<'a, Message: 'a> ImageThumbnail<'a, Message> {
    pub fn new(handle: AssetHandle<Image>) -> Self {
        let content = State::new(handle.id()).view();
        Self { handle, content }
    }

    fn load(handle: &AssetHandle<Image>) -> Result<(String, Handle)> {
        let name = handle.get()?.metadata.name.clone();
        let path = read_or_create_cached_thumbnail(handle)?;
        let thumbnail = open(path)?.into_rgba8();
        let handle = Handle::from_rgba(thumbnail.width(), thumbnail.height(), thumbnail.into_raw());
        Ok((name, handle))
    }
}

struct State {
    id: AssetId<Image>,
    task: Option<BoxFuture<'static, Result<(String, Handle)>>>,
    result: Option<Result<(String, Handle)>>,
}

impl State {
    fn new(id: AssetId<Image>) -> Self {
        Self {
            id,
            task: None,
            result: None,
        }
    }

    fn view<'a, Message: 'a>(&self) -> ThumbnailElement<'a, Message> {
        let thumbnail = match &self.result {
            Some(Ok((_, handle))) => {
                ThumbnailElement::<Message>::from(image(handle.clone()).width(32).height(32))
            }
            _ => ThumbnailElement::<Message>::from(space().width(32).height(32)),
        };
        let name = match &self.result {
            Some(Ok((name, _))) => name.clone(),
            Some(Err(error)) => format!("{error:#}"),
            None => self.id.to_string(),
        };
        row![thumbnail, label(name)].gap(4.0).into()
    }
}

struct ThumbnailWaker(iced_core::shell::Waker);

impl Wake for ThumbnailWaker {
    fn wake(self: Arc<Self>) {
        self.0.wake();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.0.wake();
    }
}

impl<'a, Message: 'a> Widget<Message, Theme, Renderer> for ImageThumbnail<'a, Message> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::new(self.handle.id()))
    }

    fn diff(&mut self, tree: &mut Tree) {
        let state = tree.state.downcast_mut::<State>();
        if state.id != self.handle.id() {
            *state = State::new(self.handle.id());
        }
        self.content = state.view();
        tree.diff_children(slice::from_mut(&mut self.content));
    }

    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.content
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits)
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn Operation,
    ) {
        self.content
            .as_widget_mut()
            .operate(&mut tree.children[0], layout, renderer, operation);
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        if layout.bounds().intersects(viewport) {
            let state = tree.state.downcast_mut::<State>();
            if state.task.is_none() && state.result.is_none() {
                let handle = self.handle.clone();
                state.task = Some(smol::unblock(move || Self::load(&handle)).boxed());
            }

            if let Some(task) = &mut state.task {
                let waker = Waker::from(Arc::new(ThumbnailWaker(shell.waker().clone())));
                let mut context = Context::from_waker(&waker);
                if let Poll::Ready(result) = task.as_mut().poll(&mut context) {
                    if let Err(error) = &result {
                        log::warn!("Failed to load image thumbnail {}: {error:#}", state.id);
                    }
                    state.task = None;
                    state.result = Some(result);
                    self.content = state.view();
                    tree.diff_children(slice::from_mut(&mut self.content));
                    shell.invalidate_layout();
                    shell.request_redraw();
                }
            }
        }

        self.content.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout,
            cursor,
            renderer,
            shell,
            viewport,
        );
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.content.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout,
            cursor,
            viewport,
        );
    }
}

impl<'a, Message: 'a> From<ImageThumbnail<'a, Message>> for ThumbnailElement<'a, Message> {
    fn from(thumbnail: ImageThumbnail<'a, Message>) -> Self {
        Element::new(thumbnail)
    }
}
