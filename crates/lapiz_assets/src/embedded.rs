use std::{fs, io, path::Path};

use anyhow::Result;
use rust_embed::RustEmbed;

/// Embedded assets compiled into the binary via [`RustEmbed`].
#[derive(RustEmbed)]
#[folder = "../../assets"]
#[include = "builtin_assets/**"]
#[include = "*.lazurite"]
#[include = "**/*.lazurite"]
#[include = "*.abr"]
#[include = "**/*.abr"]
struct EmbeddedAssets;

pub fn extract_if_empty(root: &Path) -> Result<()> {
    match fs::read_dir(root) {
        Ok(mut entries) => {
            if entries.next().is_some() {
                return Ok(());
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }

    fs::create_dir_all(root)?;
    for path in EmbeddedAssets::iter() {
        let Some(file) = EmbeddedAssets::get(path.as_ref()) else {
            continue;
        };
        let destination = root.join(path.as_ref());
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&destination, &*file.data)?;
    }
    Ok(())
}
