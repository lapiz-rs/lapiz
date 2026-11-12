use std::collections::HashMap;

use lapiz_config::Configuration;
use serde::{Deserialize, Serialize};

/// Persistent configuration storing per-format export settings keyed by canonical file extension.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageExporterConfig {
    #[serde(default)]
    pub adapters: HashMap<String, toml::Value>,
}

impl Configuration for ImageExporterConfig {
    const NAME: &'static str = "image_exporter.toml";

    const DEFAULT: &'static str = include_str!("../../../default_config/image_exporter.toml");
}
