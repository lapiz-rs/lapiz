use std::sync::Arc;

use iced_core::Point;
use lapiz_assets::store::AssetRegistry;
use lapiz_image::texel::TexelType;
use uuid::Uuid;

use crate::{
    asset::{
        EffectInputSlotId, EffectOutputSlotId, EffectPassDispatchStrategy, EffectPassId,
        EffectPassInputSlotId, EffectPassOutputSlotId,
    },
    editor::{EffectEditorMessage, EffectEditorState, EffectEditorView, dispatch_choices},
    instance::{EffectInputSlot, EffectInstance, EffectOutputSlot, EffectPass},
    nodes::{
        DispatchIndexNode, PassInput, PassInputNode, PassOutput, PassOutputDef, PassOutputNode,
        effect_nodes,
    },
    shader_graph::{
        GraphElement,
        builtin::{
            builtin_types,
            types::{
                atomic::{ArrayAtomicI32Type, ArrayAtomicU32Type},
                handle::{ArrayType, LayerType},
                primitive::F32Type,
            },
        },
        graph::{Graph, GraphResources, node::GraphNodeId, slot::ErasedGraphValueType},
    },
};

fn test_instance() -> EffectInstance {
    let resources = GraphResources {
        type_registry: Arc::new(builtin_types()),
        node_registry: Arc::new(effect_nodes()),
        assets: AssetRegistry::new_in_memory(Arc::new(Default::default())),
    };
    let mut instance = EffectInstance {
        name: "Test".into(),
        passes: Default::default(),
        inputs: Default::default(),
        outputs: Default::default(),
    };
    instance.passes.insert(
        EffectPassId::new(Uuid::new_v4()),
        EffectPass {
            name: "Pass".into(),
            graph: Graph::new(resources),
            dispatch_strategy: EffectPassDispatchStrategy::Once,
        },
    );
    instance
}

fn add_input(
    instance: &mut EffectInstance,
    ty: Arc<dyn ErasedGraphValueType>,
) -> EffectPassInputSlotId {
    let input = EffectInputSlotId::new(Uuid::new_v4());
    instance.inputs.insert(
        input,
        EffectInputSlot {
            name: ty.id().id,
            id: input,
            ty,
        },
    );
    let graph = &mut instance.single_pass_mut().unwrap().graph;
    let node = graph.add_node(Point::ORIGIN, PassInputNode);
    graph.update_node_state::<PassInputNode>(node, |state| {
        state.input = Some(PassInput::Effect(input));
    });
    graph
        .get_node(&node)
        .unwrap()
        .data
        .state::<PassInputNode>()
        .unwrap()
        .id
}

fn add_output(
    instance: &mut EffectInstance,
    ty: Arc<dyn ErasedGraphValueType>,
    external: bool,
) -> (GraphNodeId, EffectPassOutputSlotId) {
    let output = if external {
        let output = EffectOutputSlotId::new(Uuid::new_v4());
        instance.outputs.insert(
            output,
            EffectOutputSlot {
                name: "Result".into(),
                id: output,
                ty,
            },
        );
        PassOutput::Effect(output)
    } else {
        PassOutput::Pass(PassOutputDef {
            name: "Local".into(),
            ty,
        })
    };
    let graph = &mut instance.single_pass_mut().unwrap().graph;
    let node = graph.add_node(Point::ORIGIN, PassOutputNode);
    graph.update_node_state::<PassOutputNode>(node, |state| state.output = Some(output));
    let port = graph
        .get_node(&node)
        .unwrap()
        .data
        .state::<PassOutputNode>()
        .unwrap()
        .id;
    (node, port)
}

#[test]
fn dispatch_targets_use_compatible_bound_ports() {
    let mut instance = test_instance();
    let layer = Arc::new(LayerType {
        texel_type: TexelType::RGBA8,
    });
    let layer_input = add_input(&mut instance, layer.clone());
    let array_input = add_input(
        &mut instance,
        Arc::new(ArrayType {
            element_type: Arc::new(F32Type),
            len: 4,
        }),
    );
    let atomic_i32_input = add_input(&mut instance, Arc::new(ArrayAtomicI32Type { len: 4 }));
    let atomic_u32_input = add_input(&mut instance, Arc::new(ArrayAtomicU32Type { len: 4 }));
    add_input(&mut instance, Arc::new(F32Type));
    let (_, external_output) = add_output(&mut instance, layer.clone(), true);
    let (_, local_output) = add_output(&mut instance, layer, false);
    add_output(&mut instance, Arc::new(F32Type), false);
    instance
        .single_pass_mut()
        .unwrap()
        .graph
        .add_node(Point::ORIGIN, PassInputNode);
    instance
        .single_pass_mut()
        .unwrap()
        .graph
        .add_node(Point::ORIGIN, PassOutputNode);
    instance.sync_pass_graph_effect_properties().unwrap();

    let choices = dispatch_choices(&instance, instance.single_pass_id().unwrap());
    assert_eq!(choices.len(), 7);
    for strategy in [
        EffectPassDispatchStrategy::Once,
        EffectPassDispatchStrategy::EveryInputLayerPixel(layer_input),
        EffectPassDispatchStrategy::EveryBufferElement(array_input),
        EffectPassDispatchStrategy::EveryBufferElement(atomic_i32_input),
        EffectPassDispatchStrategy::EveryBufferElement(atomic_u32_input),
        EffectPassDispatchStrategy::EveryOutputLayerPixel(external_output),
        EffectPassDispatchStrategy::EveryOutputLayerPixel(local_output),
    ] {
        assert!(choices.iter().any(|choice| choice.strategy == strategy));
    }
    assert!(
        choices
            .iter()
            .any(|choice| choice.label.starts_with("Result"))
    );
    assert!(
        choices
            .iter()
            .any(|choice| choice.label.starts_with("Local"))
    );
}

