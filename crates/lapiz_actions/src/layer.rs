use std::any::TypeId;

use iced_runtime::Task;
use lapiz_canvas::{
    CanvasAppExt, CanvasUndoStackAppExt,
    command::{
        DeleteLayersCommand, GroupLayerCommand, InsertLayerCommand, LayerWithPosition,
        MoveLayersCommand,
    },
};
use lapiz_i18n::t;
use lapiz_image::layer::{
    LayerId, LayerPosition, LayerStackNode,
    group_layer::GroupLayer,
    pixel_layer::PixelLayer,
    properties::{LayerProperties, NameProp},
};
use lapiz_runtime::Services;
use lapiz_utils::log_err::LogErr;

use crate::{ActionFunction, ActionId};

#[derive(Default)]
pub struct CreateNewLayerAction;

impl ActionFunction for CreateNewLayerAction {
    type Message = ();

    fn id(&self) -> ActionId {
        ActionId::new("create_new_layer_action".into())
    }

    fn trigger(&self, services: &mut Services) -> Task<Self::Message> {
        let Some(canvas_id) = services.current_canvas_id() else {
            return Task::none();
        };

        let cmd = services
            .update_canvas(&canvas_id, |canvas, _| {
                let (parent, position) = {
                    let mut cur_parent = canvas.active_layer_node();
                    let mut cur_position = LayerPosition::foreground();
                    while !cur_parent
                        .instance()
                        .can_have_children_of(TypeId::of::<PixelLayer>())
                    {
                        let parent_id =
                            canvas.image.layer_stack().get_layer(cur_parent.parent()?)?;
                        cur_position = LayerPosition::above(*cur_parent.id());
                        cur_parent = canvas
                            .image
                            .layer_stack()
                            .get_layer(parent_id.id())
                            .unwrap();
                    }
                    (*cur_parent.id(), cur_position)
                };
                let name = canvas.image.next_name_of_layer(t!("default_layer_name"));

                let new_layer =
                    LayerStackNode::without_parent(LayerId::random(), Box::new(PixelLayer), {
                        let mut props = LayerProperties::new::<PixelLayer>();
                        props.set(NameProp(name));
                        props
                    });
                Some(InsertLayerCommand::new(canvas, new_layer, parent, position))
            })
            .flatten();

        if let Some(cmd) = cmd {
            services.push_undo_command_to_current(cmd).log_err();
        }

        Task::none()
    }
}

#[derive(Default)]
pub struct GroupSelectedLayersAction;

impl ActionFunction for GroupSelectedLayersAction {
    type Message = ();

    fn id(&self) -> ActionId {
        ActionId::new("group_selected_layers_action".into())
    }

    fn trigger(&self, services: &mut Services) -> Task<Self::Message> {
        let Some(canvas_id) = services.current_canvas_id() else {
            return Task::none();
        };

        let cmd = services.update_canvas(&canvas_id, |canvas, _| {
            let group_name = canvas.image.next_name_of_layer(t!("default_group_name"));
            let reduced_layers = canvas
                .image
                .layer_stack()
                .reduce_ancestors(canvas.selected_layer_ids().iter().copied());
            let sorted_selected_layers = canvas
                .image
                .layer_stack()
                .sort_by_depth_and_index(reduced_layers)
                .unwrap();
            let children_layers = sorted_selected_layers
                .into_iter()
                .map(|l| {
                    let parent = canvas.image.layer_stack().get_parent_of(&l).unwrap();
                    let above = parent.child_below(&l);
                    LayerWithPosition {
                        id: l,
                        original_parent: *parent.id(),
                        original_above: above,
                    }
                })
                .collect();

            let (active_layer_parent, active_layer_index) = canvas
                .image
                .layer_stack()
                .get_position_of(&canvas.active_layer_id())
                .unwrap();

            let group_layer =
                LayerStackNode::without_parent(LayerId::random(), Box::new(GroupLayer), {
                    let mut props = LayerProperties::new::<GroupLayer>();
                    props.set(NameProp(group_name));
                    props
                });

            GroupLayerCommand {
                canvas: canvas.id(),
                group: group_layer,
                children: children_layers,
                parent_id: *active_layer_parent.id(),
                index: active_layer_index,
            }
        });

        if let Some(cmd) = cmd {
            services.push_undo_command_to_current(cmd).log_err();
        }

        Task::none()
    }
}

#[derive(Default)]
pub struct MoveLayerUpAction;

impl ActionFunction for MoveLayerUpAction {
    type Message = ();

    fn id(&self) -> ActionId {
        ActionId::new("move_layer_up_action".into())
    }

    fn trigger(&self, services: &mut Services) -> Task<Self::Message> {
        let Some(canvas_id) = services.current_canvas_id() else {
            return Task::none();
        };

        let cmd = services
            .update_canvas(&canvas_id, |canvas, _| {
                let mut layers = canvas
                    .selected_layer_ids()
                    .iter()
                    .copied()
                    .collect::<Vec<_>>();
                canvas.image.layer_stack().sort_by_visual_index(&mut layers);

                let head = layers.last().copied().unwrap();
                let head_parent = canvas.image.layer_stack().get_parent_of(&head).unwrap();
                let head_parent_node = canvas
                    .image
                    .layer_stack()
                    .get_layer(head_parent.id())
                    .unwrap();

                let (new_parent, new_position) =
                    if let Some(sibling_id) = head_parent_node.child_above(&head) {
                        if canvas
                            .image
                            .layer_stack()
                            .can_have_children_of(&sibling_id, &head)
                            .unwrap()
                        {
                            (sibling_id, LayerPosition::background())
                        } else {
                            (*head_parent.id(), LayerPosition::above(sibling_id))
                        }
                    } else {
                        let head_parent_parent = head_parent_node.parent().copied()?;
                        (head_parent_parent, LayerPosition::above(*head_parent.id()))
                    };

                Some(MoveLayersCommand::new(
                    canvas,
                    layers,
                    new_parent,
                    new_position,
                ))
            })
            .flatten();

        if let Some(cmd) = cmd {
            services.push_undo_command_to_current(cmd).log_err();
        }

        Task::none()
    }
}

