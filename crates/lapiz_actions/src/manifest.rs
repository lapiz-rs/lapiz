use std::collections::HashMap;

use anyhow::Result;
use lapiz_config::{Config, ConfigType};
use lapiz_input::key::KeySequence;
use serde::{Deserialize, Serialize};

use crate::ActionId;

pub type MenuBarManifestConfig = Config<MenuBarManifest>;

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct MenuBarManifest {
    pub categories: Vec<MenuBarCategory>,
}

impl ConfigType for MenuBarManifest {
    const NAME: &'static str = "menu_bar_manifest.toml";

    const DEFAULT: &'static str = include_str!("../../../default_config/menu_bar_manifest.toml");
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct MenuBarCategory {
    pub title: String,
    pub items: Vec<MenuBarItem>,
}

#[derive(Debug, Clone)]
pub enum MenuBarItem {
    Separator,
    Item(ActionId),
    Submenu {
        title: String,
        items: Vec<MenuBarItem>,
    },
}

#[derive(Serialize, Deserialize)]
struct SubmenuDef {
    title: String,
    items: Vec<MenuBarItem>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum MenuBarItemDef {
    Text(String),
    Submenu(SubmenuDef),
}

impl Serialize for MenuBarItem {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            MenuBarItem::Separator => serializer.serialize_str("Separator"),
            MenuBarItem::Item(action) => action.serialize(serializer),
            MenuBarItem::Submenu { title, items } => SubmenuDef {
                title: title.clone(),
                items: items.clone(),
            }
            .serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for MenuBarItem {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Ok(match MenuBarItemDef::deserialize(deserializer)? {
            MenuBarItemDef::Text(text) => {
                if text == "Separator" {
                    MenuBarItem::Separator
                } else {
                    MenuBarItem::Item(ActionId::new(text.into()))
                }
            }
            MenuBarItemDef::Submenu(def) => MenuBarItem::Submenu {
                title: def.title,
                items: def.items,
            },
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionBinding {
    pub shortcut: KeySequence,
    pub action_name: String,
    #[serde(default)]
    #[serde(skip_serializing_if = "is_null")]
    pub action_data: serde_json::Value,
    #[serde(default)]
    #[serde(skip_serializing_if = "is_none")]
    pub context: Option<String>,
}

fn is_null(value: &serde_json::Value) -> bool {
    matches!(value, serde_json::Value::Null)
}

fn is_none<T>(value: &Option<T>) -> bool {
    value.is_none()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionBindingManifest {
    pub name: String,
    pub actions: Vec<ActionBinding>,
}

impl ConfigType for ActionBindingManifest {
    const NAME: &'static str = "action_bindings.json";

    const DEFAULT: &'static str = include_str!("../../../default_config/action_bindings.json");

    fn parse(value: &str) -> Result<Self> {
        Ok(serde_json::from_str(value)?)
    }

    fn unparse(&self) -> Result<String> {
        Ok(serde_json::to_string(self)?)
    }
}

pub type ActionBindingManifestConfig = Config<ActionBindingManifest>;

#[derive(Clone)]
pub struct ActionCollection {
    shortcuts: HashMap<KeySequence, ActionId>,
    shortcuts_by_action: HashMap<ActionId, KeySequence>,
}

impl ActionCollection {
    pub fn new(manifest: &ActionBindingManifest) -> Self {
        let mut shortcuts = HashMap::new();
        let mut shortcuts_by_action = HashMap::new();

        for def in &manifest.actions {
            let action_id = ActionId::new(def.action_name.clone().into());
            shortcuts.insert(def.shortcut, action_id.clone());
            shortcuts_by_action.insert(action_id, def.shortcut);
        }

        Self {
            shortcuts,
            shortcuts_by_action,
        }
    }

    pub fn get_action_id(&self, shortcut: KeySequence) -> Option<ActionId> {
        self.shortcuts.get(&shortcut).cloned()
    }

    pub fn shortcut_for(&self, action: &ActionId) -> Option<KeySequence> {
        self.shortcuts_by_action.get(action).cloned()
    }
}
