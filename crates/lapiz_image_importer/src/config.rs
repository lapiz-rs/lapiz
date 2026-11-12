use std::collections::HashMap;

use lapiz_config::Configuration;
use serde::{Deserialize, Serialize};

/// Persistent configuration storing per-format import settings keyed by canonical file extension.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageImporterConfig {
    #[serde(default)]
    pub importers: HashMap<String, toml::Value>,
}

impl Configuration for ImageImporterConfig {
    const NAME: &'static str = "image_importer.toml";

    const DEFAULT: &'static str = include_str!("../../../default_config/image_importer.toml");
}
