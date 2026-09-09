use iced_core::{
    Alignment, Element, Font, Layout, Length, Point, Rectangle, Size,
    font::Weight,
    layout::{self, Limits},
    mouse, renderer,
    widget::Tree,
};
use iced_widget::{button, column, row, stack};
use indexmap::IndexMap;
use lapiz_i18n::Translated;
use lapiz_image::{
    composite::BlendFunctionRegistry,
    layer::{
        LayerId, LayerPosition,
        properties::{
            BlendFunctionPropertyExt, DisabledChannelsPropertyExt, LayerProperties,
            LockedChannelsPropertyExt, LockedPropertyExt, NamePropertyExt, OpacityPropertyExt,
            VisiblePropertyExt,
        },
    },
    tile::GpuTileStorage,
};
use lapiz_widgets::{
    button::Button,
    checkbox::Checkbox,
    combo_box::ComboBox,
    drag_drop_column::{DragDropColumn, DragDropInfo},
    icon,
    label::Label,
    menu::{ContextMenu, Menu},
    panel::{self, Panel},
    spin_slider::SpinSlider,
    text_input::TextInput,
};

use crate::{CCanvas, command::LayerPropertyChangeCommand};

/// The horizontal indent applied per nesting depth (matches the spacer width
/// used in the row layout).
const ROW_INDENT: f32 = 20.0;

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
    DropPreview(Option<DropInfo>),
}

#[derive(Debug, Clone)]
pub struct DropInfo {
    pub parent: LayerId,
    pub child_position: LayerPosition,
    pub position: Point,
    pub length: f32,
}

