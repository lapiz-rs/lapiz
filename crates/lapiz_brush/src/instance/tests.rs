use lapiz_effect::asset::EffectAsset;
use lapiz_shader_graph::wgsl_std::types::primitive::F32Type;

use super::*;

fn assets() -> AssetRegistry {
    AssetRegistry::new_in_memory(Arc::new(Default::default()))
}

fn preset(main: &EffectInstance) -> BrushPreset {
    let empty = EffectAsset {
        name: "Empty".into(),
        passes: Vec::new(),
        inputs: Vec::new(),
        outputs: Vec::new(),
    };
    BrushPreset {
        metadata: BrushPresetMetadata {
            name: "Test".into(),
        },
        spacing_effect: empty.clone(),
        main_effect: main.as_asset().unwrap(),
        postprocess_effect: empty,
        parameters: Default::default(),
    }
}

#[test]
fn main_structure_roundtrip_and_graph_compile() {
    let assets = assets();
    let main = new_main_effect(assets.clone()).unwrap();
    let output = validate_main_effect(&main).unwrap();
    let pass = main.single_pass().unwrap();
    let port = pass_output_port_of(&main, output).unwrap();
    assert_eq!(
        pass.dispatch_strategy,
        EffectPassDispatchStrategy::EveryOutputLayerPixel(port)
    );
    let node = pass.graph.iter_nodes().next().unwrap();
    assert_eq!(node.inputs.len(), 2);
    pass.graph
        .compile(Vec::new(), GraphVarIdentGenerator::default())
        .unwrap();

    let instance = BrushPresetInstance::new(&preset(&main), assets.clone()).unwrap();
    let saved = instance.as_asset(&assets).unwrap();
    let loaded = BrushPresetInstance::new(&saved, assets).unwrap();
    assert_eq!(validate_main_effect(loaded.main_effect()).unwrap(), output);
}

#[test]
fn main_structure_checks_pass_count_at_load_and_save() {
    for count in [0, 2] {
        let assets = assets();
        let main = new_main_effect(assets.clone()).unwrap();
        let mut instance = BrushPresetInstance::new(&preset(&main), assets.clone()).unwrap();
        let main = instance.main_effect_mut();
        if count == 0 {
            main.passes.clear();
        } else {
            main.passes.insert(
                EffectPassId::new(Uuid::new_v4()),
                EffectPass {
                    name: "Extra".into(),
                    graph: Graph::new(main_effect_resources(assets.clone())),
                    dispatch_strategy: EffectPassDispatchStrategy::Once,
                },
            );
        }
        let invalid = preset(main);
        let error = BrushPresetInstance::new(&invalid, assets.clone())
            .err()
            .unwrap();
        assert!(error.to_string().contains("exactly one pass"), "{error:#}");
        assert!(
            instance
                .as_asset(&assets)
                .err()
                .unwrap()
                .to_string()
                .contains("exactly one pass")
        );
    }
}

#[test]
fn main_structure_checks_dispatch_at_load_and_save() {
    for (strategy, diagnostic) in [
        (EffectPassDispatchStrategy::Once, "dispatch strategy"),
        (
            EffectPassDispatchStrategy::EveryBufferElement(EffectPassInputSlotId::new(
                Uuid::new_v4(),
            )),
            "dispatch strategy",
        ),
        (
            EffectPassDispatchStrategy::EveryInputLayerPixel(EffectPassInputSlotId::new(
                Uuid::new_v4(),
            )),
            "dispatch strategy",
        ),
        (
            EffectPassDispatchStrategy::EveryOutputLayerPixel(EffectPassOutputSlotId::new(
                Uuid::new_v4(),
            )),
            "dispatch target",
        ),
    ] {
        let assets = assets();
        let main = new_main_effect(assets.clone()).unwrap();
        let mut instance = BrushPresetInstance::new(&preset(&main), assets.clone()).unwrap();
        instance
            .main_effect_mut()
            .single_pass_mut()
            .unwrap()
            .dispatch_strategy = strategy;
        let invalid = preset(instance.main_effect());
        let error = BrushPresetInstance::new(&invalid, assets.clone())
            .err()
            .unwrap();
        assert!(error.to_string().contains(diagnostic), "{error:#}");
        let error = instance.as_asset(&assets).err().unwrap();
        assert!(error.to_string().contains(diagnostic), "{error:#}");
    }
}

#[test]
fn main_structure_checks_output_declaration() {
    let mut main = new_main_effect(assets()).unwrap();
    let output = main.outputs.first_mut().unwrap().1;
    output.name = "Other".into();
    assert!(
        validate_main_effect(&main)
            .unwrap_err()
            .to_string()
            .contains("layer")
    );

    let output = main.outputs.first_mut().unwrap().1;
    output.name = MAIN_DAB_BUFFER.into();
    output.ty = Arc::new(F32Type);
    assert!(
        validate_main_effect(&main)
            .unwrap_err()
            .to_string()
            .contains("layer")
    );

    let mut main = new_main_effect(assets()).unwrap();
    main.outputs.clear();
    assert!(
        validate_main_effect(&main)
            .unwrap_err()
            .to_string()
            .contains("exactly one output")
    );

    let extra = new_main_effect(assets()).unwrap();
    main = new_main_effect(assets()).unwrap();
    main.outputs.extend(extra.outputs);
    assert!(
        validate_main_effect(&main)
            .unwrap_err()
            .to_string()
            .contains("exactly one output")
    );
}

#[test]
fn main_structure_checks_output_node_binding() {
    for target in [
        None,
        Some(PassOutput::Effect(EffectOutputSlotId::new(Uuid::new_v4()))),
    ] {
        let mut main = new_main_effect(assets()).unwrap();
        let node = main
            .single_pass_mut()
            .unwrap()
            .graph
            .iter_nodes_mut()
            .next()
            .unwrap();
        node.data.state_mut::<PassOutputNode>().unwrap().output = target;
        assert!(
            validate_main_effect(&main)
                .unwrap_err()
                .to_string()
                .contains("must target")
        );
    }

    let mut main = new_main_effect(assets()).unwrap();
    main.single_pass_mut()
        .unwrap()
        .graph
        .add_node(Point::ORIGIN, PassOutputNode);
    assert!(
        validate_main_effect(&main)
            .unwrap_err()
            .to_string()
            .contains("exactly one output node")
    );

    let mut main = new_main_effect(assets()).unwrap();
    main.single_pass_mut().unwrap().graph = Graph::new(main_effect_resources(assets()));
    assert!(
        validate_main_effect(&main)
            .unwrap_err()
            .to_string()
            .contains("one output node")
    );
}
