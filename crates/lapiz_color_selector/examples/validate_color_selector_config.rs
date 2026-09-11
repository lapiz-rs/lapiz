use lapiz_color_selector::config::ColorSelectorConfigGroup;
use lapiz_config::Configuration;

fn main() {
    let source = std::fs::read_to_string("target/configs/color_selector.toml").unwrap();
    let config = ColorSelectorConfigGroup::parse(&source).unwrap();
    assert_eq!(config.configs.len(), 3);
    assert!(config.configs.iter().all(|config| config.planes.len() == 1));
}
