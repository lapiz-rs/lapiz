use std::str::FromStr as _;

use lapiz_config::{Config, Configuration};
use serde::{Deserialize, Serialize, de};
use unic_langid::LanguageIdentifier;

pub type LanguageConfig = Config<Language>;

/// Persisted language preference configuration.
///
/// Stores an optional [`LanguageIdentifier`] override; when `None`, the application falls back to
/// the operating system's requested desktop languages.
#[derive(Debug, Clone)]
pub struct Language {
    pub lang: Option<LanguageIdentifier>,
}

impl Configuration for Language {
    const NAME: &'static str = "language.toml";

    const DEFAULT: &'static str = include_str!("../../../default_config/language.toml");
}

/// Helper struct for serializing and deserializing [`Language`] as an optional string tag.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct SerializableLanguage {
    lang: Option<String>,
}

impl Serialize for Language {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        SerializableLanguage {
            lang: self.lang.as_ref().map(|l| l.to_string()),
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Language {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let lang = SerializableLanguage::deserialize(deserializer)?;
        if let Some(lang) = lang.lang {
            Ok(Language {
                lang: Some(
                    LanguageIdentifier::from_str(&lang).map_err(<D::Error as de::Error>::custom)?,
                ),
            })
        } else {
            Ok(Language { lang: None })
        }
    }
}