fn resolve_drop_target(
    canvas: &CCanvas,
    rows: &IndexMap<LayerId, Rectangle>,
    mouse_position: Point,
    dragged_id: LayerId,
) -> Option<DropInfo> {
    let layer_stack = canvas.image.layer_stack();
    let root_id = *layer_stack.root_id();

    let (target_id, target_bounds) = rows
        .iter()
        .filter(|(id, _)| **id != dragged_id && !layer_stack.is_ancestor(&dragged_id, id))
        .find(|(_, bounds)| mouse_position.y < bounds.y + bounds.height)?;
    let (target_id, target_bounds) = (*target_id, *target_bounds);

    // The cursor below the last row: drop at the end of the list (the root's
    // background), previewed at the last row's bottom edge.
    if target_id == root_id {
        return Some(DropInfo {
            parent: root_id,
            child_position: LayerPosition::background(),
            position: target_bounds.position(),
            length: target_bounds.width,
        });
    }

    if layer_stack.is_ancestor(&dragged_id, &target_id) {
        return None;
    }

    let target_node = layer_stack.get_layer(&target_id)?;
    let center_y = target_bounds.y + target_bounds.height / 2.0;

    if mouse_position.y < center_y {
        // Above the target's row: drop as a sibling, just above the target.
        return Some(DropInfo {
            parent: *target_node.parent()?,
            child_position: LayerPosition::above(target_id),
            position: target_bounds.position(),
            length: target_bounds.width,
        });
    }

    // Below the target's row.
    let target_parent = target_node.parent().copied()?;
    let target_parent_node = layer_stack.get_layer(&target_parent)?;
    let target_index = target_parent_node.child_index(&target_id)?;

    // If the target can hold children and the cursor is at the target's own
    // indent (inside the target's row, not in a shallower ancestor's gutter),
    // dropping on its lower half nests the dragged layer as the target's new
    // top child (preview at the target's bottom edge). A shallower cursor
    // falls through to the x-driven ancestor matching below.
    if mouse_position.x >= target_bounds.x
        && layer_stack.can_have_children_of(&target_id, &dragged_id)?
    {
        return Some(DropInfo {
            parent: target_id,
            child_position: LayerPosition::foreground(),
            position: Point::new(target_bounds.x, target_bounds.y + target_bounds.height),
            length: target_bounds.width,
        });
    }

    // The target can't hold children (or the cursor is at a shallower indent),
    // so the dragged layer lands below it as a sibling. When the target is the
    // bottom child of its parent, "below it" is ambiguous: it could mean a new
    // bottom of the target's parent, or the bottom of an ancestor further up.
    // The cursor's horizontal indent picks which ancestor's bottom to append to.
    if target_index != 0 {
        // Target has a lower sibling, so "below the target" stays inside the
        // target's parent, just under the target.
        return Some(DropInfo {
            parent: target_parent,
            child_position: LayerPosition::below(target_id),
            position: Point::new(target_bounds.x, target_bounds.y + target_bounds.height),
            length: target_bounds.width,
        });
    }

    // Target is the bottom child. Only ancestors for which the target's
    // branch is also the bottom child are valid "append to bottom" targets.
    let ancestors = layer_stack.ancestors(target_id).collect::<Vec<_>>();
    let mut ambiguous_count = 0;
    {
        let mut previous_child = target_id;
        for ancestor in &ancestors {
            let ancestor_node = layer_stack.get_layer(ancestor)?;
            if ancestor_node.child_index(&previous_child) == Some(0) {
                ambiguous_count += 1;
            } else {
                break;
            }
            previous_child = *ancestor;
        }
    }

    if ambiguous_count == 0 {
        return None;
    }

    // The deepest candidate ancestor whose row the cursor is still inside is
    // the new parent; the dragged layer becomes its new bottom child. If the
    // cursor is left of every candidate, append to the shallowest one.
    let mut resolved_parent_index = ambiguous_count - 1;
    for (index, ancestor) in ancestors.iter().take(ambiguous_count).enumerate() {
        let bounds = rows.get(ancestor)?;
        if mouse_position.x >= bounds.x {
            resolved_parent_index = index;
            break;
        }
    }
    let resolved_parent = ancestors[resolved_parent_index];
    if layer_stack.is_ancestor(&dragged_id, &resolved_parent) {
        return None;
    }

    let resolved_preview_bounds = if resolved_parent_index == 0 {
        target_bounds
    } else {
        *rows.get(&ancestors[resolved_parent_index - 1])?
    };

    Some(DropInfo {
        parent: resolved_parent,
        child_position: LayerPosition::background(),
        position: Point::new(
            resolved_preview_bounds.x,
            target_bounds.y + target_bounds.height,
        ),
        length: resolved_preview_bounds.width,
    })
}

pub fn property_button_style(
    selected: bool,
) -> impl Fn(&iced_core::Theme, button::Status) -> button::Style {
    move |theme: &iced_core::Theme, status| {
        let palette = theme.extended_palette();
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

fn resolve_target<'b>(
    info: &DragDropInfo<'b>,
    canvas: &CCanvas,
    layers: &[LayerId],
) -> Option<DropInfo> {
    let layer_stack = canvas.image.layer_stack();
    let root_id = *layer_stack.root_id();
    let dragged_id = layers[info.dragged_index];
    let mut rows = info
        .column_layout
        .children()
        .zip(layers)
        .map(|(node, id)| {
            let depth = layer_stack.ancestors(*id).filter(|a| *a != root_id).count() as f32;
            let bounds = node.bounds();
            (
                *id,
                Rectangle {
                    x: depth * ROW_INDENT,
                    y: bounds.y,
                    width: bounds.width,
                    height: bounds.height,
                },
            )
        })
        .collect::<IndexMap<_, _>>();
    let column_bounds = info.column_layout.bounds();
    let root_bounds = Rectangle {
        x: 0.0,
        y: rows
            .last()
            .map(|(_, bounds)| bounds.y + bounds.height)
            .unwrap_or(0.0),
        width: column_bounds.size().width,
        height: (column_bounds.size().height
            - rows
                .last()
                .map(|(_, bounds)| bounds.y + bounds.height)
                .unwrap_or(0.0))
        .max(0.0),
    };
    rows.insert(root_id, root_bounds);
    resolve_drop_target(canvas, &rows, info.mouse_position, dragged_id)
}

pub struct LayerStackView<'a, Message: Clone + 'a> {
    canvas: &'a CCanvas,
    blend_functions: &'a BlendFunctionRegistry,
    tile_storage: &'a GpuTileStorage,
    renaming_layer: Option<LayerId>,
    rename_value: &'a str,
    drop_preview: Option<DropInfo>,
    on_message: &'a dyn Fn(LayerStackMessage) -> Message,
}

impl<'a, Message: Clone + 'a> LayerStackView<'a, Message> {
    pub fn new(
        canvas: &'a CCanvas,
        blend_functions: &'a BlendFunctionRegistry,
        tile_storage: &'a GpuTileStorage,
        renaming_layer: Option<LayerId>,
        rename_value: &'a str,
        drop_preview: Option<DropInfo>,
        on_message: &'a dyn Fn(LayerStackMessage) -> Message,
    ) -> Self {
        Self {
            canvas,
            blend_functions,
            tile_storage,
            renaming_layer,
            rename_value,
            drop_preview,
            on_message,
        }
    }
}

