use lapiz_assets::{asset::Asset, loader::AssetSerializer};
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolBindingManifest {
    pub name: String,
    pub bindings: Vec<ToolBinding>,
}

impl Asset for ToolBindingManifest {
    const TYPE_NAME: &'static str = "tool_bindings";
}

#[derive(Default)]
pub struct ToolBindingManifestSerializer;

impl AssetSerializer for ToolBindingManifestSerializer {
    type Asset = ToolBindingManifest;

    type Error = serde_json::Error;

    fn file_extension() -> &'static str {
        "tool_bindings"
    }

    fn read(&self, reader: &mut dyn std::io::Read) -> Result<Self::Asset, Self::Error> {
        serde_json::from_reader(reader)
    }

    fn write(
        &self,
        asset: &Self::Asset,
        writer: &mut dyn std::io::Write,
    ) -> Result<(), Self::Error> {
        serde_json::to_writer(writer, asset)
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct ToolBoxManifest {
    pub groups: Vec<ToolBarGroup>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct ToolBarGroup {
    pub name: String,
    pub tools: Vec<ToolId>,
}
