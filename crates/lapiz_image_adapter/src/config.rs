use std::collections::HashMap;

use lapiz_config::Configuration;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageAdapterConfig {
    #[serde(default)]
    pub adapters: HashMap<String, toml::Value>,
}

impl Configuration for ImageAdapterConfig {
    const NAME: &'static str = "image_adapter.toml";

    const DEFAULT: &'static str = include_str!("../../../default_config/image_adapter.toml");
}