pub struct DropIndicatorOverlay {
    drop_preview: Option<DropInfo>,
}

impl DropIndicatorOverlay {
    pub fn new(drop_preview: Option<DropInfo>) -> Self {
        Self { drop_preview }
    }
}

impl<Message, Theme, Renderer> iced_core::Widget<Message, Theme, Renderer> for DropIndicatorOverlay
where
    Renderer: iced_core::Renderer,
{
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Fill)
    }

    fn layout(&mut self, _tree: &mut Tree, _renderer: &Renderer, limits: &Limits) -> layout::Node {
        layout::Node::new(limits.max())
    }

    fn draw(
        &self,
        _tree: &Tree,
        renderer: &mut Renderer,
        _theme: &Theme,
        _style: &renderer::Style,
        _layout: Layout<'_>,
        _cursor: mouse::Cursor,
        _viewport: &Rectangle,
    ) {
        let Some(drop_info) = &self.drop_preview else {
            return;
        };
        let indicator = Rectangle {
            x: drop_info.position.x,
            y: drop_info.position.y,
            width: drop_info.length.max(0.0),
            height: 2.0,
        };
        renderer.fill_quad(
            renderer::Quad {
                bounds: indicator,
                ..Default::default()
            },
            iced_core::Background::Color(iced_core::Color::from_rgb(0.3, 0.6, 1.0)),
        );
    }
}

