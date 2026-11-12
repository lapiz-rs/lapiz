use std::{
    env, fmt,
    fs::File,
    io::Write as _,
    path::{Path, PathBuf},
    sync::{Arc, LazyLock, Mutex},
    time::Instant,
};

use iced_core::{Element, Point};
use indexmap::IndexMap;
use lapiz_assets::{loader::AssetSerializer as _, store::AssetRegistry};
use lapiz_effect::{
    asset::{
        EffectAssetSerializer, EffectInputSlotId, EffectOutputSlotId, EffectPassDispatchStrategy,
        EffectPassId, EffectPassInputSlotId, EffectPassOutputSlotId,
    },
    editor::{EffectEditorMessage, EffectEditorState, EffectEditorView},
    instance::{EffectInputSlot, EffectInstance, EffectOutputSlot, EffectPass},
    nodes::{PassInput, PassInputNode, PassOutput, PassOutputDef, PassOutputNode, effect_nodes},
};
use lapiz_shader_graph::{
    GraphRenderer, GraphTheme,
    graph::{
        Graph, GraphResources,
        node::{GraphNodeId, GraphNodeRegistry},
        slot::ErasedGraphValueType,
        variable::GraphTypeRegistry,
    },
    wgsl_std::{
        builtin_types,
        nodes::math::{ScalarMathNode, ScalarMathNodeMode},
        types::primitive::F32Type,
    },
};
use uuid::Uuid;

static TYPE_REGISTRY: LazyLock<Arc<GraphTypeRegistry>> =
    LazyLock::new(|| Arc::new(builtin_types()));

static NODE_REGISTRY: LazyLock<Arc<GraphNodeRegistry>> = LazyLock::new(|| Arc::new(effect_nodes()));

// Temporary diagnosis logging; remove once the io editor refresh bug is fixed.
static DEBUG_START: LazyLock<Instant> = LazyLock::new(Instant::now);
static DEBUG_LOG: Mutex<Option<File>> = Mutex::new(None);

fn debug_log(line: impl fmt::Display) {
    let mut slot = DEBUG_LOG.lock().unwrap();
    let file = slot.get_or_insert_with(|| {
        File::create(env::temp_dir().join("editor_demo_debug.log")).expect("create debug log")
    });
    let elapsed = DEBUG_START.elapsed().as_millis();
    let _ = writeln!(file, "[{elapsed:>7}ms] {line}");
    let _ = file.flush();
    eprintln!("[{elapsed:>7}ms] {line}");
}

fn graph_resources() -> GraphResources {
    GraphResources {
        type_registry: TYPE_REGISTRY.clone(),
        node_registry: NODE_REGISTRY.clone(),
        assets: AssetRegistry::new_in_memory(Arc::new(Default::default())),
    }
}

struct DemoEditor {
    instance: EffectInstance,
    editor_state: EffectEditorState,
    save_path: PathBuf,
}

impl DemoEditor {
    fn new() -> Self {
        let save_path = env::temp_dir().join("lapiz_effect_editor_demo.lef");
        eprintln!("effect editor demo persists to {}", save_path.display());
        let loaded = load_instance(&save_path);
        debug_log(if loaded.is_some() {
            "startup: loaded effect from save file"
        } else {
            "startup: built fresh demo effect"
        });
        Self {
            instance: loaded.unwrap_or_else(demo_effect),
            editor_state: EffectEditorState::new(graph_resources()),
            save_path,
        }
    }

    fn update(&mut self, message: EffectEditorMessage) {
        debug_log(format!("update: {message:?}"));
        self.editor_state.update(&mut self.instance, message);
        self.save();
    }

    fn save(&self) {
        let asset = match self.instance.as_asset() {
            Ok(asset) => asset,
            Err(error) => {
                eprintln!("failed to serialize the effect: {error}");
                return;
            }
        };
        let Ok(mut file) = File::create(&self.save_path) else {
            eprintln!("failed to create {}", self.save_path.display());
            return;
        };
        if let Err(error) = EffectAssetSerializer.write(&asset, &mut file) {
            eprintln!("failed to save the effect: {error}");
        }
    }

    fn view(&self) -> Element<'_, EffectEditorMessage, GraphTheme, GraphRenderer> {
        debug_log("view: rebuilt");
        EffectEditorView::new(&self.instance, &self.editor_state).into()
    }
}

fn load_instance(path: &Path) -> Option<EffectInstance> {
    let mut file = File::open(path).ok()?;
    let relative_path = Path::new(path.file_name()?);
    let asset = EffectAssetSerializer.read(relative_path, &mut file).ok()?;
    EffectInstance::from_asset(&asset, graph_resources()).ok()
}

