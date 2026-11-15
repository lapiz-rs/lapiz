use std::{
    io::{Read, Write},
    path::Path,
};

use anyhow::Result;
use lapiz_assets::{asset::Asset, loader::AssetSerializer};

use crate::parameter::PhotoshopBrushPreset;

impl Asset for PhotoshopBrushPreset {
    const TYPE_NAME: &'static str = "photoshop_brush";
}

#[derive(Default)]
pub struct PhotoshopBrushSerializer;

impl AssetSerializer for PhotoshopBrushSerializer {
    type Asset = PhotoshopBrushPreset;

    fn file_extension() -> &'static str {
        "labr"
    }

    fn read(&self, _relative_path: &Path, reader: &mut dyn Read) -> Result<Self::Asset> {
        let mut source = String::new();
        reader.read_to_string(&mut source)?;
        Ok(toml::from_str(&source)?)
    }

    fn write(&self, asset: &Self::Asset, writer: &mut dyn Write) -> Result<()> {
        writer.write_all(toml::to_string_pretty(asset)?.as_bytes())?;
        Ok(())
    }
}
