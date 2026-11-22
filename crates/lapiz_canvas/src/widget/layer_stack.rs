use iced_core::{Element, Font, Length, font::Weight};
use lapiz_i18n::Translated;
use lapiz_image::{
    composite::BlendFunctionRegistry,
    layer::{
        LayerId, LayerPosition, LayerStack, LayerStackNode,
        properties::{
            LayerProperties,
            builtin::{
                BlendFunctionPropertyExt as _, DisabledChannelsPropertyExt as _,
                LockedChannelsPropertyExt as _, LockedPropertyExt as _, NamePropertyExt as _,
                OpacityPropertyExt as _, VisiblePropertyExt as _,
            },
        },
    },
    tile::GpuTileStorage,
};
use lapiz_runtime::{Renderer, Theme};
use lapiz_widgets::{
    button, checkbox, column, combo_box, container, context_menu,
    flex::Flex,
    icon, label, menu, panel, space,
    spin_slider::SpinSlider,
    text_input,
    tree::{Tree, TreeNode, TreePosition},
};

use crate::{CCanvas, command::LayerPropertyChangeCommand};

/// UI messages emitted by [`LayerStackView`] for layer selection, renaming, drag-and-drop reordering, and property edits.
#[derive(Debug, Clone)]
pub enum LayerStackMessage {
    LayerPropertyChanged(LayerPropertyChangeCommand),
    MoveLayers {
        layer_ids: Vec<LayerId>,
        new_parent: LayerId,
        new_position: LayerPosition,
    },
    SelectLayer(LayerId),
    RenameLayer(LayerId),
    RenameChanged(String),
    RenameCommit(LayerId),
}

pub fn property_button_style(
    selected: bool,
) -> impl Fn(&iced_core::Theme, button::Status) -> button::Style {
    move |theme: &iced_core::Theme, status| {
        let palette = theme.palette();
        if selected {
            button::Style {
                background: Some(iced_core::Background::Color(palette.primary.base.color)),
                text_color: palette.primary.base.text,
                border: iced_core::Border {
                    color: palette.primary.strong.color,
                    width: 1.0,
                    ..Default::default()
                },
                ..Default::default()
            }
        } else {
            match status {
                button::Status::Pressed => button::Style {
                    background: Some(iced_core::Background::Color(palette.primary.base.color)),
                    text_color: palette.primary.base.text,
                    ..Default::default()
                },
                button::Status::Hovered => button::Style {
                    background: Some(iced_core::Background::Color(
                        palette.primary.weak.color.scale_alpha(0.5),
                    )),
                    text_color: palette.background.base.text,
                    ..Default::default()
                },
                _ => button::Style {
                    background: None,
                    text_color: palette.background.base.text,
                    ..Default::default()
                },
            }
        }
    }
}

fn property_command(
    canvas: &CCanvas,
    layer_id: LayerId,
    old: &LayerProperties,
    apply: impl Fn(&mut LayerProperties),
) -> LayerPropertyChangeCommand {
    let mut layer_props = old.clone();
    apply(&mut layer_props);
    LayerPropertyChangeCommand {
        canvas: canvas.id(),
        layer_id,
        old: old.clone(),
        new: layer_props,
    }
}

/// View builder that renders the hierarchical layer stack panel, blend mode/opacity controls, and drag-and-drop list for a [`CCanvas`].
pub struct LayerStackView<'a, Message: Clone + 'a> {
    canvas: &'a CCanvas,
    blend_functions: &'a BlendFunctionRegistry,
    tile_storage: &'a GpuTileStorage,
    renaming_layer: Option<LayerId>,
    rename_value: &'a str,
    on_message: &'a dyn Fn(LayerStackMessage) -> Message,
}

