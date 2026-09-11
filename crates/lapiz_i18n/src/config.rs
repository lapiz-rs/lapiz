use std::str::FromStr;

use lapiz_config::{Config, ConfigType};
use serde::{Deserialize, Serialize};
use unic_langid::LanguageIdentifier;

pub type LanguageConfig = Config<Language>;

#[derive(Debug, Clone)]
pub struct Language {
    pub lang: Option<LanguageIdentifier>,
}

impl ConfigType for Language {
    const NAME: &'static str = "language.toml";

    const DEFAULT: &'static str = include_str!("../../../default_config/language.toml");
}

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
                    LanguageIdentifier::from_str(&lang)
                        .map_err(<D::Error as serde::de::Error>::custom)?,
                ),
            })
        } else {
            Ok(Language { lang: None })
        }
    }
}