impl<'a, Message: Clone + 'a> From<LayerStackView<'a, Message>>
    for Element<'a, Message, iced_core::Theme, iced_wgpu::Renderer>
{
    fn from(view: LayerStackView<'a, Message>) -> Self {
        let canvas = view.canvas;
        let on_message = view.on_message;

        let layer_nodes = canvas
            .image
            .layer_stack()
            .iter_layers_dfs_display_order_without_root()
            .collect::<Vec<_>>();
        let layers = layer_nodes
            .iter()
            .map(|(node, _)| *node.id())
            .collect::<Vec<_>>();

        let mut rows: Vec<Element<'a, Message, iced_core::Theme, iced_wgpu::Renderer>> =
            Vec::with_capacity(layer_nodes.len());

        for (node, depth) in &layer_nodes {
            let layer_id = *node.id();
            let properties = node.properties();
            let name = properties
                .get_name()
                .map(|n| n.to_owned())
                .unwrap_or_default();
            let is_selected = canvas.selected_layer_ids().contains(&layer_id);
            let is_active = canvas.active_layer_id() == layer_id;
            let is_renaming = view.renaming_layer == Some(layer_id);

            let mut children: Vec<Element<'_, Message, iced_core::Theme, iced_wgpu::Renderer>> =
                vec![];

            if let Some(visible) = properties.get_visible() {
                children.push(
                    Checkbox::new(visible)
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
                TextInput::new("", view.rename_value)
                    .on_input(move |value| on_message(LayerStackMessage::RenameChanged(value)))
                    .on_submit(on_message(LayerStackMessage::RenameCommit(layer_id)))
                    .into()
            } else {
                Label::new(name)
                    .size(12)
                    .width(Length::Fill)
                    .font(if is_active {
                        Font {
                            weight: Weight::Bold,
                            ..Font::default()
                        }
                    } else {
                        Font::default()
                    })
                    .into()
            };
            children.push(
                row([
                    iced_widget::Space::new()
                        .width(Length::Fixed(20.0 * *depth as f32))
                        .into(),
                    name_element,
                ])
                .width(Length::Fill)
                .into(),
            );

            if let Some(locked) = properties.get_locked() {
                let message = LayerStackMessage::LayerPropertyChanged(property_command(
                    canvas,
                    layer_id,
                    properties,
                    move |p| p.set_locked(!locked),
                ));
                children.push(
                    Button::new(icon::lock().size(13))
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
                    Button::new(icon::inherit_alpha().size(13))
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
                    Button::new(icon::alpha_lock().size(13))
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

            let content = Panel::new(
                row(children)
                    .spacing(3)
                    .align_y(Alignment::Center)
                    .padding([3, 5])
                    .height(30),
            )
            .width(Length::Fill)
            .style(move |theme: &iced_core::Theme| panel::Style {
                background: if is_selected {
                    Some(theme.extended_palette().primary.weak.color.into())
                } else {
                    None
                },
                ..Default::default()
            });

            let menu = Menu::new().item(
                "Rename",
                on_message(LayerStackMessage::RenameLayer(layer_id)),
            );
            rows.push(ContextMenu::new(content, menu).into());
        }

        let on_press_layers = layers.clone();
        let on_drag_layers = layers.clone();
        let on_drop_layers = layers.clone();
        let on_press = move |info: DragDropInfo| {
            on_message(LayerStackMessage::SelectLayer(
                on_press_layers[info.dragged_index],
            ))
        };
        let on_drag = move |info: DragDropInfo| {
            let target = resolve_target(&info, canvas, &on_drag_layers);
            on_message(LayerStackMessage::DropPreview(target))
        };
        let on_drop = move |info: DragDropInfo| {
            let target = resolve_target(&info, canvas, &on_drop_layers);
            on_message(LayerStackMessage::DropPreview(None));
            match target {
                Some(target) => on_message(LayerStackMessage::MoveLayers {
                    layer_ids: canvas.selected_layer_ids().iter().copied().collect(),
                    new_parent: target.parent,
                    new_position: target.child_position,
                }),
                None => on_message(LayerStackMessage::DropPreview(None)),
            }
        };

        let list = DragDropColumn::new(rows)
            .spacing(1.0)
            .on_press(on_press)
            .on_drag(on_drag)
            .on_drop(on_drop);
        let overlay = Element::new(DropIndicatorOverlay::new(view.drop_preview));
        let list_with_overlay = stack![list, overlay]
            .width(Length::Fill)
            .height(Length::Fill);
        let active_layer = canvas.active_layer_node().properties();
        let active_id = canvas.active_layer_id();
        let mut params: Vec<Element<'_, Message, iced_core::Theme, iced_wgpu::Renderer>> = vec![];

        if let Some(blend) = active_layer.get_blend_function() {
            let all = view
                .blend_functions
                .all_ids()
                .cloned()
                .map(Translated)
                .collect::<Vec<_>>();
            params.push(
                ComboBox::new(all, Some(Translated(blend.clone())), move |option| {
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

        let params: Element<'_, Message, iced_core::Theme, iced_wgpu::Renderer> =
            if params.is_empty() {
                iced_widget::Space::new()
                    .width(Length::Fill)
                    .height(Length::Shrink)
                    .into()
            } else {
                Panel::new(column(params).spacing(4.0)).padding(6.0).into()
            };

        column![params, list_with_overlay]
            .spacing(8.0)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }
}
