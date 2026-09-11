use std::{
    any::{Any, TypeId},
    collections::HashMap,
    hash::Hash,
    io::Write,
    marker::PhantomData,
    path::PathBuf,
    sync::{Arc, LazyLock},
};

use anyhow::{Result, anyhow};
use arc_swap::ArcSwap;
use async_broadcast::{InactiveReceiver, Receiver, RecvError, Sender};
use directories::BaseDirs;
use futures::stream;
use iced_futures::{
    BoxStream, Subscription,
    subscription::{EventStream, Hasher, Recipe, from_recipe},
};
use parking_lot::Mutex;
use serde::{Serialize, de::DeserializeOwned};

pub fn resolve_config_dir(name: &str) -> PathBuf {
    static BASE_DIRS: LazyLock<Option<BaseDirs>> = LazyLock::new(BaseDirs::new);

    let config_base = if let Ok(dir) = std::env::var("CONFIG_DIR") {
        PathBuf::from(dir)
    } else if let Some(dir) = BASE_DIRS.as_ref() {
        dir.config_local_dir().to_path_buf()
    } else if let Ok(dir) = std::env::current_exe() {
        dir.parent().unwrap().to_path_buf()
    } else {
        PathBuf::from(".")
    };

    config_base.join(name)
}

pub trait ConfigType: Serialize + DeserializeOwned + Clone + Send + Sync + 'static {
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

struct Shared<T: ConfigType> {
    value: ArcSwap<T>,
    writer: Mutex<()>,
    sender: Sender<()>,
    receiver: InactiveReceiver<()>,
}

impl<T: ConfigType> Shared<T> {
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

#[derive(Clone)]
pub struct Config<T: ConfigType> {
    shared: Arc<Shared<T>>,
}

impl<T: ConfigType> Config<T> {
    fn persist(value: &T) -> Result<()> {
        let path = resolve_config_dir(T::NAME);
        let parent = path
            .parent()
            .ok_or_else(|| anyhow!("config path has no parent: {}", path.display()))?;
        let content = value.unparse()?;

        std::fs::create_dir_all(parent)?;
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

        let path = resolve_config_dir(T::NAME);
        let value = match std::fs::read_to_string(&path) {
            Ok(content) => T::parse(&content)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
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
