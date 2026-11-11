use std::{collections::HashMap, mem, sync::Arc};

use anyhow::Result;
use iced_core::{Event, Length, Size, Theme, keyboard, window};
use iced_futures::{Subscription, subscription};
use iced_runtime::{
    Task,
    window::{close, drag, open},
};
use indexmap::IndexMap;
use lapiz_assets::{AssetAppExt as _, asset::AssetHandle};
use lapiz_canvas::{
    CCanvas, CanvasAppExt as _, CanvasId, CanvasUndoStackAppExt as _, command::TileReplaceCommand,
    event::CanvasUpdated,
};
use lapiz_effect::asset::EffectInputSlotId;
use lapiz_i18n::t;
use lapiz_image::{
    composite::{LayerPreviewOverriders, PixelPreviewOverrider},
    layer::{
        LayerId,
        properties::builtin::{LayerTexelTypePropertyExt as _, LockedPropertyExt as _},
    },
    texel::TexelType,
    tile::{DynamicLayerStorage, GpuTileStorage, TileStorageAppExt as _},
};
use lapiz_render::render_context::RenderContextAppExt as _;
use lapiz_runtime::{
    event::Event as _,
    global::Globals,
    windows::{OpenWindowViewCommand, WindowCommandBuffer, WindowView, WindowViewId},
};
use lapiz_shader_graph::graph::slot::{ErasedGraphLiteralUpdateMessage, GraphInputSlotId};
use lapiz_undo::BatchedUndoCommand;
use lapiz_widgets::{button, column, flex::Flex, label, panel, row, scrollable, space, title_bar};

use crate::{
    asset::FilterPreset,
    instance::{FilterInstance, FilterParameter},
    render::FilterRenderer,
};

/// Modal dialog window for interactively previewing and applying a [`FilterPreset`] to selected canvas layers.
///
/// When a user selects a filter or tweaks its [`FilterParameter`] controls, `FilterPanel` compiles and
/// dispatches the filter asynchronously via [`FilterRenderer`] across all unlocked RGBA8 target layers.
/// Completed renders are installed into [`LayerPreviewOverriders`] for non-destructive live canvas previewing;
/// confirming commits the modified tiles to the canvas undo stack via [`TileReplaceCommand`], whereas
/// canceling removes the preview overrides and restores the original layer contents.
pub struct FilterPanel {
    windows: Arc<[window::Id]>,
    main_window: window::Id,
    filters: Vec<AssetHandle<FilterPreset>>,
    selected: Option<FilterInstance>,
    renderer: Option<FilterRenderer>,
    generation: u64,
    rendering: bool,
    results: HashMap<LayerId, DynamicLayerStorage>,
    preview_installed: bool,
    target_layers: Vec<LayerId>,
    canvas_id: Option<CanvasId>,
}

/// UI and asynchronous render completion messages handled by [`FilterPanel`].
pub enum FilterPanelMessage {
    FilterSelected(usize),
    ParameterUpdated(EffectInputSlotId, ErasedGraphLiteralUpdateMessage),
    NewFilter,
    EditFilter,
    Confirm,
    Cancel,
    RenderFinished(u64, Result<HashMap<LayerId, DynamicLayerStorage>>),
    WindowClosed,

    Close,
    Drag,
}

impl Clone for FilterPanelMessage {
    fn clone(&self) -> Self {
        match self {
            FilterPanelMessage::FilterSelected(i) => FilterPanelMessage::FilterSelected(*i),
            FilterPanelMessage::ParameterUpdated(id, m) => {
                FilterPanelMessage::ParameterUpdated(*id, m.clone())
            }
            FilterPanelMessage::NewFilter => FilterPanelMessage::NewFilter,
            FilterPanelMessage::EditFilter => FilterPanelMessage::EditFilter,
            FilterPanelMessage::Confirm => FilterPanelMessage::Confirm,
            FilterPanelMessage::Cancel => FilterPanelMessage::Cancel,
            FilterPanelMessage::RenderFinished(_, _) => {
                // TODO DynamicLayerStorage is not clonable, but can we avoid this clone impl in the future?
                unreachable!("FilterPanel RenderFinished is never cloned")
            }
            FilterPanelMessage::WindowClosed => FilterPanelMessage::WindowClosed,
            FilterPanelMessage::Close => FilterPanelMessage::Close,
            FilterPanelMessage::Drag => FilterPanelMessage::Drag,
        }
    }
}

