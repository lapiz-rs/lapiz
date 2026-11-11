use std::{
    any::{Any, TypeId},
    collections::HashMap,
    fs,
    hash::Hash as _,
    io::{self, Write as _},
    marker::PhantomData,
    sync::{Arc, LazyLock},
};

use anyhow::{Result, anyhow};
use arc_swap::ArcSwap;
use async_broadcast::{InactiveReceiver, Receiver, RecvError, Sender};
use futures::stream;
use iced_futures::{
    BoxStream, Subscription,
    subscription::{EventStream, Hasher, Recipe, from_recipe},
};
use lapiz_dirs::config_dir;
use parking_lot::Mutex;
use serde::{Serialize, de::DeserializeOwned};

/// A typed configuration document that can be loaded from and persisted to the configuration directory.
///
/// Types implementing `Configuration` define a file [`NAME`](Configuration::NAME) relative to
/// [`config_dir`] and a built-in [`DEFAULT`](Configuration::DEFAULT) string payload used to
/// initialize the file when it does not yet exist on disk.
///
/// By default, configurations are serialized and deserialized as TOML via [`parse`](Configuration::parse)
/// and [`unparse`](Configuration::unparse), though implementations may override these methods to use
/// alternative formats such as JSON.
pub trait Configuration: Serialize + DeserializeOwned + Clone + Send + Sync + 'static {
    const NAME: &'static str;

    const DEFAULT: &'static str;

    fn parse(value: &str) -> Result<Self> {
        Ok(toml::from_str(value)?)
    }

    fn unparse(&self) -> Result<String> {
        Ok(toml::to_string(self)?)
    }
}

type ErasedShared = Arc<dyn Any + Send + Sync>;

static REGISTRY: LazyLock<Mutex<HashMap<TypeId, ErasedShared>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

struct Shared<T: Configuration> {
    value: ArcSwap<T>,
    writer: Mutex<()>,
    sender: Sender<()>,
    receiver: InactiveReceiver<()>,
}

impl<T: Configuration> Shared<T> {
    fn new(value: T) -> Self {
        let (mut sender, receiver) = async_broadcast::broadcast(1);
        sender.set_overflow(true);
        sender.set_await_active(false);

        Self {
            value: ArcSwap::from_pointee(value),
            writer: Mutex::new(()),
            sender,
            receiver: receiver.deactivate(),
        }
    }
}

/// Thread-safe handle to a shared, atomically persisted [`Configuration`] instance.
///
/// `Config<T>` manages a process-wide singleton per configuration type `T`. It provides lock-free
/// reads via [`get`](Config::get), synchronized mutations with atomic temporary-file persistence
/// via [`update`](Config::update), and reactive change notifications via [`listen_to`](Config::listen_to).
///
/// # Initialization
///
/// - [`read_or_init`](Config::read_or_init): Loads the configuration from disk, or parses and persists
///   [`Configuration::DEFAULT`] if the file is missing, caching the shared state for subsequent callers.
/// - [`fallback`](Config::fallback): Creates an uncached in-memory instance from [`Configuration::DEFAULT`].
/// - [`read_or_init_or_fallback`](Config::read_or_init_or_fallback): Attempts [`read_or_init`](Config::read_or_init)
///   and falls back to [`fallback`](Config::fallback) on I/O or parse errors.
#[derive(Clone)]
pub struct Config<T: Configuration> {
    shared: Arc<Shared<T>>,
}

impl<T: Configuration> Config<T> {
    fn persist(value: &T) -> Result<()> {
        let path = config_dir().join(T::NAME);
        let parent = path
            .parent()
            .ok_or_else(|| anyhow!("config path has no parent: {}", path.display()))?;
        let content = value.unparse()?;

        fs::create_dir_all(parent)?;
        let mut temp = tempfile::NamedTempFile::new_in(parent)?;
        temp.write_all(content.as_bytes())?;
        temp.as_file().sync_all()?;
        temp.persist(&path)?;
        Ok(())
    }

    pub fn read_or_init() -> Result<Self> {
        let id = TypeId::of::<T>();
        let mut registry = REGISTRY.lock();

        if let Some(erased) = registry.get(&id) {
            return Ok(Self {
                shared: erased.clone().downcast().unwrap(),
            });
        }

        let path = config_dir().join(T::NAME);
        let value = match fs::read_to_string(&path) {
            Ok(content) => T::parse(&content)?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let value = T::parse(T::DEFAULT)?;
                Self::persist(&value)?;
                value
            }
            Err(error) => return Err(error.into()),
        };
        let shared = Arc::new(Shared::new(value));
        registry.insert(id, shared.clone());

        Ok(Self { shared })
    }

    pub fn fallback() -> Self {
        Self {
            shared: Arc::new(Shared::new(
                T::parse(T::DEFAULT).expect("default config must parse"),
            )),
        }
    }

    pub fn read_or_init_or_fallback() -> Self {
        Self::read_or_init().unwrap_or_else(|_| Self::fallback())
    }

    pub fn get(&self) -> Arc<T> {
        self.shared.value.load_full()
    }

    pub fn update(&self, f: impl FnOnce(&mut T)) -> Result<()> {
        let _writer = self.shared.writer.lock();
        let mut value = self.shared.value.load().as_ref().clone();
        f(&mut value);
        Self::persist(&value)?;
        self.shared.value.store(Arc::new(value));
        let _ = self.shared.sender.try_broadcast(());
        Ok(())
    }

    pub fn listen_to(&self) -> Subscription<()> {
        from_recipe(ConfigUpdated::<T> {
            receiver: self.shared.receiver.activate_cloned(),
            _config: PhantomData,
        })
    }
}

/// Subscription recipe that emits an event whenever a [`Config<T>`] is updated.
struct ConfigUpdated<T> {
    receiver: Receiver<()>,
    _config: PhantomData<fn() -> T>,
}

impl<T: 'static> Recipe for ConfigUpdated<T> {
    type Output = ();

    fn hash(&self, state: &mut Hasher) {
        TypeId::of::<T>().hash(state);
    }

    fn stream(self: Box<Self>, _input: EventStream) -> BoxStream<()> {
        Box::pin(stream::unfold(self.receiver, |mut receiver| async move {
            match receiver.recv().await {
                Ok(()) | Err(RecvError::Overflowed(_)) => Some(((), receiver)),
                Err(RecvError::Closed) => None,
            }
        }))
    }
}
