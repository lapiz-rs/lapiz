use std::collections::{HashMap, HashSet};

use anyhow::{Context as _, Result, bail};
use lapiz_assets::{
    asset::{AssetMetadata, UntypedAssetId},
    index_db::UntypedAssetFilter,
    store::AssetRegistry,
    tag::{PositionInTag, Tag, TagId},
};
use lapiz_widgets::drag_drop::DropZone;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AssetList {
    All,
    Tagged,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DragItem {
    Asset(UntypedAssetId, AssetList),
    Tag { id: TagId, parent: TagId },
    List(AssetList),
    TagRoot,
}

pub struct TagRow {
    pub id: TagId,
    pub parent: TagId,
    pub depth: usize,
}

#[derive(Default)]
pub struct EditorModel {
    pub current_asset_ty: Option<String>,
    pub current_tag: Option<TagId>,
    pub selected_asset: Option<UntypedAssetId>,
    pub tags: HashMap<TagId, Tag>,
    pub children: HashMap<TagId, Vec<TagId>>,
    pub all_assets: Vec<AssetMetadata>,
    pub tagged_assets: Vec<AssetMetadata>,
    pub expanded: HashMap<String, HashSet<TagId>>,
    pub root: Option<TagId>,
}

impl EditorModel {
    pub fn refresh(&mut self, assets: &AssetRegistry) -> Result<()> {
        assets.sync_type_tag_assets()?;
        self.tags = assets
            .all_tags()?
            .into_iter()
            .map(|tag| (tag.id, tag))
            .collect();
        self.children.clear();
        for &id in self.tags.keys() {
            let children = assets
                .index_db()
                .get_child_tags(&id)?
                .into_iter()
                .map(|tag| tag.id)
                .collect();
            self.children.insert(id, children);
        }
        self.root = self
            .tags
            .values()
            .find(|tag| tag.is_type_tag() && tag.asset_ty == self.current_asset_ty)
            .map(|tag| tag.id);
        let Some(root) = self.root else {
            self.all_assets.clear();
            self.tagged_assets.clear();
            self.current_tag = None;
            self.selected_asset = None;
            return Ok(());
        };

        let visible = self
            .tags
            .values()
            .filter(|tag| self.visible(tag))
            .map(|tag| tag.id)
            .collect::<HashSet<_>>();
        let nested = self
            .children
            .iter()
            .filter(|(parent, _)| visible.contains(parent))
            .flat_map(|(_, children)| children.iter().copied())
            .collect::<HashSet<_>>();
        let mut roots = visible.difference(&nested).copied().collect::<Vec<_>>();
        roots.sort_by(|a, b| self.tags[a].name.cmp(&self.tags[b].name).then(a.cmp(b)));
        let previous = self.children.get(&root).cloned().unwrap_or_default();
        let mut ordered = previous
            .iter()
            .copied()
            .filter(|id| roots.contains(id))
            .collect::<Vec<_>>();
        for id in roots {
            if !ordered.contains(&id) {
                ordered.push(id);
            }
        }
        for id in previous.iter().filter(|id| !ordered.contains(id)) {
            assets.remove_child_from_tag(&self.tags[&root], id)?;
        }
        for &id in ordered.iter().filter(|id| !previous.contains(id)) {
            assets.add_tag_to_tag_at(&self.tags[&root], &id, PositionInTag::Last)?;
        }
        self.children.insert(root, ordered);
        self.all_assets = assets.index_db().get_assets(UntypedAssetFilter {
            ty: self.current_asset_ty.clone(),
            tag: Some(root),
            ..Default::default()
        })?;
        if self.current_tag.is_some_and(|id| !visible.contains(&id)) {
            self.current_tag = None;
        }
        self.tagged_assets = match self.current_tag {
            Some(tag) => assets.index_db().get_assets(UntypedAssetFilter {
                ty: self.current_asset_ty.clone(),
                tag: Some(tag),
                ..Default::default()
            })?,
            None => Vec::new(),
        };
        if self
            .selected_asset
            .is_some_and(|id| !self.all_assets.iter().any(|asset| asset.asset_id == id))
        {
            self.selected_asset = None;
        }
        Ok(())
    }

    fn visible(&self, tag: &Tag) -> bool {
        !tag.is_type_tag() && (tag.asset_ty.is_none() || tag.asset_ty == self.current_asset_ty)
    }

    pub fn is_tag_expanded(&self, id: TagId) -> bool {
        self.current_asset_ty
            .as_ref()
            .and_then(|ty| self.expanded.get(ty))
            .is_some_and(|expanded| expanded.contains(&id))
    }

    pub fn set_tag_expanded(&mut self, id: TagId, expanded: bool) {
        let Some(ty) = &self.current_asset_ty else {
            return;
        };
        let tags = self.expanded.entry(ty.clone()).or_default();

        if expanded {
            tags.insert(id);
        } else {
            tags.remove(&id);
        }
    }

    pub fn rows(&self) -> Vec<TagRow> {
        let mut rows = Vec::new();
        if let Some(root) = self.root {
            self.append_rows(root, 0, &mut HashSet::new(), &mut rows);
        }
        rows
    }

    fn append_rows(
        &self,
        parent: TagId,
        depth: usize,
        path: &mut HashSet<TagId>,
        rows: &mut Vec<TagRow>,
    ) {
        if !path.insert(parent) {
            return;
        }
        for &id in self.children.get(&parent).into_iter().flatten() {
            if !self.tags.get(&id).is_some_and(|tag| self.visible(tag)) {
                continue;
            }
            rows.push(TagRow { id, parent, depth });
            if self.is_tag_expanded(id) {
                self.append_rows(id, depth + 1, path, rows);
            }
        }
        path.remove(&parent);
    }

    pub fn has_children(&self, id: TagId) -> bool {
        self.children.get(&id).is_some_and(|children| {
            children
                .iter()
                .any(|id| self.tags.get(id).is_some_and(|tag| self.visible(tag)))
        })
    }

    fn contains_tag(&self, ancestor: TagId, descendant: TagId) -> bool {
        let mut pending = vec![ancestor];
        let mut seen = HashSet::new();
        while let Some(id) = pending.pop() {
            if id == descendant {
                return true;
            }
            if seen.insert(id) {
                pending.extend(self.children.get(&id).into_iter().flatten().copied());
            }
        }
        false
    }

    pub fn accepts(&self, source: &DragItem, target: &DragItem) -> bool {
        match (*source, *target) {
            (DragItem::Asset(asset, list), DragItem::Asset(other, destination)) => {
                asset != other && (list == destination || destination == AssetList::Tagged)
            }
            (DragItem::Asset(_, list), DragItem::List(destination)) => {
                (list == destination || destination == AssetList::Tagged)
                    && (destination == AssetList::All || self.current_tag.is_some())
            }
            (DragItem::Asset(_, _), DragItem::Tag { .. }) => true,
            (DragItem::Tag { id, .. }, DragItem::Tag { id: target, .. }) => {
                !self.contains_tag(id, target)
            }
            (DragItem::Tag { .. }, DragItem::TagRoot) => true,
            _ => false,
        }
    }

    pub fn drop_item(
        &mut self,
        source: DragItem,
        target: DragItem,
        zone: DropZone,
        assets: &AssetRegistry,
    ) -> Result<()> {
        if !self.accepts(&source, &target) {
            bail!("Invalid drop target");
        }
        match source {
            DragItem::Asset(id, _) => {
                let (tag, position) = match target {
                    DragItem::Asset(reference, list) => (
                        self.list_tag(list)?,
                        match zone {
                            DropZone::Before => PositionInTag::Before(reference),
                            _ => PositionInTag::After(reference),
                        },
                    ),
                    DragItem::List(list) => (self.list_tag(list)?, PositionInTag::Last),
                    DragItem::Tag { id, .. } => (id, PositionInTag::Last),
                    _ => bail!("Invalid asset destination"),
                };
                assets.add_asset_to_tag_at(id, tag, position)?;
            }
            DragItem::Tag { id, parent } => {
                let (destination, position) = match target {
                    DragItem::Tag { id: target, parent } => match zone {
                        DropZone::Into => (target, PositionInTag::Last),
                        DropZone::Before => (parent, PositionInTag::Before(target)),
                        DropZone::After => (parent, PositionInTag::After(target)),
                    },
                    DragItem::TagRoot => (
                        self.root.context("Missing type order tag")?,
                        PositionInTag::Last,
                    ),
                    _ => bail!("Invalid tag destination"),
                };
                assets.move_tag_at(id, &self.tags[&parent], &self.tags[&destination], position)?;
                if zone == DropZone::Into {
                    self.set_tag_expanded(destination, true);
                }
            }
            _ => bail!("Invalid drag source"),
        }
        self.refresh(assets)
    }

    fn list_tag(&self, list: AssetList) -> Result<TagId> {
        match list {
            AssetList::All => self.root,
            AssetList::Tagged => self.current_tag,
        }
        .context("No tag selected")
    }

    pub fn selected_metadata(&self) -> Option<&AssetMetadata> {
        self.all_assets
            .iter()
            .find(|asset| Some(asset.asset_id) == self.selected_asset)
    }
}

pub fn matches_search(asset: &AssetMetadata, name: &str, query: &str) -> bool {
    let name = name.to_lowercase();
    let path = asset.relative_path.to_lowercase();
    query.split_whitespace().all(|word| {
        let word = word.to_lowercase();
        name.contains(&word) || path.contains(&word)
    })
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        io::{Read, Write},
        path::Path,
        sync::Arc,
    };

    use lapiz_assets::{
        asset::Asset,
        bundle::{AssetBundle as _, directory::AssetDirectory},
        loader::{AssetSerializer, AssetSerializerRegistry},
        tag::TagFile,
    };

    use super::*;

    struct TestAsset;
    impl Asset for TestAsset {
        const TYPE_NAME: &'static str = "test_asset";
    }
    #[derive(Default)]
    struct TestSerializer;
    impl AssetSerializer for TestSerializer {
        type Asset = TestAsset;
        fn file_extension() -> &'static str {
            "testasset"
        }
        fn read(&self, _: &Path, _: &mut dyn Read) -> Result<TestAsset> {
            Ok(TestAsset)
        }
        fn write(&self, _: &TestAsset, writer: &mut dyn Write) -> Result<()> {
            writer.write_all(b"asset")?;
            Ok(())
        }
    }

    #[test]
    fn editor_persists_orders_membership_and_parent_changes() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let directory = temp.path().join("builtin_assets");
        let root = TagFile::new(
            Tag::type_tag_name(TestAsset::TYPE_NAME),
            Some(TestAsset::TYPE_NAME.into()),
        );
        let parent = TagFile::new("Parent".into(), Some(TestAsset::TYPE_NAME.into()));
        let child = TagFile::new("Shared".into(), None);
        for (path, tag) in [
            (Tag::type_tag_path(TestAsset::TYPE_NAME), &root),
            ("parent.tag".into(), &parent),
            ("child.tag".into(), &child),
        ] {
            let path = directory.join(path);
            fs::create_dir_all(path.parent().expect("tag path has a parent"))?;
            fs::write(path, toml::to_string(tag)?)?;
        }
        let mut serializers = AssetSerializerRegistry::default();
        serializers.register::<TestSerializer>();
        let registry = AssetRegistry::new(temp.path(), Arc::new(serializers.clone()))?;
        let bundle = AssetDirectory::new(&directory)?;
        let bundle_id = bundle.metadata()?.bundle_id;
        registry.add_bundle(bundle)?;
        let a = registry
            .add_asset(bundle_id, "a.testasset", Arc::new(TestAsset))?
            .into_untyped();
        let b = registry
            .add_asset(bundle_id, "b.testasset", Arc::new(TestAsset))?
            .into_untyped();
        let mut model = EditorModel {
            current_asset_ty: Some(TestAsset::TYPE_NAME.into()),
            ..Default::default()
        };
        model.refresh(&registry)?;
        assert_eq!(model.rows().len(), 2);
        assert!(model.rows().iter().all(|row| row.id != root.id));
        model.drop_item(
            DragItem::Asset(b, AssetList::All),
            DragItem::Asset(a, AssetList::All),
            DropZone::Before,
            &registry,
        )?;
        assert_eq!(
            model
                .all_assets
                .iter()
                .map(|asset| asset.asset_id)
                .collect::<Vec<_>>(),
            [b, a]
        );
        model.current_tag = Some(parent.id);
        model.drop_item(
            DragItem::Asset(a, AssetList::All),
            DragItem::List(AssetList::Tagged),
            DropZone::Into,
            &registry,
        )?;
        model.drop_item(
            DragItem::Asset(b, AssetList::All),
            DragItem::Asset(a, AssetList::Tagged),
            DropZone::Before,
            &registry,
        )?;
        assert_eq!(
            model
                .tagged_assets
                .iter()
                .map(|asset| asset.asset_id)
                .collect::<Vec<_>>(),
            [b, a]
        );
        model.drop_item(
            DragItem::Tag {
                id: child.id,
                parent: root.id,
            },
            DragItem::Tag {
                id: parent.id,
                parent: root.id,
            },
            DropZone::Into,
            &registry,
        )?;
        assert!(model.has_children(parent.id));
        assert!(!model.accepts(
            &DragItem::Tag {
                id: parent.id,
                parent: root.id
            },
            &DragItem::Tag {
                id: child.id,
                parent: parent.id
            }
        ));
        assert!(
            model
                .rows()
                .iter()
                .any(|row| row.id == child.id && row.depth == 1)
        );
        assert!(matches_search(
            &model.all_assets[0],
            "Soft Brush",
            "SOFT b.test"
        ));
        assert!(!matches_search(
            &model.all_assets[0],
            "Soft Brush",
            "missing"
        ));
        drop(registry);

        let registry = AssetRegistry::new(temp.path(), Arc::new(serializers))?;
        registry.add_bundle(AssetDirectory::new(&directory)?)?;
        model.refresh(&registry)?;
        assert_eq!(model.all_assets[0].asset_id, b);
        assert_eq!(model.tagged_assets[0].asset_id, b);
        assert!(model.has_children(parent.id));
        model.drop_item(
            DragItem::Tag {
                id: child.id,
                parent: parent.id,
            },
            DragItem::TagRoot,
            DropZone::Into,
            &registry,
        )?;
        assert!(!model.has_children(parent.id));
        assert!(
            model
                .rows()
                .iter()
                .any(|row| row.id == child.id && row.depth == 0)
        );
        Ok(())
    }
}
