use lapiz_runtime::{Runtime, plugin::Plugin};
use serde::Deserialize;

lapiz_i18n::define_i18n!("about");

mod view;

pub const ABOUT_VIEW_ID: &str = "about";

const GPL_LICENSE_TEXT: &str = include_str!("../../../LICENSE");
const MIT_LICENSE_TEXT: &str = include_str!("../../../LICENSES/MIT.txt");

/// Deserialized collection of third-party license definitions and crate dependencies embedded at build time.
#[derive(Debug, Deserialize)]
pub struct ThirdPartyLicenses {
    #[serde(default)]
    pub licenses: Vec<LicenseEntry>,
    #[serde(default)]
    pub crates: Vec<CrateEntry>,
}

/// A third-party software license with its identifier, display name, and full text.
#[derive(Debug, Deserialize)]
pub struct LicenseEntry {
    pub id: String,
    pub name: String,
    pub text: String,
}

/// Metadata for a third-party crate dependency, referencing its license in [`ThirdPartyLicenses::licenses`].
#[derive(Debug, Deserialize)]
pub struct CrateEntry {
    pub name: String,
    pub version: String,
    pub repository: Option<String>,
    pub license: usize,
}

impl ThirdPartyLicenses {
    fn parse_embedded() -> Self {
        serde_json::from_str(include_str!(concat!(
            env!("OUT_DIR"),
            "/third_party_licenses.json"
        )))
        .expect("embedded third party licenses must be valid JSON")
    }
}

/// Plugin that initializes localization and registers the application's about window view.
pub struct AboutPlugin;

impl Plugin for AboutPlugin {
    fn build(&self, app: &mut Runtime) {
        i18n::init();

        app.register_view::<view::AboutView>();
    }
}
