use std::{path::PathBuf, sync::LazyLock};

use anyhow::Result;
use directories::BaseDirs;
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
}

pub struct Config<T: ConfigType> {
    value: T,
}

impl<T: ConfigType> Config<T> {
    pub fn read() -> Result<Self> {
        Ok(Self {
            value: toml::from_str(&std::fs::read_to_string(resolve_config_dir(T::NAME))?)?,
        })
    }

    pub fn write(&self) -> Result<()> {
        std::fs::write(resolve_config_dir(T::NAME), toml::to_string(&self.value)?)?;
        Ok(())
    }

    pub fn update(&mut self, f: impl FnOnce(&mut T)) -> Result<()> {
        f(&mut self.value);
        self.write()
    }
}
