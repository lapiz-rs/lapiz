use anyhow::Result;
use lapiz_config::{Config, Configuration};
use lapiz_input::key::KeySequence;
use serde::{Deserialize, Serialize};

use crate::ToolId;

/// Associates a [`ToolId`] with a keyboard [`KeySequence`] shortcut.
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

/// Serializable configuration manifest containing named [`ToolBinding`] shortcut mappings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolBindingManifest {
    pub name: String,
    pub bindings: Vec<ToolBinding>,
}

impl Configuration for ToolBindingManifest {
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

/// Serializable configuration manifest defining the layout of [`ToolBarGroup`]s in the toolbox UI.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolBoxManifest {
    pub groups: Vec<ToolBarGroup>,
}

/// A named group of [`ToolId`]s displayed together in the toolbox dock.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolBarGroup {
    pub name: String,
    pub tools: Vec<ToolId>,
}

impl Configuration for ToolBoxManifest {
    const NAME: &'static str = "tool_box_manifest.toml";

    const DEFAULT: &'static str = include_str!("../../../default_config/tool_box_manifest.toml");
}
