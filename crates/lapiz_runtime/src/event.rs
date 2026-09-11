use std::{any::TypeId, sync::LazyLock};

use async_broadcast::{InactiveReceiver, Receiver, Sender, TrySendError};
use futures::stream;
use iced_futures::Subscription;

#[doc(hidden)]
pub mod __private {
    pub use std::sync::LazyLock;

    pub use async_broadcast::{InactiveReceiver, Sender};

    pub fn event_channel<T>() -> (Sender<T>, InactiveReceiver<T>) {
        let (sender, receiver) = async_broadcast::broadcast(64);
        (sender, receiver.deactivate())
    }
}

pub use lapiz_runtime_derive::Event;

pub trait Event: Send + Sync + Clone + 'static + Sized {
    fn channel() -> &'static LazyLock<(Sender<Self>, InactiveReceiver<Self>)>;

    fn broadcast(event: Self) {
        let mut sender = Self::channel().0.clone();
        let mut event = event;

        loop {
            match sender.try_broadcast(event) {
                Ok(_) | Err(TrySendError::Inactive(_)) => return,
                Err(TrySendError::Full(returned)) => {
                    event = returned;
                    sender.set_capacity(sender.capacity().saturating_mul(2));
                }
                Err(TrySendError::Closed(_)) => unreachable!("event channel must remain open"),
            }
        }
    }

    fn listen_to() -> Subscription<Self> {
        Subscription::run_with(TypeId::of::<Self>(), |_| {
            let receiver = Self::channel().1.activate_cloned();
            stream::unfold(receiver, |mut receiver: Receiver<Self>| async move {
                receiver.recv().await.ok().map(|event| (event, receiver))
            })
        })
    }
}