#[derive(Default)]
pub struct MoveLayerDownAction;

impl ActionFunction for MoveLayerDownAction {
    type Message = ();

    fn id(&self) -> ActionId {
        ActionId::new("move_layer_down_action".into())
    }

    fn trigger(&self, services: &mut Services) -> Task<Self::Message> {
        let Some(canvas_id) = services.current_canvas_id() else {
            return Task::none();
        };

        let cmd = services
            .update_canvas(&canvas_id, |canvas, _| {
                let mut layers = canvas
                    .selected_layer_ids()
                    .iter()
                    .copied()
                    .collect::<Vec<_>>();
                canvas.image.layer_stack().sort_by_visual_index(&mut layers);

                let tail = layers.first().copied().unwrap();
                let tail_parent = canvas.image.layer_stack().get_parent_of(&tail).unwrap();
                let tail_parent_node = canvas
                    .image
                    .layer_stack()
                    .get_layer(tail_parent.id())
                    .unwrap();

                let (new_parent, new_position) =
                    if let Some(sibling_id) = tail_parent_node.child_below(&tail) {
                        if canvas
                            .image
                            .layer_stack()
                            .can_have_children_of(&sibling_id, &tail)
                            .expect("Sibling layer should always exist")
                        {
                            (sibling_id, LayerPosition::foreground())
                        } else {
                            (*tail_parent.id(), LayerPosition::below(sibling_id))
                        }
                    } else {
                        let tail_parent_parent = tail_parent_node.parent().copied()?;
                        (tail_parent_parent, LayerPosition::below(*tail_parent.id()))
                    };

                Some(MoveLayersCommand::new(
                    canvas,
                    layers,
                    new_parent,
                    new_position,
                ))
            })
            .flatten();

        if let Some(cmd) = cmd {
            services.push_undo_command_to_current(cmd).log_err();
        }

        Task::none()
    }
}

#[derive(Default)]
pub struct DeleteSelectedLayersAction;

impl ActionFunction for DeleteSelectedLayersAction {
    type Message = ();

    fn id(&self) -> ActionId {
        ActionId::new("delete_selected_layers_action".into())
    }

    fn trigger(&self, services: &mut Services) -> Task<Self::Message> {
        let Some(canvas_id) = services.current_canvas_id() else {
            return Task::none();
        };

        let cmd = services
            .update_canvas(&canvas_id, |canvas, _| {
                DeleteLayersCommand::new(
                    canvas,
                    canvas.selected_layer_ids().iter().copied().collect(),
                )
                .logged_err()
                .ok()
            })
            .flatten();

        if let Some(cmd) = cmd {
            services.push_undo_command_to_current(cmd).log_err();
        }

        Task::none()
    }
}

#[derive(Default)]
pub struct SelectPreviousLayerAction;

impl ActionFunction for SelectPreviousLayerAction {
    type Message = ();

    fn id(&self) -> ActionId {
        ActionId::new("select_previous_layer_action".into())
    }

    fn trigger(&self, services: &mut Services) -> Task<Self::Message> {
        let Some(canvas_id) = services.current_canvas_id() else {
            return Task::none();
        };
        services.update_canvas(&canvas_id, |canvas, _| {
            let active_node = canvas.active_layer_node();
            let active_parent_node = canvas
                .image
                .layer_stack()
                .get_layer(active_node.parent().unwrap())
                .unwrap();
            if let Some(layer) = active_parent_node.child_above(active_node.id()) {
                let mut current = canvas.image.layer_stack().get_layer(&layer).unwrap();
                while let Some(child) = current.children().first() {
                    current = canvas.image.layer_stack().get_layer(child).unwrap();
                }
                canvas.set_active_layer_and_clear_select(*current.id());
            } else {
                canvas.set_active_layer_and_clear_select(*active_parent_node.id());
            }
        });
        Task::none()
    }
}

#[derive(Default)]
pub struct SelectNextLayerAction;

impl ActionFunction for SelectNextLayerAction {
    type Message = ();

    fn id(&self) -> ActionId {
        ActionId::new("select_next_layer_action".into())
    }

    fn trigger(&self, services: &mut Services) -> Task<Self::Message> {
        let Some(canvas_id) = services.current_canvas_id() else {
            return Task::none();
        };
        services.update_canvas(&canvas_id, |canvas, _| {
            let active_node = canvas.active_layer_node();

            if let Some(child) = active_node.children().last() {
                canvas.set_active_layer_and_clear_select(*child);
                return;
            }

            let active_parent_node = canvas
                .image
                .layer_stack()
                .get_layer(active_node.parent().unwrap())
                .unwrap();

            if let Some(layer) = active_parent_node.child_below(active_node.id()) {
                canvas.set_active_layer_and_clear_select(layer);
                return;
            }

            let mut current = active_parent_node;
            while let Some(current_parent) = current
                .parent()
                .and_then(|p| canvas.image.layer_stack().get_layer(p))
            {
                if let Some(layer) = current_parent.child_below(current.id()) {
                    canvas.set_active_layer_and_clear_select(layer);
                    return;
                }
                current = current_parent;
            }
        });
        Task::none()
    }
}