type Element<'a> = iced_core::Element<'a, FilterPanelMessage, Theme, lapiz_runtime::Renderer>;

impl WindowView for FilterPanel {
    type Message = FilterPanelMessage;

    type BootParams = ();

    fn id() -> WindowViewId {
        WindowViewId::new("filter_panel")
    }

    fn boot(
        _params: Option<Self::BootParams>,
        globals: &mut Globals,
    ) -> Result<(Self, Task<Self::Message>)> {
        let mut filters = globals
            .assets()
            .all_handles_of::<FilterPreset>()
            .expect("Failed to list filter presets");
        filters.sort_by(|a, b| {
            let a_name = a.get().map(|f| f.metadata.name.clone()).unwrap_or_default();
            let b_name = b.get().map(|f| f.metadata.name.clone()).unwrap_or_default();
            a_name.cmp(&b_name)
        });
        let (main_window, open) = open(window::Settings {
            decorations: false,
            size: Size {
                width: 720.0,
                height: 480.0,
            },
            #[cfg(target_os = "windows")]
            platform_specific: window::settings::PlatformSpecific {
                corner_preference: window::settings::platform::CornerPreference::DoNotRound,
                ..Default::default()
            },
            ..Default::default()
        });
        Ok((
            Self {
                windows: [main_window].into(),
                main_window,
                filters,
                selected: None,
                renderer: None,
                generation: 0,
                rendering: false,
                results: HashMap::new(),
                preview_installed: false,
                target_layers: Vec::new(),
                canvas_id: None,
            },
            open.discard(),
        ))
    }

    fn view<'a>(&'a self, _: window::Id, globals: &'a Globals) -> impl Into<Element<'a>> {
        let title_bar = title_bar(label(t!("filter_panel_title")).window_title())
            .on_drag(FilterPanelMessage::Drag)
            .on_close(FilterPanelMessage::Close);

        let filter_list = self
            .filters
            .iter()
            .enumerate()
            .map(|(index, handle)| {
                let name = handle
                    .get()
                    .map(|f| f.metadata.name.clone())
                    .unwrap_or_else(|_| "<loading>".to_string());
                button(label(name))
                    .width(Length::Fill)
                    .on_press(FilterPanelMessage::FilterSelected(index))
                    .into()
            })
            .collect::<Vec<_>>();

        let sidebar = panel(
            column![
                label(t!("filters")).strong(),
                scrollable(Flex::column(filter_list).gap(2.0))
                    .width(Length::Fill)
                    .height(Length::Fill),
                row![
                    button(label(t!("new_filter"))).on_press(FilterPanelMessage::NewFilter),
                    button(label(t!("edit_filter"))).on_press(FilterPanelMessage::EditFilter),
                ]
                .gap(4.0),
            ]
            .gap(6.0),
        )
        .padding(8)
        .width(220);

        let params = if let Some(selected) = self.selected.as_ref() {
            let parameter_rows = selected
                .parameters()
                .iter()
                .map(|(id, parameter)| {
                    row![
                        label(parameter.name.clone()).width(Length::Fill),
                        parameter
                            .value
                            .ty()
                            .view_literal(
                                GraphInputSlotId::new(id.0),
                                parameter.value.value(),
                                globals.assets(),
                            )
                            .map(move |message| {
                                FilterPanelMessage::ParameterUpdated(*id, message)
                            }),
                    ]
                    .gap(6.0)
                    .into()
                })
                .collect::<Vec<_>>();
            if parameter_rows.is_empty() {
                column![label(t!("no_external_variables")).muted()].gap(6.0)
            } else {
                column![
                    label(t!("parameters")).strong(),
                    scrollable(Flex::column(parameter_rows).gap(6.0))
                        .width(Length::Fill)
                        .height(Length::Fill),
                ]
                .gap(6.0)
            }
        } else {
            column![label(t!("select_a_filter_to_adjust")).muted()]
        };

        let ok_enabled = self.selected.is_some() && !self.rendering;
        let ok_label = if self.rendering { "Rendering..." } else { "OK" };

        let footer = row![
            space().width(Length::Fill),
            button(label(t!("cancel"))).on_press(FilterPanelMessage::Cancel),
            button(label(ok_label))
                .primary()
                .on_press_maybe(ok_enabled.then_some(FilterPanelMessage::Confirm)),
        ]
        .gap(10.0)
        .padding(16);

        panel(column![
            title_bar,
            column![
                row![sidebar, panel(params).padding(8).width(Length::Fill)].height(Length::Fill),
                footer,
            ]
            .width(Length::Fill)
            .height(Length::Fill)
        ])
    }