fn input_node(
    graph: &mut Graph,
    position: Point,
    source: PassInput,
    ty: Arc<dyn ErasedGraphValueType>,
) -> (GraphNodeId, EffectPassInputSlotId) {
    let node = graph.add_node(position, PassInputNode);
    let id = graph
        .get_node(&node)
        .unwrap()
        .data
        .state::<PassInputNode>()
        .unwrap()
        .id;
    graph.update_node_state::<PassInputNode>(node, |state| {
        state.input = Some(source);
        state.cached_ty = Some(ty);
    });
    (node, id)
}

fn output_node(
    graph: &mut Graph,
    position: Point,
    output: PassOutput,
    ty: Arc<dyn ErasedGraphValueType>,
) -> (GraphNodeId, EffectPassOutputSlotId) {
    let node = graph.add_node(position, PassOutputNode);
    let id = graph
        .get_node(&node)
        .unwrap()
        .data
        .state::<PassOutputNode>()
        .unwrap()
        .id;
    graph.update_node_state::<PassOutputNode>(node, |state| {
        state.output = Some(output);
        state.cached_ty = Some(ty);
    });
    (node, id)
}

fn demo_effect() -> EffectInstance {
    let layer_ty = TYPE_REGISTRY.resolve_type("layer_rgba8").unwrap();
    let f32_ty = Arc::new(F32Type);

    let canvas_input = EffectInputSlotId::new(Uuid::new_v4());
    let strength_input = EffectInputSlotId::new(Uuid::new_v4());
    let result_output = EffectOutputSlotId::new(Uuid::new_v4());

    let mut adjust_graph = Graph::new(graph_resources());
    let (strength, _) = input_node(
        &mut adjust_graph,
        Point::new(60.0, 140.0),
        PassInput::Effect(strength_input),
        f32_ty.clone(),
    );
    let math = adjust_graph.add_node(Point::new(320.0, 140.0), ScalarMathNode);
    adjust_graph
        .update_node_state::<ScalarMathNode>(math, |mode| *mode = ScalarMathNodeMode::Multiply);
    let (local_out, local_port) = output_node(
        &mut adjust_graph,
        Point::new(580.0, 140.0),
        PassOutput::Pass(PassOutputDef {
            name: "doubled strength".into(),
            ty: f32_ty.clone(),
        }),
        f32_ty.clone(),
    );
    adjust_graph.connect_slots_by_index(strength, 0, math, 0);
    adjust_graph.connect_slots_by_index(math, 0, local_out, 0);

    let mut apply_graph = Graph::new(graph_resources());
    let (canvas, _) = input_node(
        &mut apply_graph,
        Point::new(60.0, 120.0),
        PassInput::Effect(canvas_input),
        layer_ty.clone(),
    );
    // Left unconnected on purpose: wire "doubled strength" up in the graph editor.
    input_node(
        &mut apply_graph,
        Point::new(60.0, 300.0),
        PassInput::Pass(local_port),
        f32_ty,
    );
    let (result, result_port) = output_node(
        &mut apply_graph,
        Point::new(580.0, 120.0),
        PassOutput::Effect(result_output),
        layer_ty.clone(),
    );
    apply_graph.connect_slots_by_index(canvas, 0, result, 0);
    apply_graph.connect_slots_by_index(canvas, 1, result, 1);

    let passes = IndexMap::from([
        (
            EffectPassId::new(Uuid::new_v4()),
            EffectPass {
                name: "Adjust".into(),
                graph: adjust_graph,
                dispatch_strategy: EffectPassDispatchStrategy::Once,
            },
        ),
        (
            EffectPassId::new(Uuid::new_v4()),
            EffectPass {
                name: "Apply".into(),
                graph: apply_graph,
                dispatch_strategy: EffectPassDispatchStrategy::EveryOutputLayerPixel(result_port),
            },
        ),
    ]);

    let mut instance = EffectInstance {
        name: "Demo Effect".into(),
        passes,
        inputs: IndexMap::from([
            (
                canvas_input,
                EffectInputSlot {
                    name: "Canvas".into(),
                    id: canvas_input,
                    ty: layer_ty.clone(),
                },
            ),
            (
                strength_input,
                EffectInputSlot {
                    name: "Strength".into(),
                    id: strength_input,
                    ty: TYPE_REGISTRY.resolve_type("f32").unwrap(),
                },
            ),
        ]),
        outputs: IndexMap::from([(
            result_output,
            EffectOutputSlot {
                name: "Result".into(),
                id: result_output,
                ty: layer_ty,
            },
        )]),
    };
    instance.sync_pass_graph_effect_properties().unwrap();
    instance
}

fn main() -> iced::Result {
    lapiz_shader_graph::init_i18n();
    lapiz_effect::init_i18n();
    iced::application(DemoEditor::new, DemoEditor::update, DemoEditor::view)
        .window_size((1280.0, 800.0))
        .title("Effect Editor Demo")
        .run()
}
