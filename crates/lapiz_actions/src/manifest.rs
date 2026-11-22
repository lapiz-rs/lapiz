use std::{collections::HashMap, sync::Arc};

use anyhow::{Error, Result, ensure};
use lapiz_config::{Config, Configuration};
use lapiz_input::key::KeySequence;
use lapiz_runtime::global::Global;
use serde::{Deserialize, Serialize};

use crate::{ActionId, file::NewFileAction};

pub type MenuBarManifestConfig = Config<MenuBarManifest>;

/// Configuration manifest defining the top-level categories and nested items of the menu bar.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MenuBarManifest {
    pub categories: Vec<MenuBarCategory>,
}

impl Configuration for MenuBarManifest {
    const NAME: &'static str = "menu_bar_manifest.toml";

    const DEFAULT: &'static str = include_str!("../../../default_config/menu_bar_manifest.toml");
}

/// A top-level menu category in the [`MenuBarManifest`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MenuBarCategory {
    pub title: String,
    pub items: Vec<MenuBarItem>,
}

/// An entry within a [`MenuBarCategory`] or nested submenu.
#[derive(Debug, Clone)]
pub enum MenuBarItem {
    Separator,
    Item(ActionId),
    Submenu {
        title: String,
        items: Vec<MenuBarItem>,
    },
}

/// Helper representation for serializing and deserializing a submenu in [`MenuBarItem`].
#[derive(Serialize, Deserialize)]
struct SubmenuDef {
    title: String,
    items: Vec<MenuBarItem>,
}

/// Helper enum for untagged deserialization of a [`MenuBarItem`].
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

/// Mapping between a keyboard [`KeySequence`] and an action identifier, with optional parameters and context.
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
    value.is_null()
}

fn is_none<T>(value: &Option<T>) -> bool {
    value.is_none()
}

/// Persisted configuration manifest containing named keyboard [`ActionBinding`]s.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionBindingManifest {
    pub name: String,
    pub actions: Vec<ActionBinding>,
}

impl ActionBindingManifest {
    pub fn has_shortcut_conflict(&self, index: usize, shortcut: KeySequence) -> bool {
        self.actions
            .iter()
            .enumerate()
            .any(|(other, binding)| other != index && binding.shortcut == shortcut)
    }
}

impl Configuration for ActionBindingManifest {
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

/// Bidirectional lookup table between keyboard [`KeySequence`]s and [`ActionId`]s.
///
/// Constructed from an [`ActionBindingManifest`] to resolve triggered shortcuts into actions and
/// query the assigned shortcut for a given [`ActionId`].
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

pub struct GlobalActionBindings {
    config: ActionBindingManifestConfig,
    collection: ActionCollection,
}

impl Global for GlobalActionBindings {}

impl GlobalActionBindings {
    pub fn load() -> Self {
        let config = ActionBindingManifestConfig::read_or_init_or_fallback();
        let collection = ActionCollection::new(&config.get());
        Self { config, collection }
    }

    pub fn manifest(&self) -> Arc<ActionBindingManifest> {
        self.config.get()
    }

    pub fn collection(&self) -> &ActionCollection {
        &self.collection
    }

    pub fn rebind(&mut self, index: usize, shortcut: KeySequence) -> Result<()> {
        let manifest = self.manifest();
        let binding = manifest
            .actions
            .get(index)
            .ok_or_else(|| Error::msg(format!("Action binding does not exist: {index}")))?;
        if binding.shortcut == shortcut {
            return Ok(());
        }
        ensure!(
            !manifest.has_shortcut_conflict(index, shortcut),
            "Action shortcut is already assigned: {shortcut}"
        );

        self.config
            .update(|manifest| manifest.actions[index].shortcut = shortcut)?;
        self.collection = ActionCollection::new(&self.manifest());
        Ok(())
    }
}
