use std::sync::{Arc, LazyLock};

use lapiz_shader_graph::{
    editor::{GraphEditorMessage, GraphEditorState, GraphEditorView},
    graph::{
        Graph, GraphData, GraphResources, node::GraphNodeRegistry, variable::GraphTypeRegistry,
    },
    wgsl_std::{
        builtin_nodes, builtin_types,
        nodes::{GraphInputNode, GraphOutputNode},
    },
};

#[derive(Default, Clone)]
struct DemoData;

impl GraphData for DemoData {}

static NODE_REGISTRY: LazyLock<Arc<GraphNodeRegistry<DemoData>>> = LazyLock::new(|| {
    let mut nodes = builtin_nodes();
    nodes.register::<GraphInputNode>();
    nodes.register::<GraphOutputNode>();
    Arc::new(nodes)
});

static TYPE_REGISTRY: LazyLock<Arc<GraphTypeRegistry>> =
    LazyLock::new(|| Arc::new(builtin_types()));

struct DemoEditor {
    graph: Graph<DemoData>,
    editor_state: GraphEditorState,
}

impl DemoEditor {
    fn new() -> Self {
        Self {
            graph: Graph::new(GraphResources {
                type_registry: TYPE_REGISTRY.clone(),
                node_registry: NODE_REGISTRY.clone(),
                ..Default::default()
            }),
            editor_state: GraphEditorState::default(),
        }
    }

    fn view(
        &self,
    ) -> iced_core::Element<'_, GraphEditorMessage, iced::Theme, lapiz_runtime::Renderer> {
        GraphEditorView::new(&self.graph, &self.editor_state).into()
    }

    fn update(&mut self, message: GraphEditorMessage) {
        self.editor_state.update(&mut self.graph, message);
    }
}

fn main() -> iced::Result {
    iced::application(DemoEditor::new, DemoEditor::update, DemoEditor::view)
        .window_size((1280.0, 800.0))
        .run()
}
