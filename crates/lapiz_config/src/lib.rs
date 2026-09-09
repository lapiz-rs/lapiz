use std::{path::PathBuf, sync::LazyLock};

use anyhow::Result;
use directories::BaseDirs;
use lapiz_utils::Deref;
use serde::{Serialize, de::DeserializeOwned};

pub fn resolve_config_dir(name: &str) -> std::path::PathBuf {
    static BASE_DIRS: LazyLock<Option<BaseDirs>> = LazyLock::new(BaseDirs::new);

    let config_base = if let Ok(d) = std::env::var("CONFIG_DIR") {
        PathBuf::from(d)
    } else if let Some(d) = BASE_DIRS.as_ref() {
        d.config_local_dir().to_path_buf()
    } else {
        std::env::current_exe().unwrap()
    };

    config_base.join(name)
}

pub trait ConfigType: Serialize + DeserializeOwned {
    const NAME: &'static str;
    const DEFAULT: &'static str;
}

#[derive(Deref)]
pub struct Config<T: ConfigType> {
    value: T,
}

impl<T: ConfigType> Default for Config<T> {
    fn default() -> Self {
        Self {
            value: toml::from_str(T::DEFAULT).unwrap(),
        }
    }
}

impl<T: ConfigType> Config<T> {
    pub fn new(value: T) -> Self {
        Self { value }
    }

    pub fn read_or_init() -> Result<Self> {
        let path = resolve_config_dir(T::NAME);
        let content = std::fs::read_to_string(&path).unwrap_or_else(|_| T::DEFAULT.to_string());

        Ok(Self {
            value: toml::from_str(&content)?,
        })
    }

    pub fn write(&self) -> Result<()> {
        let path = resolve_config_dir(T::NAME);
        std::fs::create_dir_all(path.parent().unwrap())?;
        std::fs::write(&path, toml::to_string(&self.value)?)?;
        Ok(())
    }

    pub fn update(&mut self, f: impl FnOnce(&mut T)) -> Result<()> {
        f(&mut self.value);
        self.write()
    }
}
