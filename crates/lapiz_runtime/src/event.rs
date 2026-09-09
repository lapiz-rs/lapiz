use std::{any::TypeId, sync::LazyLock};

use futures::stream;
use iced_futures::Subscription;
use smol::channel::{Receiver, Sender};

#[doc(hidden)]
pub mod __private {
    pub use std::sync::LazyLock;

    pub use smol::channel::{Receiver, Sender, unbounded};
}

pub use lapiz_runtime_derive::Event;

pub trait Event: Send + Sync + Clone + 'static + Sized {
    fn channel() -> &'static LazyLock<(Sender<Self>, Receiver<Self>)>;

    fn broadcast(event: Self) {
        Self::channel().0.send_blocking(event).unwrap();
    }

    fn listen_to() -> Subscription<Self> {
        Subscription::run_with(TypeId::of::<Self>(), |_| {
            let rx = Self::channel().1.clone();
            stream::unfold(
                rx,
                |rx| async move { rx.recv().await.ok().map(|e| (e, rx)) },
            )
        })
    }
}
