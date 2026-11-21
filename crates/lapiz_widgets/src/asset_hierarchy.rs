use std::{collections::HashSet, hash::Hash};

use iced_core::{Element, Length, Padding, Theme};
use lapiz_runtime::Renderer;

use crate::{collapsible, container, flex::Flex, label, panel};

/// A tag tree whose caller supplies the asset rows and owns the expanded state.
pub struct AssetHierarchy<TagId, Asset> {
    pub roots: Vec<AssetHierarchyNode<TagId, Asset>>,
}

/// A tag with nested items or an asset rendered by the caller.
pub enum AssetHierarchyNode<TagId, Asset> {
    Tag {
        id: TagId,
        name: String,
        children: Vec<Self>,
    },
    Asset(Asset),
}

impl<TagId: Copy + Eq + Hash, Asset> AssetHierarchy<TagId, Asset> {
    pub fn view<'a, Message: 'a>(
        &'a self,
        expanded_tags: &HashSet<TagId>,
        on_toggle: impl Fn(TagId) -> Message,
        view_asset: impl Fn(&'a Asset) -> Element<'a, Message, Theme, Renderer>,
    ) -> Element<'a, Message, Theme, Renderer> {
        let content = Flex::column(
            self.roots
                .iter()
                .map(|node| node.view(expanded_tags, &on_toggle, &view_asset)),
        )
        .width(Length::Fill)
        .gap(2);
        let frame = panel(content).width(Length::Fill).padding(1);
        let frame = if self.roots.is_empty() {
            frame.transparent()
        } else {
            frame
        };
        container(frame).padding(4).width(Length::Fill).into()
    }
}

impl<TagId: Copy + Eq + Hash, Asset> AssetHierarchyNode<TagId, Asset> {
    fn view<'a, Message: 'a>(
        &'a self,
        expanded_tags: &HashSet<TagId>,
        on_toggle: &impl Fn(TagId) -> Message,
        view_asset: &impl Fn(&'a Asset) -> Element<'a, Message, Theme, Renderer>,
    ) -> Element<'a, Message, Theme, Renderer> {
        match self {
            Self::Tag { id, name, children } => {
                let open = expanded_tags.contains(id);
                let visible_children = if open { children.as_slice() } else { &[] };
                let content = Flex::column(
                    visible_children
                        .iter()
                        .map(|node| node.view(expanded_tags, on_toggle, view_asset)),
                )
                .width(Length::Fill)
                .gap(2)
                .padding(Padding::ZERO.left(12));
                collapsible(label(name.as_str()), content, open)
                    .borderless()
                    .on_toggle(on_toggle(*id))
                    .into()
            }
            Self::Asset(asset) => view_asset(asset),
        }
    }
}
