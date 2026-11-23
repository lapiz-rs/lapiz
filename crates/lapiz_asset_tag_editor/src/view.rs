use std::{collections::HashMap, path::Path, sync::Arc};

use anyhow::{Context as _, Result};
use iced_core::{Element, Event, Length, Size, Theme, window};
use iced_futures::{Subscription, event::listen_with};
use iced_runtime::{
    Task,
    window::{close, drag, minimize, open, toggle_maximize},
};
use lapiz_assets::{
    AssetAppExt as _,
    asset::{Asset as _, AssetMetadata, UntypedAssetId},
    tag::{PositionInTag, TagFile, TagId},
};
use lapiz_brush::{
    engine::{BrushEngineId, BrushEngineRegistry, BrushEngineRegistryExt as _},
    widget::BrushPreviewCompiled,
};
use lapiz_i18n::t;
use lapiz_render::texture::{Image, read_or_create_cached_thumbnail};
use lapiz_runtime::{
    Renderer,
    global::Globals,
    windows::{WindowView, WindowViewId},
};
use lapiz_widgets::{
    button, column,
    drag_drop::{DragDrop, DragRegion, DropMode, DropZone},
    flex::Flex,
    icon, icon_button, image, label, panel, row, scrollable,
    splitter::Splitter,
    tag, text_input, title_bar, tooltip,
    tooltip::Position,
};

use crate::{
    ASSET_TAG_EDITOR_VIEW_ID,
    model::{AssetList, DragItem, EditorModel, matches_search},
};

pub struct AssetTagEditor {
    window: window::Id,
    model: EditorModel,
    asset_types: Vec<String>,
    names: HashMap<UntypedAssetId, String>,
    engines: HashMap<UntypedAssetId, BrushEngineId>,
    searches: [String; 2],
    ratios: [f32; 3],
    preview: Option<image::Handle>,
    preview_generation: u64,
    extra_metadata: Vec<(String, String)>,
    asset_tags: Vec<TagId>,
    error: Option<String>,
    tag_form: Option<TagForm>,
}

struct TagForm {
    parent: TagId,
    name: String,
}

#[derive(Clone)]
pub enum Message {
    SelectType(String),
    SelectTag(TagId),
    ToggleTag(TagId),
    SelectAsset(UntypedAssetId),
    Search(AssetList, String),
    ClearSearch(AssetList),
    Drop(DragItem, DragItem, DropZone),
    Resize(usize, f32),
    BeginTag(bool),
    TagName(String),
    CreateTag,
    CancelTag,
    RemoveTag,
    Refresh,
    Focused(window::Id),
    ClearError,
    Close,
    Maximize,
    Minimize,
    Drag,
}

impl WindowView for AssetTagEditor {
    type Message = Message;
    type BootParams = ();

    fn id() -> WindowViewId {
        WindowViewId::new(ASSET_TAG_EDITOR_VIEW_ID)
    }

    fn boot(_: Option<()>, globals: &mut Globals) -> Result<(Self, Task<Message>)> {
        let asset_types = globals
            .assets()
            .serializers()
            .asset_types()
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let mut model = EditorModel {
            current_asset_ty: asset_types.first().cloned(),
            ..Default::default()
        };
        model.refresh(globals.assets())?;
        let (window, task) = open(window::Settings {
            decorations: false,
            size: Size::new(1440.0, 900.0),
            min_size: Some(Size::new(1000.0, 640.0)),
            ..Default::default()
        });
        let mut editor = Self {
            window,
            model,
            asset_types,
            names: HashMap::new(),
            engines: HashMap::new(),
            searches: Default::default(),
            ratios: [0.18, 0.76, 0.5],
            preview: None,
            preview_generation: 0,
            extra_metadata: Vec::new(),
            asset_tags: Vec::new(),
            error: None,
            tag_form: None,
        };
        editor.refresh_names(globals);
        Ok((editor, task.discard()))
    }