    fn update(
        &mut self,
        message: Self::Message,
        globals: &mut Globals,
    ) -> impl Into<Task<Self::Message>> {
        match message {
            FilterPanelMessage::FilterSelected(index) => self.filter_selected(index, globals),
            FilterPanelMessage::ParameterUpdated(id, message) => {
                self.parameter_updated(id, message, globals)
            }
            FilterPanelMessage::NewFilter | FilterPanelMessage::EditFilter => {
                globals
                    .global_mut::<WindowCommandBuffer>()
                    .push(OpenWindowViewCommand::new(WindowViewId::new(
                        "filter_editor",
                    )));
                Task::none()
            }
            FilterPanelMessage::Confirm => self.confirm(globals),
            FilterPanelMessage::Cancel => self.cancel(globals),
            FilterPanelMessage::RenderFinished(generation, result) => {
                self.render_finished(generation, result, globals)
            }
            FilterPanelMessage::WindowClosed => self.window_closed(globals),
            FilterPanelMessage::Close => close(self.main_window),
            FilterPanelMessage::Drag => drag(self.main_window),
        }
    }

    fn subscription(&self, _globals: &Globals) -> Subscription<Self::Message> {
        let main_window = self.main_window;
        subscription::filter_map(("filter_panel", main_window), move |event| match event {
            subscription::Event::Interaction {
                window,
                event: Event::Window(window::Event::Closed),
                status: _,
            } if window == main_window => Some(FilterPanelMessage::WindowClosed),
            subscription::Event::Interaction {
                window,
                event: Event::Keyboard(keyboard::Event::KeyPressed { key, modifiers, .. }),
                status: _,
            } if window == main_window
                && modifiers.control()
                && matches!(
                    &key,
                    keyboard::Key::Character(character)
                        if character.eq_ignore_ascii_case("w")
                ) =>
            {
                Some(FilterPanelMessage::Cancel)
            }
            _ => None,
        })
    }

    fn close(self, _: &mut Globals) -> Task<()> {
        close(self.main_window)
    }

    fn windows(&self) -> Arc<[window::Id]> {
        self.windows.clone()
    }

    fn root_window(&self) -> Option<window::Id> {
        Some(self.main_window)
    }
}

