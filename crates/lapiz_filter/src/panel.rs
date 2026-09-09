use std::{collections::HashMap, sync::Arc};

use anyhow::Result;
use iced_core::{Alignment, Length, Size, Theme, keyboard, window};
use iced_futures::Subscription;
use iced_runtime::Task;
use iced_widget::{Space, column, row};
use lapiz_assets::{AssetAppExt, asset::AssetHandle};
use lapiz_canvas::{
    CCanvas, CanvasAppExt, CanvasId, CanvasUndoStackAppExt, command::TileReplaceCommand,
    event::CanvasUpdated,
};
use lapiz_i18n::t;
use lapiz_image::{
    composite::{LayerPreviewOverriders, PixelPreviewOverrider},
    layer::{
        LayerId,
        properties::{LayerTexelTypePropertyExt, LockedPropertyExt},
    },
    texel::TexelType,
    tile::{DynamicLayerStorage, GpuTileStorage, TileStorageAppExt},
};
use lapiz_render::render_context::RenderContextAppExt;
use lapiz_runtime::{
    Services,
    event::Event,
    windows::{OpenWindowViewCommand, WindowCommandBuffer, WindowView, WindowViewId},
};
use lapiz_shader_graph::graph::{
    external::ExternalVariableId, slot::ErasedGraphLiteralUpdateMessage,
};
use lapiz_undo::BatchedUndoCommand;
use lapiz_widgets::{button::Button, label::Label, panel::Panel, scrollable::Scrollable};

use crate::{asset::FilterPreset, instance::FilterInstance, render::FilterRenderer};

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

pub enum FilterPanelMessage {
    FilterSelected(usize),
    ExternalVarUpdated(ErasedGraphLiteralUpdateMessage),
    NewFilter,
    EditFilter,
    Confirm,
    Cancel,
    RenderFinished(u64, Result<HashMap<LayerId, DynamicLayerStorage>>),
    WindowClosed,
}

impl Clone for FilterPanelMessage {
    fn clone(&self) -> Self {
        match self {
            FilterPanelMessage::FilterSelected(i) => FilterPanelMessage::FilterSelected(*i),
            FilterPanelMessage::ExternalVarUpdated(m) => {
                FilterPanelMessage::ExternalVarUpdated(m.clone())
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
        }
    }
}

type Element<'a> = iced_core::Element<'a, FilterPanelMessage, Theme, lapiz_runtime::Renderer>;

impl WindowView for FilterPanel {
    type Message = FilterPanelMessage;

    fn id() -> WindowViewId {
        WindowViewId::new("filter_panel")
    }

    fn boot(services: &mut Services) -> (Self, Task<Self::Message>) {
        let mut filters = services
            .assets()
            .all_handles_of::<FilterPreset>()
            .expect("Failed to list filter presets");
        filters.sort_by(|a, b| {
            let a_name = a.get().map(|f| f.metadata.name.clone()).unwrap_or_default();
            let b_name = b.get().map(|f| f.metadata.name.clone()).unwrap_or_default();
            a_name.cmp(&b_name)
        });
        let (main_window, open) = iced_runtime::window::open(window::Settings {
            size: Size {
                width: 720.0,
                height: 480.0,
            },
            ..Default::default()
        });
        (
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
        )
    }