    fn view<'a>(
        &'a self,
        _: window::Id,
        globals: &'a Globals,
    ) -> impl Into<Element<'a, Message, Theme, Renderer>> {
        let title =
            title_bar(row![icon::folder_open(), label(t!("title")).window_title()].gap(8.0))
                .on_close(Message::Close)
                .on_maximize(Message::Maximize)
                .on_minimize(Message::Minimize)
                .on_drag(Message::Drag);
        let lists = Splitter::horizontal(
            self.asset_list(AssetList::All),
            self.asset_list(AssetList::Tagged),
            self.ratios[2],
        )
        .on_resize(|ratio| Message::Resize(2, ratio));
        let content = Splitter::horizontal(lists, self.inspector(globals), self.ratios[1])
            .on_resize(|ratio| Message::Resize(1, ratio));
        let body = Splitter::horizontal(self.sidebar(), content, self.ratios[0])
            .on_resize(|ratio| Message::Resize(0, ratio));
        let body = DragDrop::new(
            body,
            |source, target| self.model.accepts(source, target),
            Message::Drop,
        );
        let mut layout = column![title, body].gap(0.0).height(Length::Fill);
        if let Some(error) = &self.error {
            layout = layout.push(
                panel(
                    row![
                        icon::warning().danger(),
                        label(error).width(Length::Fill),
                        icon_button(icon::close()).on_press(Message::ClearError)
                    ]
                    .gap(8.0),
                )
                .padding(8)
                .width(Length::Fill),
            );
        }
        layout = layout.push(
            panel(
                row![
                    label(t!("drag_help")).muted().size(11),
                    icon_button(icon::refresh()).on_press(Message::Refresh)
                ]
                .space_between(),
            )
            .padding([4, 8])
            .width(Length::Fill),
        );
        panel(layout).width(Length::Fill).height(Length::Fill)
    }

    fn update(&mut self, message: Message, globals: &mut Globals) -> impl Into<Task<Message>> {
        match message {
            Message::SelectType(ty) => {
                self.model.current_asset_ty = Some(ty);
                self.model.current_tag = None;
                self.model.selected_asset = None;
                self.searches = Default::default();
                self.tag_form = None;
                self.clear_preview();
                let result = self.model.refresh(globals.assets());
                self.record(result);
            }
            Message::SelectTag(id) => {
                self.model.current_tag = Some(id);
                self.searches[1].clear();
                let result = self.model.refresh(globals.assets());
                self.record(result);
            }
            Message::ToggleTag(id) => {
                let expanded = self.model.is_tag_expanded(id);
                self.model.set_tag_expanded(id, !expanded);
            }
            Message::SelectAsset(id) => {
                self.model.selected_asset = Some(id);
                return self.load_preview(globals);
            }
            Message::Search(list, query) => self.searches[Self::list_index(list)] = query,
            Message::ClearSearch(list) => self.searches[Self::list_index(list)].clear(),
            Message::Drop(source, target, zone) => {
                let result = self.model.drop_item(source, target, zone, globals.assets());
                self.record(result);
                self.refresh_asset_tags(globals);
            }
            Message::Resize(index, ratio) => {
                let (min, max) = match index {
                    0 => (0.13, 0.3),
                    1 => (0.55, 0.85),
                    _ => (0.25, 0.75),
                };
                self.ratios[index] = ratio.clamp(min, max);
            }
            Message::BeginTag(child) => {
                if let Some(parent) = if child {
                    self.model.current_tag
                } else {
                    self.model.root
                } {
                    self.tag_form = Some(TagForm {
                        parent,
                        name: String::new(),
                    });
                }
            }
            Message::TagName(name) => {
                if let Some(form) = &mut self.tag_form {
                    form.name = name;
                }
            }
            Message::CreateTag => {
                let result = self.create_tag(globals);
                self.record(result);
            }
            Message::CancelTag => self.tag_form = None,
            Message::RemoveTag => {
                if let (Some(asset), Some(tag)) =
                    (self.model.selected_asset, self.model.current_tag)
                {
                    let result = globals
                        .assets()
                        .remove_asset_from_tag(asset, tag)
                        .and_then(|()| self.model.refresh(globals.assets()));
                    self.record(result);
                    self.refresh_asset_tags(globals);
                }
            }
            Message::Focused(id) if id != self.window => {}
            Message::Refresh | Message::Focused(_) => {
                self.refresh_names(globals);
                let result = self.model.refresh(globals.assets());
                self.record(result);
                if self.model.selected_asset.is_some() {
                    return self.load_preview(globals);
                }
                self.clear_preview();
            }
            Message::ClearError => self.error = None,
            Message::Close => return close(self.window),
            Message::Maximize => return toggle_maximize(self.window),
            Message::Minimize => return minimize(self.window, true),
            Message::Drag => return drag(self.window),
        }
        Task::none()
    }

    fn subscription(&self, _: &Globals) -> Subscription<Message> {
        listen_with(|event, _, id| {
            matches!(event, Event::Window(window::Event::Focused)).then_some(Message::Focused(id))
        })
    }

    fn close(self, _: &mut Globals) -> Task<()> {
        close(self.window)
    }
    fn windows(&self) -> Arc<[window::Id]> {
        [self.window].into()
    }
    fn root_window(&self) -> Option<window::Id> {
        Some(self.window)
    }
}

