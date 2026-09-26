use std::path::PathBuf;

use lapiz_config::Configuration;
use lapiz_dirs::cache_dir;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use xxhash_rust::xxh3::xxh3_128;

#[derive(Clone, Serialize, Deserialize)]
pub struct RecentFiles {
    pub files: Vec<RecentFileRecord>,
}

impl RecentFiles {
    pub fn update(&mut self, path_or_uri: String, name: String) {
        if let Some(index) = self.files.iter().position(|r| r.path_or_uri == path_or_uri) {
            let mut rec = self.files.remove(index);
            rec.name = name;
            self.files.push(rec);
        } else {
            self.files.push(RecentFileRecord { path_or_uri, name });
        }
    }
}

impl Configuration for RecentFiles {
    const NAME: &'static str = "recent_files.toml";

    const DEFAULT: &'static str = "files = []";
}

#[derive(Clone, Serialize, Deserialize)]
pub struct RecentFileRecord {
    #[serde(alias = "path")]
    pub path_or_uri: String,
    #[serde(default)]
    pub name: String,
}

pub fn recent_file_thumbnail_path(source: &str) -> PathBuf {
    cache_dir().join("file_thumbnails").join(format!(
        "{}.png",
        Uuid::from_u128(xxh3_128(source.as_bytes()))
    ))
}

#[cfg(test)]
mod tests {
    use lapiz_config::Configuration as _;

    use super::RecentFiles;

    #[test]
    fn reopening_document_keeps_one_record_and_updates_its_name() {
        let mut recent = RecentFiles { files: Vec::new() };
        recent.update("content://documents/123".into(), "old.png".into());
        recent.update("content://documents/456".into(), "other.png".into());
        recent.update("content://documents/123".into(), "new.png".into());

        assert_eq!(recent.files.len(), 2);
        assert_eq!(recent.files[1].path_or_uri, "content://documents/123");
        assert_eq!(recent.files[1].name, "new.png");
    }

    #[test]
    fn legacy_paths_remain_available() {
        let recent = RecentFiles::parse("[[files]]\npath = 'C:/images/old.png'\n").unwrap();
        assert_eq!(recent.files[0].path_or_uri, "C:/images/old.png");
        assert!(recent.files[0].name.is_empty());
    }
}