    fn view<'a>(&'a self, _: window::Id, _: &'a Services) -> impl Into<Element<'a>> {
        let filter_list = self
            .filters
            .iter()
            .enumerate()
            .map(|(index, handle)| {
                let name = handle
                    .get()
                    .map(|f| f.metadata.name.clone())
                    .unwrap_or_else(|_| "<loading>".to_string());
                Button::new(Label::new(name))
                    .width(Length::Fill)
                    .on_press(FilterPanelMessage::FilterSelected(index))
                    .into()
            })
            .collect::<Vec<_>>();

        let sidebar = Panel::new(
            column![
                Label::new(t!("filters")).strong(),
                Scrollable::new(column(filter_list).spacing(2))
                    .width(Length::Fill)
                    .height(Length::Fill),
                row![
                    Button::new(Label::new(t!("new_filter")))
                        .on_press(FilterPanelMessage::NewFilter),
                    Button::new(Label::new(t!("edit_filter")))
                        .on_press(FilterPanelMessage::EditFilter),
                ]
                .spacing(4),
            ]
            .spacing(6),
        )
        .padding(8)
        .width(220);

        let params = if let Some(selected) = self.selected.as_ref() {
            let variable_rows = selected
                .iter_external_vars()
                .map(|(id, variable)| {
                    row![
                        Label::new(variable.name.clone()).width(Length::Fill),
                        variable
                            .value
                            .ty()
                            .view_literal((*id).into(), variable.value.value())
                            .map(FilterPanelMessage::ExternalVarUpdated),
                    ]
                    .spacing(6)
                    .into()
                })
                .collect::<Vec<_>>();
            if variable_rows.is_empty() {
                column![Label::new(t!("no_external_variables")).muted()].spacing(6)
            } else {
                column![
                    Label::new(t!("parameters")).strong(),
                    Scrollable::new(column(variable_rows).spacing(6))
                        .width(Length::Fill)
                        .height(Length::Fill),
                ]
                .spacing(6)
            }
        } else {
            column![Label::new(t!("select_a_filter_to_adjust")).muted()]
        };

        let ok_enabled = self.selected.is_some() && !self.rendering;
        let ok_label = if self.rendering { "Rendering..." } else { "OK" };

        let footer = row![
            Space::new().width(Length::Fill),
            Button::new(Label::new(t!("cancel"))).on_press(FilterPanelMessage::Cancel),
            Button::new(Label::new(ok_label))
                .primary()
                .on_press_maybe(ok_enabled.then_some(FilterPanelMessage::Confirm)),
        ]
        .align_y(Alignment::Center)
        .spacing(10)
        .padding(16);

        column![
            row![sidebar, Panel::new(params).padding(8).width(Length::Fill)].height(Length::Fill),
            footer,
        ]
        .width(Length::Fill)
        .height(Length::Fill)
    }

    fn update(
        &mut self,
        message: Self::Message,
        services: &mut Services,
    ) -> impl Into<Task<Self::Message>> {
        match message {
            FilterPanelMessage::FilterSelected(index) => self.filter_selected(index, services),
            FilterPanelMessage::ExternalVarUpdated(message) => {
                self.external_var_updated(message, services)
            }
            FilterPanelMessage::NewFilter | FilterPanelMessage::EditFilter => {
                services
                    .service_mut::<WindowCommandBuffer>()
                    .push(OpenWindowViewCommand::new(WindowViewId::new(
                        "filter_editor",
                    )));
                Task::none()
            }
            FilterPanelMessage::Confirm => self.confirm(services),
            FilterPanelMessage::Cancel => self.cancel(services),
            FilterPanelMessage::RenderFinished(generation, result) => {
                self.render_finished(generation, result, services)
            }
            FilterPanelMessage::WindowClosed => self.window_closed(services),
        }
    }