impl<'a, Message: Clone + 'a> LayerStackView<'a, Message> {
    pub fn new(
        canvas: &'a CCanvas,
        blend_functions: &'a BlendFunctionRegistry,
        tile_storage: &'a GpuTileStorage,
        renaming_layer: Option<LayerId>,
        rename_value: &'a str,
        on_message: &'a dyn Fn(LayerStackMessage) -> Message,
    ) -> Self {
        Self {
            canvas,
            blend_functions,
            tile_storage,
            renaming_layer,
            rename_value,
            on_message,
        }
    }
}

impl<'a, Message: Clone + 'a> From<LayerStackView<'a, Message>>
    for Element<'a, Message, iced_core::Theme, lapiz_runtime::Renderer>
{
    fn from(view: LayerStackView<'a, Message>) -> Self {
        let canvas = view.canvas;
        let on_message = view.on_message;

        let stack = canvas.image.layer_stack();
        let mut layer_widget = |node: &'a LayerStackNode| {
            let layer_id = *node.id();
            let properties = node.properties();
            let name = properties
                .get_name()
                .map(|n| n.to_owned())
                .unwrap_or_default();
            let is_selected = canvas.selected_layer_ids().contains(&layer_id);
            let is_active = canvas.active_layer_id() == layer_id;
            let is_renaming = view.renaming_layer == Some(layer_id);

            let mut children =
                Vec::<Element<'_, Message, iced_core::Theme, lapiz_runtime::Renderer>>::new();

            if let Some(visible) = properties.get_visible() {
                children.push(
                    checkbox(visible)
                        .on_toggle(move |checked| {
                            on_message(LayerStackMessage::LayerPropertyChanged(property_command(
                                canvas,
                                layer_id,
                                properties,
                                move |p| p.set_visible(checked),
                            )))
                        })
                        .into(),
                );
            }

            let name_element = if is_renaming {
                Element::from(
                    text_input("", view.rename_value)
                        .on_input(move |value| on_message(LayerStackMessage::RenameChanged(value)))
                        .on_submit(on_message(LayerStackMessage::RenameCommit(layer_id))),
                )
            } else {
                Element::from(label(name).size(12).width(Length::Fill).font(if is_active {
                    Font {
                        weight: Weight::Bold,
                        ..Font::default()
                    }
                } else {
                    Font::default()
                }))
            };
            children.push(name_element);

            if let Some(locked) = properties.get_locked() {
                let message = LayerStackMessage::LayerPropertyChanged(property_command(
                    canvas,
                    layer_id,
                    properties,
                    move |p| p.set_locked(!locked),
                ));
                children.push(
                    button(icon::lock().size(13))
                        .width(22)
                        .height(22)
                        .padding(4)
                        .style(property_button_style(locked))
                        .on_press(on_message(message))
                        .into(),
                );
            }

            let alpha_index = view
                .tile_storage
                .get_layer_info(layer_id)
                .map(|info| info.texel_type.alpha_channel_index())
                .unwrap_or_else(|| canvas.image.texel_type().alpha_channel_index());

            if let Some(channels) = properties.get_disabled_channels() {
                let message = LayerStackMessage::LayerPropertyChanged(property_command(
                    canvas,
                    layer_id,
                    properties,
                    move |p| {
                        let mut channels = channels;
                        channels.toggle_channel_disabled(alpha_index);
                        p.set_disabled_channels(channels)
                    },
                ));
                children.push(
                    button(icon::inherit_alpha().size(13))
                        .width(22)
                        .height(22)
                        .padding(4)
                        .style(property_button_style(
                            channels.is_channel_disabled(alpha_index),
                        ))
                        .on_press(on_message(message))
                        .into(),
                );
            }

            if let Some(channels) = properties.get_locked_channels() {
                let message = property_command(canvas, layer_id, properties, move |p| {
                    let mut channels = channels;
                    channels.toggle_channel_locked(alpha_index);
                    p.set_locked_channels(channels)
                });
                children.push(
                    button(icon::alpha_lock().size(13))
                        .width(22)
                        .height(22)
                        .padding(4)
                        .style(property_button_style(
                            channels.is_channel_locked(alpha_index),
                        ))
                        .on_press(on_message(LayerStackMessage::LayerPropertyChanged(message)))
                        .into(),
                );
            }

            let content = panel(Flex::row(children).gap(3.0).padding([3.0, 5.0]).height(30))
                .width(Length::Fill)
                .style(move |theme: &iced_core::Theme| container::Style {
                    background: if is_selected {
                        Some(theme.palette().primary.weak.color.into())
                    } else {
                        None
                    },
                    ..Default::default()
                });

            let menu = menu().item(
                "Rename",
                None,
                on_message(LayerStackMessage::RenameLayer(layer_id)),
            );
            context_menu(content, menu).into()
        };

        fn layer_tree_nodes<'a, Message>(
            stack: &'a LayerStack,
            parent: &'a LayerStackNode,
            layer_widget: &mut impl FnMut(&'a LayerStackNode) -> Element<'a, Message, Theme, Renderer>,
        ) -> Vec<TreeNode<'a, Message, LayerId>> {
            parent
                .iter_children_display_order()
                .map(|id| {
                    let node = stack.get_layer(id).unwrap();
                    TreeNode::Branch {
                        id: *id,
                        label: layer_widget(node),
                        children: layer_tree_nodes(stack, node, layer_widget),
                    }
                })
                .collect()
        }

        let nodes = layer_tree_nodes(stack, stack.root_node(), &mut layer_widget);
        let moved_roots = stack.reduce_ancestors(canvas.selected_layer_ids().iter().copied());
        let list = Tree::new(nodes)
            .height(Length::Fill)
            .spacing(1.0)
            .draggable(true)
            .on_press(move |&id| on_message(LayerStackMessage::SelectLayer(id)))
            .can_be_child_of(move |parent, _| {
                let parent = parent.unwrap_or_else(|| stack.root_id());
                moved_roots.iter().all(|child| {
                    parent != child
                        && !stack.is_ancestor(child, parent)
                        && stack.can_have_children_of(parent, child) == Some(true)
                })
            })
            .on_drag_drop(move |info| {
                on_message(LayerStackMessage::MoveLayers {
                    layer_ids: canvas.selected_layer_ids().iter().copied().collect(),
                    new_parent: *info.target_parent_id.unwrap_or_else(|| stack.root_id()),
                    new_position: match info.position {
                        TreePosition::Above(id) => LayerPosition::above(*id),
                        TreePosition::Below(id) => LayerPosition::below(*id),
                        TreePosition::First => LayerPosition::foreground(),
                        TreePosition::Last => LayerPosition::background(),
                    },
                })
            });
        let active_layer = canvas.active_layer_node().properties();
        let active_id = canvas.active_layer_id();
        let mut params = Vec::<Element<'_, Message, Theme, Renderer>>::new();

        if let Some(blend) = active_layer.get_blend_function() {
            let all = view
                .blend_functions
                .all_ids()
                .cloned()
                .map(Translated)
                .collect::<Vec<_>>();
            params.push(
                combo_box(all, Some(Translated(blend.clone())), move |option| {
                    let id = option.into_inner();
                    let message = property_command(canvas, active_id, active_layer, |p| {
                        p.set_blend_function(id.clone())
                    });
                    on_message(LayerStackMessage::LayerPropertyChanged(message))
                })
                .width(Length::Fill)
                .into(),
            );
        }

        if let Some(opacity) = active_layer.get_opacity() {
            params.push(
                SpinSlider::new_percent(opacity * 100.0)
                    .prefix("Opacity: ")
                    .suffix("%")
                    .on_confirm(move |value| {
                        let message = property_command(canvas, active_id, active_layer, move |p| {
                            p.set_opacity(value / 100.0)
                        });
                        on_message(LayerStackMessage::LayerPropertyChanged(message))
                    })
                    .into(),
            );
        }

        let params = if params.is_empty() {
            Element::from(space().width(Length::Fill).height(Length::Shrink))
        } else {
            Element::from(panel(Flex::column(params).gap(4.0)).padding(6.0))
        };

        column![params, list]
            .gap(8.0)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }
}