impl AssetTagEditor {
    fn list_index(list: AssetList) -> usize {
        match list {
            AssetList::All => 0,
            AssetList::Tagged => 1,
        }
    }

    fn sidebar(&self) -> Element<'_, Message, Theme, Renderer> {
        let types = self
            .asset_types
            .iter()
            .map(|ty| {
                button(
                    row![
                        icon::folder(),
                        label(ty.replace('_', " ")).width(Length::Fill)
                    ]
                    .gap(8.0),
                )
                .width(Length::Fill)
                .height(34)
                .clip(true)
                .activated(self.model.current_asset_ty.as_ref() == Some(ty))
                .on_press(Message::SelectType(ty.clone()))
                .into()
            })
            .collect::<Vec<_>>();
        let rows = self
            .model
            .rows()
            .into_iter()
            .map(|entry| {
                let tag = &self.model.tags[&entry.id];
                let expanded = self.model.is_tag_expanded(entry.id);
                let toggle = if self.model.has_children(entry.id) {
                    Element::from(
                        icon_button(if expanded {
                            icon::caret_down()
                        } else {
                            icon::caret_right()
                        })
                        .on_press(Message::ToggleTag(entry.id)),
                    )
                } else {
                    panel(label("")).width(24).transparent().into()
                };
                let name = row![icon::folder(), label(&tag.name).width(Length::Fill)].gap(6.0);
                let row = row![
                    toggle,
                    button(name)
                        .width(Length::Fill)
                        .height(30)
                        .clip(true)
                        .activated(self.model.current_tag == Some(entry.id))
                        .on_press(Message::SelectTag(entry.id))
                ]
                .gap(2.0);
                let row = panel(row)
                    .transparent()
                    .padding(iced_core::Padding {
                        left: entry.depth as f32 * 14.0,
                        ..Default::default()
                    })
                    .width(Length::Fill);
                DragRegion::new(
                    DragItem::Tag {
                        id: entry.id,
                        parent: entry.parent,
                    },
                    row,
                )
                .mode(DropMode::Tree)
                .into()
            })
            .collect::<Vec<_>>();
        let mut tree = Flex::column(rows).gap(2.0);
        if self.model.rows().is_empty() {
            tree = tree.push(label(t!("no_tags")).muted());
        }
        tree = tree.push(
            DragRegion::new(
                DragItem::TagRoot,
                panel(label(t!("root_drop")).faint().size(11))
                    .padding(10)
                    .width(Length::Fill)
                    .transparent(),
            )
            .draggable(false)
            .mode(DropMode::Into),
        );
        let actions = row![
            tooltip(
                icon_button(icon::plus()).on_press(Message::BeginTag(false)),
                label(t!("new_tag")),
                Position::Bottom
            ),
            tooltip(
                icon_button(icon::folder())
                    .on_press_maybe(self.model.current_tag.map(|_| Message::BeginTag(true))),
                label(t!("new_child")),
                Position::Bottom
            ),
        ]
        .gap(4.0);
        let mut content = column![
            label(t!("asset_types")).strong(),
            scrollable(Flex::column(types).gap(3.0)).height(180),
            row![label(t!("tags")).strong().width(Length::Fill), actions],
            scrollable(tree).height(Length::Fill)
        ]
        .gap(10.0)
        .height(Length::Fill);
        if let Some(form) = &self.tag_form {
            let parent = &self.model.tags[&form.parent];
            let heading = if parent.is_type_tag() {
                t!("new_tag")
            } else {
                format!("{} / {}", parent.name, t!("new_child"))
            };
            content = content.push(
                panel(
                    column![
                        label(heading).strong(),
                        text_input(&t!("tag_name"), &form.name)
                            .on_input(Message::TagName)
                            .on_submit(Message::CreateTag),
                        row![
                            button(label(t!("create"))).primary().on_press_maybe(
                                (!form.name.trim().is_empty()).then_some(Message::CreateTag)
                            ),
                            button(label(t!("cancel"))).on_press(Message::CancelTag)
                        ]
                        .gap(6.0)
                    ]
                    .gap(8.0),
                )
                .padding(8)
                .width(Length::Fill),
            );
        }
        panel(content)
            .padding(10)
            .width(Length::Fill)
            .height(Length::Fill)
            .borderless()
            .into()
    }

    fn asset_list(&self, list: AssetList) -> Element<'_, Message, Theme, Renderer> {
        let index = Self::list_index(list);
        let assets = match list {
            AssetList::All => &self.model.all_assets,
            AssetList::Tagged => &self.model.tagged_assets,
        };
        let filtered = assets
            .iter()
            .filter(|asset| matches_search(asset, &self.asset_name(asset), &self.searches[index]))
            .collect::<Vec<_>>();
        let title = match list {
            AssetList::All => t!("all_assets"),
            AssetList::Tagged => self
                .model
                .current_tag
                .and_then(|id| self.model.tags.get(&id))
                .map(|tag| tag.name.clone())
                .unwrap_or_else(|| t!("tagged_assets")),
        };
        let count = if self.searches[index].is_empty() {
            assets.len().to_string()
        } else {
            format!("{} / {}", filtered.len(), assets.len())
        };
        let header = row![label(title).strong().width(Length::Fill), tag(count)].gap(8.0);
        let search = row![
            icon::search().muted(),
            text_input(&t!("search"), &self.searches[index])
                .width(Length::Fill)
                .on_input(move |query| Message::Search(list, query)),
            icon_button(icon::close()).on_press_maybe(
                (!self.searches[index].is_empty()).then_some(Message::ClearSearch(list))
            )
        ]
        .gap(6.0);
        let rows = filtered
            .iter()
            .map(|asset| {
                let name = self.asset_name(asset);
                let row = row![
                    icon::grip().muted(),
                    icon::file().muted(),
                    column![
                        label(name).strong(),
                        label(&asset.relative_path).muted().size(11)
                    ]
                    .gap(3.0)
                    .width(Length::Fill)
                ]
                .gap(8.0);
                let item = button(row)
                    .width(Length::Fill)
                    .height(54)
                    .padding(8)
                    .clip(true)
                    .activated(self.model.selected_asset == Some(asset.asset_id))
                    .on_press(Message::SelectAsset(asset.asset_id));
                DragRegion::new(DragItem::Asset(asset.asset_id, list), item).into()
            })
            .collect::<Vec<_>>();
        let mut items = Flex::column(rows).gap(3.0).width(Length::Fill);
        if filtered.is_empty() {
            let message = if list == AssetList::Tagged && self.model.current_tag.is_none() {
                t!("select_tag")
            } else if !self.searches[index].is_empty() {
                t!("no_results")
            } else if list == AssetList::Tagged {
                t!("drop_assets")
            } else {
                t!("no_assets")
            };
            items = items.push(
                panel(
                    column![icon::folder_open().size(32).muted(), label(message).muted()].gap(12.0),
                )
                .transparent()
                .padding(24)
                .width(Length::Fill),
            );
        }
        items = items.push(
            DragRegion::new(
                DragItem::List(list),
                panel(
                    label(if list == AssetList::Tagged {
                        t!("drop_to_tag")
                    } else {
                        t!("drop_to_end")
                    })
                    .faint()
                    .size(11),
                )
                .transparent()
                .padding(14)
                .width(Length::Fill),
            )
            .draggable(false)
            .mode(DropMode::Into),
        );
        let area = scrollable(items).height(Length::Fill);
        panel(column![header, search, area].gap(10.0).height(Length::Fill))
            .padding(12)
            .width(Length::Fill)
            .height(Length::Fill)
            .borderless()
            .into()
    }

    fn inspector<'a>(&'a self, globals: &'a Globals) -> Element<'a, Message, Theme, Renderer> {
        let Some(metadata) = self.model.selected_metadata() else {
            return panel(
                column![
                    label(t!("details")).strong(),
                    panel(
                        column![
                            icon::info().size(32).muted(),
                            label(t!("select_asset")).muted()
                        ]
                        .gap(12.0)
                    )
                    .transparent()
                    .padding(20)
                ]
                .gap(16.0),
            )
            .padding(12)
            .width(Length::Fill)
            .height(Length::Fill)
            .borderless()
            .into();
        };
        let mut content = column![
            label(t!("details")).strong(),
            label(self.asset_name(metadata)).strong().size(18)
        ]
        .gap(12.0);
        if let Some(preview) = &self.preview {
            content = content.push(
                panel(
                    image(preview.clone())
                        .width(Length::Fill)
                        .height(180)
                        .content_fit(iced_core::ContentFit::Contain),
                )
                .inset()
                .padding(8)
                .width(Length::Fill),
            );
        } else if let Some(engine_id) = self.engines.get(&metadata.asset_id) {
            let preview = globals
                .brush_engine_registry()
                .get(engine_id)
                .and_then(|factory| {
                    let engine =
                        factory.load_brush_from_assets(metadata.asset_id, globals.assets())?;
                    engine.compile_brush(globals)
                });
            let preview = match preview {
                Ok(brush) => Element::<'_, Message, Theme, Renderer>::from(
                    BrushPreviewCompiled::new(brush, Some(metadata.asset_id), globals)
                        .generation(self.preview_generation),
                ),
                Err(error) => Element::from(label(format!("{error:#}")).muted()),
            };
            content = content.push(panel(preview).inset().padding(8).width(Length::Fill));
        }
        let bundle = globals
            .assets()
            .bundle(&metadata.bundle_id)
            .map(|bundle| bundle.metadata().name.clone())
            .unwrap_or_else(|_| metadata.bundle_id.to_string());
        let fields = [
            (t!("type"), metadata.ty.clone()),
            (t!("bundle"), bundle),
            (t!("path"), metadata.relative_path.clone()),
            (t!("revision"), metadata.revision.to_string()),
            (
                t!("modified"),
                metadata
                    .last_modified
                    .format("%Y-%m-%d %H:%M UTC")
                    .to_string(),
            ),
            (
                t!("state"),
                if metadata.in_memory {
                    t!("unsaved")
                } else {
                    t!("saved")
                },
            ),
        ];
        for (key, value) in fields
            .into_iter()
            .chain(self.extra_metadata.iter().cloned())
        {
            content = content.push(column![label(key).muted().size(11), label(value)].gap(4.0));
        }
        content = content.push(label(t!("tags")).strong());
        let tags = self
            .asset_tags
            .iter()
            .filter_map(|id| self.model.tags.get(id))
            .filter(|tag| !tag.is_type_tag())
            .map(|entry| tag(&entry.name).into())
            .collect::<Vec<_>>();
        if tags.is_empty() {
            content = content.push(label(t!("untagged")).muted());
        } else {
            content = content.push(Flex::row(tags).wrap().gap(4.0));
        }
        if self
            .model
            .current_tag
            .is_some_and(|id| self.asset_tags.contains(&id))
        {
            content = content.push(
                button(row![icon::minus(), label(t!("remove_tag"))].gap(6.0))
                    .outline()
                    .width(Length::Fill)
                    .height(34)
                    .on_press(Message::RemoveTag),
            );
        }
        panel(scrollable(content).height(Length::Fill))
            .padding(12)
            .width(Length::Fill)
            .height(Length::Fill)
            .borderless()
            .into()
    }

    fn asset_name(&self, asset: &AssetMetadata) -> String {
        self.names.get(&asset.asset_id).cloned().unwrap_or_else(|| {
            Path::new(&asset.relative_path)
                .file_stem()
                .and_then(|name| name.to_str())
                .unwrap_or(&asset.relative_path)
                .to_owned()
        })
    }

    fn refresh_names(&mut self, globals: &Globals) {
        self.names.clear();
        self.engines.clear();
        for (engine, entry) in globals
            .global::<BrushEngineRegistry>()
            .all_brushes(globals.assets())
        {
            self.names.insert(entry.id, entry.name);
            self.engines.insert(entry.id, engine);
        }
    }

    fn clear_preview(&mut self) {
        self.preview_generation += 1;
        self.preview = None;
        self.extra_metadata.clear();
        self.asset_tags.clear();
    }

    fn refresh_asset_tags(&mut self, globals: &Globals) {
        if let Some(id) = self.model.selected_asset {
            match globals.assets().index_db().get_asset_tags(&id) {
                Ok(tags) => self.asset_tags = tags.into_iter().collect(),
                Err(error) => self.error = Some(format!("{error:#}")),
            }
        }
    }

    fn load_preview(&mut self, globals: &mut Globals) -> Task<Message> {
        self.clear_preview();
        self.refresh_asset_tags(globals);
        let Some(metadata) = self.model.selected_metadata().cloned() else {
            return Task::none();
        };
        let id = metadata.asset_id;
        let assets = globals.assets().clone();
        if metadata.ty == Image::TYPE_NAME {
            let result = assets.handle(id.into_typed::<Image>()).and_then(|handle| {
                let image = handle.get()?;
                self.names.insert(id, image.metadata.name.clone());
                self.extra_metadata = vec![
                    (
                        t!("dimensions"),
                        format!("{} × {} px", image.image.width(), image.image.height()),
                    ),
                    (t!("format"), format!("{:?}", image.format)),
                ];
                read_or_create_cached_thumbnail(&handle)
            });
            match result {
                Ok(path) => self.preview = Some(image::Handle::from_path(path)),
                Err(error) => self.error = Some(format!("{error:#}")),
            }
            return Task::none();
        }
        Task::none()
    }

    fn create_tag(&mut self, globals: &Globals) -> Result<()> {
        let form = self.tag_form.as_ref().context("No tag form open")?;
        let name = form.name.trim();
        if name.is_empty() {
            return Ok(());
        }
        let parent = form.parent;
        let root = self.model.root.context("No asset type selected")?;
        let file = TagFile::new(name.into(), self.model.current_asset_ty.clone());
        let path = format!("tags/{}.tag", file.id);
        let tag = globals
            .assets()
            .create_tag(self.model.tags[&root].bundle_id, path, file)?;
        globals.assets().add_tag_to_tag_at(
            &self.model.tags[&parent],
            &tag.id,
            PositionInTag::Last,
        )?;
        self.model.set_tag_expanded(parent, true);
        self.model.current_tag = Some(tag.id);
        self.tag_form = None;
        self.model.refresh(globals.assets())
    }

    fn record(&mut self, result: Result<()>) {
        match result {
            Ok(()) => self.error = None,
            Err(error) => self.error = Some(format!("{error:#}")),
        }
    }
}
