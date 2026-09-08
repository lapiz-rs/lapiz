use std::borrow::Cow;
use std::collections::BTreeMap;

use i18n_embed::{
    DefaultLocalizer, DesktopLanguageRequester, I18nAssets, LanguageLoader, Localizer,
};
use parking_lot::RwLock;
use unic_langid::LanguageIdentifier;

pub use fluent_bundle::{FluentArgs, FluentValue};
pub use i18n_embed::fluent::FluentLanguageLoader;
pub use rust_embed::RustEmbed;

pub const AVAILABLE: &[(&str, &str)] = &[("en", "English"), ("zh-CN", "简体中文")];
pub const FALLBACK: &str = "en";

pub fn new_loader(domain: &str) -> FluentLanguageLoader {
    FluentLanguageLoader::new(
        domain,
        FALLBACK.parse().expect("fallback langid must parse"),
    )
}

pub fn assert_parity(loader: &FluentLanguageLoader, assets: &dyn I18nAssets) {
    fn message_ids(assets: &dyn I18nAssets, path: &str) -> Vec<String> {
        let mut ids = Vec::new();
        for file in assets.get_files(path) {
            let source =
                std::str::from_utf8(&file).unwrap_or_else(|error| panic!("{path}: {error}"));
            let resource = fluent_syntax::parser::parse(source)
                .unwrap_or_else(|(errors, _)| panic!("{path}: {errors:?}"));
            ids.extend(resource.body.iter().filter_map(|entry| match entry {
                fluent_syntax::ast::Entry::Message(message) => Some(message.id.name.to_string()),
                fluent_syntax::ast::Entry::Term(term) => Some(format!("-{}", term.id.name)),
                _ => None,
            }));
        }
        ids.sort();
        ids
    }

    let languages = loader.available_languages(assets).unwrap();
    let fallback = loader.fallback_language().clone();
    let domain = loader.domain().to_string();
    let fallback_ids = message_ids(assets, &format!("{fallback}/{domain}.ftl"));

    for language in languages {
        if language == fallback {
            continue;
        }
        assert_eq!(
            message_ids(assets, &format!("{language}/{domain}.ftl")),
            fallback_ids,
            "{language} is out of parity with {fallback}"
        );
    }
}

static REGISTRY: RwLock<
    Vec<(
        &'static FluentLanguageLoader,
        &'static (dyn I18nAssets + Send + Sync),
    )>,
> = RwLock::new(Vec::new());

pub fn register(
    loader: &'static FluentLanguageLoader,
    assets: &'static (dyn I18nAssets + Send + Sync),
) {
    REGISTRY.write().push((loader, assets));
}

pub fn init() {
    select_all(&DesktopLanguageRequester::requested_languages());
}

pub fn set_language(id: &LanguageIdentifier) {
    select_all(std::slice::from_ref(id));
}

pub fn current_language() -> LanguageIdentifier {
    REGISTRY
        .read()
        .first()
        .map(|(loader, _)| loader.current_language())
        .unwrap_or_else(|| "en".parse().unwrap())
}

fn select_all(requested: &[LanguageIdentifier]) {
    for (loader, assets) in REGISTRY.read().iter() {
        select(loader, *assets, requested);
    }
}

fn select(
    loader: &FluentLanguageLoader,
    assets: &(dyn I18nAssets + Send + Sync + 'static),
    requested: &[LanguageIdentifier],
) {
    if let Err(error) = DefaultLocalizer::new(loader, assets).select(requested) {
        log::error!("failed to select languages {requested:?}: {error}");
    }
}

pub fn load_fallback_locale(loader: &FluentLanguageLoader, assets: &dyn I18nAssets) {
    loader
        .load_fallback_language(assets)
        .expect("failed to load fallback locale");
}

pub fn t(key: &str, args: Option<&'_ FluentArgs<'_>>) -> String {
    for (loader, _) in REGISTRY.read().iter() {
        if loader.has(key) {
            return loader.get_args_fluent(key, args);
        }
    }
    log::warn!("missing i18n message {key:?} across all registered domains");
    key.to_string()
}

pub struct MemoryAssets {
    files: BTreeMap<String, Vec<u8>>,
}

impl MemoryAssets {
    pub fn new(files: impl IntoIterator<Item = (impl Into<String>, impl Into<Vec<u8>>)>) -> Self {
        Self {
            files: files
                .into_iter()
                .map(|(path, content)| (path.into(), content.into()))
                .collect(),
        }
    }
}

impl I18nAssets for MemoryAssets {
    fn get_files(&self, file_path: &str) -> Vec<Cow<'_, [u8]>> {
        self.files
            .get(file_path)
            .map(|content| vec![Cow::Owned(content.clone())])
            .unwrap_or_default()
    }

    fn filenames_iter(&self) -> Box<dyn Iterator<Item = String> + '_> {
        Box::new(self.files.keys().cloned())
    }
}

