use anyhow::Result;
use lapiz_assets::{asset::Asset, loader::AssetSerializer};
use lapiz_config::{Config, ConfigType};
use lapiz_input::key::KeySequence;
use serde::{Deserialize, Serialize};

use crate::ToolId;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolBinding {
    pub tool: ToolId,
    pub shortcut: KeySequence,
    #[serde(default)]
    #[serde(skip_serializing_if = "is_false")]
    pub is_temporary: bool,
}

fn is_false(b: &bool) -> bool {
    !b
}

pub type ToolBindingManifestConfig = Config<ToolBindingManifest>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolBindingManifest {
    pub name: String,
    pub bindings: Vec<ToolBinding>,
}

impl ConfigType for ToolBindingManifest {
    const NAME: &'static str = "tool_bindings.json";

    const DEFAULT: &'static str = include_str!("../../../default_config/tool_bindings.json");

    fn parse(value: &str) -> Result<Self> {
        Ok(serde_json::from_str(value)?)
    }

    fn unparse(&self) -> Result<String> {
        Ok(serde_json::to_string(self)?)
    }
}

pub type ToolBoxManifestConfig = Config<ToolBoxManifest>;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolBoxManifest {
    pub groups: Vec<ToolBarGroup>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolBarGroup {
    pub name: String,
    pub tools: Vec<ToolId>,
}

impl ConfigType for ToolBoxManifest {
    const NAME: &'static str = "tool_box_manifest.toml";

    const DEFAULT: &'static str = include_str!("../../../default_config/tool_box_manifest.toml");
}