impl FilterPanel {
    fn filter_selected(&mut self, index: usize, globals: &mut Globals) -> Task<FilterPanelMessage> {
        let Some(handle) = self.filters.get(index).cloned() else {
            return Task::none();
        };
        let Some(canvas) = globals.current_canvas() else {
            return Task::none();
        };
        let canvas_id = canvas.id();
        let target_layers = resolve_target_layers(canvas);
        let removing_preview = self.preview_installed;
        let preview_canvas_id = self.canvas_id.unwrap_or(canvas_id);
        let dirty_tiles = if removing_preview {
            self.canvas_id
                .and_then(|preview_id| globals.canvas(&preview_id))
                .map(|preview_canvas| self.preview_dirty_tiles(preview_canvas))
        } else {
            None
        };
        if target_layers.is_empty() {
            log::warn!("Filter: no eligible target layers on current canvas");
            self.selected = None;
            self.renderer = None;
            self.results.clear();
            self.remove_previews(globals);
            self.preview_installed = false;
            self.rendering = false;
            self.target_layers.clear();
            self.canvas_id = None;
            if let Some(dirty_tiles) = dirty_tiles {
                CanvasUpdated::broadcast(CanvasUpdated {
                    id: preview_canvas_id,
                    dirty_tiles,
                });
            }
            return Task::none();
        }

        self.generation += 1;
        self.rendering = true;
        self.results.clear();
        self.remove_previews(globals);
        self.preview_installed = false;
        if let Some(dirty_tiles) = dirty_tiles {
            CanvasUpdated::broadcast(CanvasUpdated {
                id: preview_canvas_id,
                dirty_tiles,
            });
        }
        self.target_layers = target_layers.clone();
        self.canvas_id = Some(canvas_id);

        let instance = match FilterInstance::from_asset(&handle, globals.assets()) {
            Ok(instance) => instance,
            Err(e) => {
                log::error!("Failed to load filter preset: {e}");
                self.rendering = false;
                self.selected = None;
                self.renderer = None;
                return Task::none();
            }
        };
        let renderer = match FilterRenderer::new(globals, &instance) {
            Ok(renderer) => renderer,
            Err(e) => {
                log::error!("Failed to create filter renderer: {e}");
                self.rendering = false;
                self.selected = None;
                self.renderer = None;
                return Task::none();
            }
        };
        let generation = self.generation;
        let parameters = instance.parameters().clone();
        self.selected = Some(instance);
        self.renderer = Some(renderer);
        self.rerender(generation, target_layers, parameters, globals)
    }

    fn parameter_updated(
        &mut self,
        id: EffectInputSlotId,
        message: ErasedGraphLiteralUpdateMessage,
        globals: &mut Globals,
    ) -> Task<FilterPanelMessage> {
        if let Some(instance) = self.selected.as_mut() {
            instance.update_parameter(&id, message);
        }
        if self.target_layers.is_empty() || self.renderer.is_none() {
            return Task::none();
        }
        let Some(instance) = self.selected.as_ref() else {
            return Task::none();
        };
        self.generation += 1;
        let generation = self.generation;
        let target_layers = self.target_layers.clone();
        let parameters = instance.parameters().clone();
        self.rerender(generation, target_layers, parameters, globals)
    }

    fn rerender(
        &self,
        generation: u64,
        target_layers: Vec<LayerId>,
        parameters: IndexMap<EffectInputSlotId, FilterParameter>,
        globals: &mut Globals,
    ) -> Task<FilterPanelMessage> {
        let Some(renderer) = self.renderer.as_ref() else {
            return Task::none();
        };
        let device = globals.render_device().clone();
        let queue = globals.render_queue().clone();
        let tile_storage = globals.tile_storage().clone();
        renderer
            .run(target_layers, parameters, &tile_storage, &device, &queue)
            .map(move |result| FilterPanelMessage::RenderFinished(generation, result))
    }

    fn render_finished(
        &mut self,
        generation: u64,
        result: Result<HashMap<LayerId, DynamicLayerStorage>>,
        globals: &mut Globals,
    ) -> Task<FilterPanelMessage> {
        if generation != self.generation {
            return Task::none();
        }
        self.rendering = false;
        let results = match result {
            Ok(results) => results,
            Err(e) => {
                log::error!("Filter render failed: {e}");
                return Task::none();
            }
        };
        let Some(canvas_id) = self.canvas_id else {
            return Task::none();
        };
        let Some(canvas) = globals.canvas(&canvas_id) else {
            log::warn!("Filter preview canvas no longer exists; dropping results");
            return Task::none();
        };
        let dirty_tiles = self.preview_dirty_tiles(canvas);
        {
            let overriders = globals.global_mut::<LayerPreviewOverriders>();
            for (layer_id, storage) in &results {
                overriders.insert_overrider(
                    *layer_id,
                    PixelPreviewOverrider::from_layer_storage(storage),
                );
            }
        }
        self.results = results;
        self.preview_installed = true;
        CanvasUpdated::broadcast(CanvasUpdated {
            id: canvas_id,
            dirty_tiles,
        });
        Task::none()
    }