#[test]
fn selecting_dispatch_updates_graph_slots_and_saved_strategy() {
    let mut instance = test_instance();
    let (_, output) = add_output(
        &mut instance,
        Arc::new(LayerType {
            texel_type: TexelType::RGBA8,
        }),
        true,
    );
    let array_input = add_input(&mut instance, Arc::new(ArrayAtomicU32Type { len: 4 }));
    let layer_input = add_input(
        &mut instance,
        Arc::new(LayerType {
            texel_type: TexelType::RGBA8,
        }),
    );
    let (_, second_output) = add_output(
        &mut instance,
        Arc::new(LayerType {
            texel_type: TexelType::RGBA8,
        }),
        false,
    );
    let pass_id = instance.single_pass_id().unwrap();
    let graph = &mut instance.single_pass_mut().unwrap().graph;
    let index = graph.add_node(Point::ORIGIN, DispatchIndexNode);
    let resources = graph.resources().clone();
    instance.sync_pass_graph_effect_properties().unwrap();
    let mut editor = EffectEditorState::new(resources.clone());
    editor.update(&mut instance, EffectEditorMessage::OpenPass(pass_id));

    for strategy in [
        EffectPassDispatchStrategy::EveryOutputLayerPixel(output),
        EffectPassDispatchStrategy::EveryOutputLayerPixel(second_output),
        EffectPassDispatchStrategy::EveryInputLayerPixel(layer_input),
        EffectPassDispatchStrategy::EveryBufferElement(array_input),
        EffectPassDispatchStrategy::Once,
    ] {
        editor.update(
            &mut instance,
            EffectEditorMessage::PassDispatchStrategySelected(pass_id, strategy),
        );
        let pass = instance.single_pass().unwrap();
        let node = pass.graph.get_node(&index).unwrap();
        assert_eq!(pass.dispatch_strategy, strategy);
        assert_eq!(
            node.data
                .state::<DispatchIndexNode>()
                .unwrap()
                .cached_dispatch_strategy,
            strategy,
        );
        assert_eq!(
            node.outputs.len(),
            usize::from(strategy != EffectPassDispatchStrategy::Once)
        );
        drop(GraphElement::from(EffectEditorView::new(
            &instance, &editor,
        )));

        let asset = instance.as_asset().unwrap();
        let saved = toml::to_string(&asset).unwrap();
        let restored =
            EffectInstance::from_asset(&toml::from_str(&saved).unwrap(), resources.clone())
                .unwrap();
        assert_eq!(restored.single_pass().unwrap().dispatch_strategy, strategy);
    }
}

#[test]
fn deleted_or_wrong_pass_targets_are_not_selected() {
    let mut instance = test_instance();
    let (node, output) = add_output(
        &mut instance,
        Arc::new(LayerType {
            texel_type: TexelType::RGBA8,
        }),
        false,
    );
    let pass_id = instance.single_pass_id().unwrap();
    let resources = instance.single_pass().unwrap().graph.resources().clone();
    instance.sync_pass_graph_effect_properties().unwrap();
    let mut editor = EffectEditorState::new(resources);
    editor.update(&mut instance, EffectEditorMessage::OpenPass(pass_id));
    let strategy = EffectPassDispatchStrategy::EveryOutputLayerPixel(output);
    editor.update(
        &mut instance,
        EffectEditorMessage::PassDispatchStrategySelected(pass_id, strategy),
    );
    instance.single_pass_mut().unwrap().graph.delete_node(&node);
    instance.sync_pass_graph_effect_properties().unwrap();
    drop(GraphElement::from(EffectEditorView::new(
        &instance, &editor,
    )));

    editor.update(
        &mut instance,
        EffectEditorMessage::PassDispatchStrategySelected(
            pass_id,
            EffectPassDispatchStrategy::Once,
        ),
    );
    editor.update(
        &mut instance,
        EffectEditorMessage::PassDispatchStrategySelected(pass_id, strategy),
    );
    editor.update(
        &mut instance,
        EffectEditorMessage::PassDispatchStrategySelected(
            EffectPassId::new(Uuid::new_v4()),
            strategy,
        ),
    );
    assert_eq!(
        instance.single_pass().unwrap().dispatch_strategy,
        EffectPassDispatchStrategy::Once
    );
}