#[macro_export]
macro_rules! t {
    ($key:expr) => {
        $crate::t($key, None)
    };
    ($key:expr, $($arg_name:ident = $arg_value:expr),* $(,)?) => {
        $crate::t($key, Some({
            let mut args = $crate::FluentArgs::new();
            $(
                args.set(
                    stringify!($arg_name),
                    $crate::FluentValue::from($arg_value),
                );
            )*
            args
        }).as_ref())
    };
}

#[macro_export]
macro_rules! define_i18n {
    ($domain:literal) => {
        mod i18n {
            #[derive($crate::RustEmbed)]
            #[folder = "locales/"]
            pub struct Localizations;

            pub static LOADER: ::std::sync::LazyLock<$crate::FluentLanguageLoader> =
                ::std::sync::LazyLock::new(|| {
                    let loader = $crate::new_loader($domain);
                    $crate::load_fallback_locale(&loader, &Localizations);
                    loader
                });

            pub fn init() {
                $crate::register(&LOADER, &Localizations);
            }

            #[cfg(test)]
            mod i18n_parity_tests {
                #[test]
                fn locales_parity() {
                    $crate::assert_parity(&super::LOADER, &super::Localizations);
                }
            }
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::LazyLock;

    #[test]
    fn available_ids_parse() {
        for (id, _) in AVAILABLE {
            id.parse::<LanguageIdentifier>()
                .unwrap_or_else(|error| panic!("{id}: {error}"));
        }
    }

    #[test]
    fn set_language_roundtrip() {
        static ASSETS: LazyLock<MemoryAssets> = LazyLock::new(|| {
            MemoryAssets::new([
                ("en/test.ftl", "welcome = hello".as_bytes().to_vec()),
                ("zh-CN/test.ftl", "welcome = 你好".as_bytes().to_vec()),
            ])
        });
        static TEST_LOADER: LazyLock<FluentLanguageLoader> =
            LazyLock::new(|| FluentLanguageLoader::new("test", "en".parse().unwrap()));

        register(&TEST_LOADER, &*ASSETS);

        let zh_cn: LanguageIdentifier = "zh-CN".parse().unwrap();
        set_language(&zh_cn);
        assert_eq!(current_language(), zh_cn);

        let en: LanguageIdentifier = "en".parse().unwrap();
        set_language(&en);
        assert_eq!(current_language(), en);
    }

    #[test]
    fn lookup_hits_registered_domains_and_falls_back() {
        static ASSETS: LazyLock<MemoryAssets> = LazyLock::new(|| {
            MemoryAssets::new([
                ("en/lookup.ftl", "greet = hi".as_bytes().to_vec()),
                // zh must exist too: current_language() reads the first registered
                // loader, and test registration order is not deterministic.
                ("zh-CN/lookup.ftl", "greet = 你好".as_bytes().to_vec()),
            ])
        });
        static LOOKUP_LOADER: LazyLock<FluentLanguageLoader> =
            LazyLock::new(|| FluentLanguageLoader::new("lookup_test", "en".parse().unwrap()));

        register(&LOOKUP_LOADER, &*ASSETS);

        assert!(matches!(t("greet", None).as_str(), "hi" | "你好"));
        assert_eq!(t("no-such-key", None), "no-such-key");
    }
}