    fn confirm(&mut self, globals: &mut Globals) -> Task<FilterPanelMessage> {
        self.generation += 1;
        self.rendering = false;
        let Some(canvas_id) = self.canvas_id else {
            return Task::none();
        };
        let device = globals.render_device().clone();
        let queue = globals.render_queue().clone();
        let results = mem::take(&mut self.results);
        self.remove_previews(globals);
        self.preview_installed = false;
        let target_layers = mem::take(&mut self.target_layers);

        let mut commands = Vec::<TileReplaceCommand>::new();
        {
            let tiles = globals.tile_storage();
            for layer_id in &target_layers {
                let Some(result) = results.get(layer_id) else {
                    continue;
                };
                let Some(result_texture) = result.texture().cloned() else {
                    continue;
                };
                let Some(original) = tiles.get_layer(*layer_id) else {
                    continue;
                };
                commands.push(TileReplaceCommand::new(
                    "Filter".into(),
                    canvas_id,
                    &device,
                    &queue,
                    *layer_id,
                    &original,
                    result.iter_tile_indices().collect(),
                    result_texture,
                ));
            }
        }

        if !commands.is_empty() {
            let batched = BatchedUndoCommand::new("Filter".into(), commands);
            if let Err(e) = globals.push_undo_command(&canvas_id, batched) {
                log::error!("Failed to push filter undo command: {e}");
            }
        }
        self.canvas_id = None;
        close(self.main_window)
    }

    fn cancel(&mut self, globals: &mut Globals) -> Task<FilterPanelMessage> {
        self.cancel_internal(globals)
    }

    fn window_closed(&mut self, globals: &mut Globals) -> Task<FilterPanelMessage> {
        self.cancel_internal(globals)
    }

    fn cancel_internal(&mut self, globals: &mut Globals) -> Task<FilterPanelMessage> {
        self.generation += 1;
        self.rendering = false;
        self.remove_previews(globals);
        self.preview_installed = false;
        if let Some(canvas_id) = self.canvas_id
            && let Some(canvas) = globals.canvas(&canvas_id)
        {
            let dirty_tiles = self.preview_dirty_tiles(canvas);
            CanvasUpdated::broadcast(CanvasUpdated {
                id: canvas_id,
                dirty_tiles,
            });
        }
        self.results.clear();
        self.target_layers.clear();
        self.canvas_id = None;
        close(self.main_window)
    }

    fn preview_dirty_tiles(&self, canvas: &CCanvas) -> bevy_math::IRect {
        let mut dirty_tiles = GpuTileStorage::pixel_rect_to_tile(canvas.image.image_tile_rect());
        for storage in self.results.values() {
            dirty_tiles = dirty_tiles.union(storage.compute_tile_bounds());
        }
        dirty_tiles
    }

    fn remove_previews(&mut self, globals: &mut Globals) {
        if !self.preview_installed {
            return;
        }
        let overriders = globals.global_mut::<LayerPreviewOverriders>();
        for layer_id in &self.target_layers {
            overriders.remove_overrider(layer_id);
        }
    }
}

fn resolve_target_layers(canvas: &CCanvas) -> Vec<LayerId> {
    let layer_stack = canvas.image.layer_stack();
    let mut targets = Vec::new();
    for &layer_id in canvas.selected_layer_ids() {
        let Some(node) = layer_stack.get_layer(&layer_id) else {
            continue;
        };
        let props = node.properties();
        let Some(texel) = props.get_texel_type() else {
            log::warn!("Filter: skipping non-pixel layer {layer_id}");
            continue;
        };
        if props.locked() {
            log::warn!("Filter: skipping locked layer {layer_id}");
            continue;
        }
        if texel != TexelType::RGBA8 {
            log::warn!("Filter: skipping non-Rgba8 layer {layer_id}");
            continue;
        }
        targets.push(layer_id);
    }
    targets
}