    fn subscription(&self, _services: &Services) -> Subscription<Self::Message> {
        let main_window = self.main_window;
        iced_futures::subscription::filter_map(("filter_panel", main_window), move |event| {
            match event {
                iced_futures::subscription::Event::Interaction {
                    window,
                    event: iced_core::Event::Window(iced_core::window::Event::Closed),
                    status: _,
                } if window == main_window => Some(FilterPanelMessage::WindowClosed),
                iced_futures::subscription::Event::Interaction {
                    window,
                    event:
                        iced_core::Event::Keyboard(keyboard::Event::KeyPressed {
                            key, modifiers, ..
                        }),
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
            }
        })
    }

    fn close(self, _: &mut Services) -> Task<()> {
        iced_runtime::window::close(self.main_window)
    }

    fn windows(&self) -> Arc<[window::Id]> {
        self.windows.clone()
    }

    fn root_window(&self) -> Option<window::Id> {
        Some(self.main_window)
    }
}

impl FilterPanel {
    fn filter_selected(
        &mut self,
        index: usize,
        services: &mut Services,
    ) -> Task<FilterPanelMessage> {
        let Some(handle) = self.filters.get(index).cloned() else {
            return Task::none();
        };
        let Some(canvas) = services.current_canvas() else {
            return Task::none();
        };
        let canvas_id = canvas.id();
        let target_layers = resolve_target_layers(canvas);
        let removing_preview = self.preview_installed;
        let preview_canvas_id = self.canvas_id.unwrap_or(canvas_id);
        let dirty_tiles = if removing_preview {
            self.canvas_id
                .and_then(|preview_id| services.canvas(&preview_id))
                .map(|preview_canvas| self.preview_dirty_tiles(preview_canvas))
        } else {
            None
        };
        if target_layers.is_empty() {
            log::warn!("Filter: no eligible target layers on current canvas");
            self.selected = None;
            self.renderer = None;
            self.results.clear();
            self.remove_previews(services);
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
        self.remove_previews(services);
        self.preview_installed = false;
        if let Some(dirty_tiles) = dirty_tiles {
            CanvasUpdated::broadcast(CanvasUpdated {
                id: preview_canvas_id,
                dirty_tiles,
            });
        }
        self.target_layers = target_layers.clone();
        self.canvas_id = Some(canvas_id);

        let (instance, errors) = FilterInstance::from_asset(&handle, services);
        for error in errors {
            log::error!("Failed to load filter preset: {error}");
        }
        let Some(instance) = instance else {
            log::error!("Failed to load filter preset");
            self.rendering = false;
            self.selected = None;
            self.renderer = None;
            return Task::none();
        };
        let compiled = match instance.compile() {
            Ok(compiled) => compiled,
            Err(e) => {
                log::error!("Failed to compile filter preset: {e}");
                self.rendering = false;
                self.selected = None;
                self.renderer = None;
                return Task::none();
            }
        };
        let renderer = match FilterRenderer::new(services, compiled) {
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
        self.selected = Some(instance);
        self.renderer = Some(renderer);
        let renderer = self.renderer.as_ref().unwrap();
        renderer
            .run(services, canvas_id, target_layers)
            .map(move |result| FilterPanelMessage::RenderFinished(generation, result))
    }

    fn external_var_updated(
        &mut self,
        message: ErasedGraphLiteralUpdateMessage,
        services: &mut Services,
    ) -> Task<FilterPanelMessage> {
        if let Some(instance) = self.selected.as_mut() {
            let id = ExternalVariableId::new(*message.id);
            instance.update_external_var(&id, message);
        }
        let Some(canvas_id) = self.canvas_id else {
            return Task::none();
        };
        if self.target_layers.is_empty() {
            return Task::none();
        }
        let Some(renderer) = self.renderer.as_ref() else {
            return Task::none();
        };
        self.generation += 1;
        self.rendering = true;
        let generation = self.generation;
        let target_layers = self.target_layers.clone();
        renderer
            .run(services, canvas_id, target_layers)
            .map(move |result| FilterPanelMessage::RenderFinished(generation, result))
    }

    fn render_finished(
        &mut self,
        generation: u64,
        result: Result<HashMap<LayerId, DynamicLayerStorage>>,
        services: &mut Services,
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
        let Some(canvas) = services.canvas(&canvas_id) else {
            log::warn!("Filter preview canvas no longer exists; dropping results");
            return Task::none();
        };
        let dirty_tiles = self.preview_dirty_tiles(canvas);
        {
            let overriders = services.service_mut::<LayerPreviewOverriders>();
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

    fn confirm(&mut self, services: &mut Services) -> Task<FilterPanelMessage> {
        self.generation += 1;
        self.rendering = false;
        let Some(canvas_id) = self.canvas_id else {
            return Task::none();
        };
        let device = services.render_device().clone();
        let queue = services.render_queue().clone();
        let results = std::mem::take(&mut self.results);
        self.remove_previews(services);
        self.preview_installed = false;
        let target_layers = std::mem::take(&mut self.target_layers);

        let mut commands: Vec<TileReplaceCommand> = Vec::new();
        {
            let tiles = services.tile_storage();
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
            if let Err(e) = services.push_undo_command(&canvas_id, batched) {
                log::error!("Failed to push filter undo command: {e}");
            }
        }
        self.canvas_id = None;
        iced_runtime::window::close(self.main_window)
    }

    fn cancel(&mut self, services: &mut Services) -> Task<FilterPanelMessage> {
        self.cancel_internal(services)
    }

    fn window_closed(&mut self, services: &mut Services) -> Task<FilterPanelMessage> {
        self.cancel_internal(services)
    }

    fn cancel_internal(&mut self, services: &mut Services) -> Task<FilterPanelMessage> {
        self.generation += 1;
        self.rendering = false;
        self.remove_previews(services);
        self.preview_installed = false;
        if let Some(canvas_id) = self.canvas_id
            && let Some(canvas) = services.canvas(&canvas_id)
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
        iced_runtime::window::close(self.main_window)
    }

    fn preview_dirty_tiles(&self, canvas: &CCanvas) -> bevy_math::IRect {
        let mut dirty_tiles = GpuTileStorage::pixel_rect_to_tile(canvas.image.image_tile_rect());
        for storage in self.results.values() {
            dirty_tiles = dirty_tiles.union(storage.compute_tile_bounds());
        }
        dirty_tiles
    }

    fn remove_previews(&mut self, services: &mut Services) {
        if !self.preview_installed {
            return;
        }
        let overriders = services.service_mut::<LayerPreviewOverriders>();
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
