use std::{f32::consts::TAU, fmt};

use iced_core::{Color, Length, Theme, text::IntoFragment};
// TODO: Re-add the color picker once it is ported or vendored into `lapiz_widgets`.
use lapiz_color::model::rgb::Rgb;
use lapiz_config::Configuration;
use lapiz_i18n::{Translated, t};
use lapiz_runtime::Renderer;
use lapiz_widgets::{
    button, checkbox, column, combo_box, flex::Flex, label, panel, radio, row, scrollable, space,
    spin_slider, text_input,
};
use serde::{Deserialize, Serialize};

use crate::{ColorModel, GradientPlaneShape};

/// Persistent configuration file storing the user's list of [`ColorSelectorConfig`] presets.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColorSelectorConfigGroup {
    pub configs: Vec<ColorSelectorConfig>,
}

impl Configuration for ColorSelectorConfigGroup {
    const NAME: &'static str = "color_selector.toml";

    const DEFAULT: &'static str = include_str!("../../../default_config/color_selector.toml");
}

/// A named preset defining the layout, planes, bars, and gamut-handling options for [`ColorSelector`](crate::ColorSelector).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColorSelectorConfig {
    pub name: String,
    pub max_plane_size: u32,
    pub max_planes_per_row: usize,
    pub planes: Vec<GradientPlaneConfig>,
    pub bars: Vec<GradientBarConfig>,
    pub out_of_gamut_color: Rgb,
    pub use_out_of_gamut_color: bool,
    pub clip_to_gamut: bool,
}

/// Configuration for an individual 2D color gradient plane and its optional primary-channel ring.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GradientPlaneConfig {
    pub model: ColorModel,
    pub shape: GradientPlaneShape,
    pub variable_channels: u8,
    pub flip_axis: GradientPlaneFlipAxis,
    pub rotation: f32,
    pub show_primary_channel_ring: bool,
    pub primary_channel_ring_width: f32,
    pub ring_bar_saturated_hue_channel: bool,
    pub ring_rotation: f32,
    pub reversed_ring: bool,
}

bitflags::bitflags! {
    /// Bitflags specifying horizontal (`X`) or vertical (`Y`) axis mirroring for a gradient plane.
    #[derive(Debug, Clone, Copy, Serialize, Deserialize)]
    #[serde(transparent)]
    pub struct GradientPlaneFlipAxis : u8 {
        const X = 0b01;
        const Y = 0b10;
    }
}

/// Configuration for a single-channel 1D color gradient bar.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GradientBarConfig {
    pub model: ColorModel,
    pub channel: u8,
    pub bar_height: f32,
    pub show_channel_label: bool,
    pub show_precise_spin_box: bool,
    pub show_primary_channel_lock: bool,
}

/// UI messages emitted by [`ColorSelectorConfigEditorState`] while editing presets.
#[derive(Debug, Clone)]
pub enum ColorSelectorConfigMessage {
    ConfigSelected(usize),
    ConfigNameChanged(String),
    MaxPlaneSizeChanged(u32),
    MaxPlanesPerRowChanged(usize),
    AddConfig,
    RemoveConfig,
    MoveConfigUp,
    MoveConfigDown,
    OutOfGamutColorToggled(bool),
    OutOfGamutPickerToggled,
    OutOfGamutPickerCancelled,
    OutOfGamutColorSubmitted(Color),
    ClipToGamutToggled(bool),
    AddPlane,
    RemovePlane(usize),
    MovePlaneUp(usize),
    MovePlaneDown(usize),
    PlaneModelChanged(usize, ColorModel),
    PlaneShapeChanged(usize, GradientPlaneShape),
    PlanePrimaryChannelChanged(usize, usize),
    PlaneFlipXChanged(usize, bool),
    PlaneFlipYChanged(usize, bool),
    PlaneRotationChanged(usize, f32),
    PlaneShowRingChanged(usize, bool),
    PlaneSaturatedPrimaryChannelChanged(usize, bool),
    PlaneReversedRingChanged(usize, bool),
    PlaneRingWidthChanged(usize, f32),
    PlaneRingRotationChanged(usize, f32),
    AddBar,
    RemoveBar(usize),
    MoveBarUp(usize),
    MoveBarDown(usize),
    BarModelChanged(usize, ColorModel),
    BarHeightChanged(usize, f32),
    BarChannelChanged(usize, usize),
    BarShowChannelLabelChanged(usize, bool),
    BarShowPreciseSpinBoxChanged(usize, bool),
    BarShowPrimaryChannelLockChanged(usize, bool),

    Confirmed,
    Cancelled,
}

/// Combo-box item representing a selectable [`ColorSelectorConfig`] entry by index and name.
#[derive(Debug, Clone, PartialEq)]
struct ConfigItem {
    index: usize,
    name: String,
}

impl fmt::Display for ConfigItem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name)
    }
}

/// Interactive editor state and view builder for customizing [`ColorSelectorConfig`] presets.
///
/// Allows adding, reordering, removing, and configuring presets, their 2D [`GradientPlaneConfig`]s,
/// and their 1D [`GradientBarConfig`]s before confirming or cancelling changes.
pub struct ColorSelectorConfigEditorState {
    configs: Vec<ColorSelectorConfig>,
    selected_config: Option<usize>,
    out_of_gamut_picker_open: bool,
}

impl ColorSelectorConfigEditorState {
    pub fn new(configs: Vec<ColorSelectorConfig>, selected_config: Option<usize>) -> Self {
        let selected_config = selected_config.filter(|index| *index < configs.len());
        Self {
            configs,
            selected_config,
            out_of_gamut_picker_open: false,
        }
    }

    pub fn configs(&self) -> &[ColorSelectorConfig] {
        &self.configs
    }

    fn config_mut(&mut self) -> Option<&mut ColorSelectorConfig> {
        self.selected_config.map(|index| &mut self.configs[index])
    }

    fn plane_mut(&mut self, index: usize) -> Option<&mut GradientPlaneConfig> {
        self.config_mut()
            .and_then(|config| config.planes.get_mut(index))
    }

    fn bar_mut(&mut self, index: usize) -> Option<&mut GradientBarConfig> {
        self.config_mut()
            .and_then(|config| config.bars.get_mut(index))
    }

    fn move_plane(&mut self, index: usize, offset: isize) {
        let Some(config) = self.config_mut() else {
            return;
        };
        let target = index.saturating_add_signed(offset);
        if target >= config.planes.len() {
            return;
        }
        config.planes.swap(index, target);
    }

    fn move_bar(&mut self, index: usize, offset: isize) {
        let Some(config) = self.config_mut() else {
            return;
        };
        let target = index.saturating_add_signed(offset);
        if target >= config.bars.len() {
            return;
        }
        config.bars.swap(index, target);
    }

    fn move_config(&mut self, offset: isize) {
        let Some(index) = self.selected_config else {
            return;
        };
        let target = index.saturating_add_signed(offset);
        if target >= self.configs.len() || target == index {
            return;
        }
        self.configs.swap(index, target);
        self.selected_config = Some(target);
    }

    pub fn update(&mut self, message: ColorSelectorConfigMessage) {
        match message {
            ColorSelectorConfigMessage::ConfigSelected(index) => {
                if index < self.configs.len() {
                    self.selected_config = Some(index);
                }
            }
            ColorSelectorConfigMessage::ConfigNameChanged(name) => {
                if let Some(config) = self.config_mut() {
                    config.name = name;
                }
            }
            ColorSelectorConfigMessage::MaxPlaneSizeChanged(value) => {
                if let Some(config) = self.config_mut() {
                    config.max_plane_size = value;
                }
            }
            ColorSelectorConfigMessage::MaxPlanesPerRowChanged(value) => {
                if let Some(config) = self.config_mut() {
                    config.max_planes_per_row = value;
                }
            }
            ColorSelectorConfigMessage::AddConfig => {
                self.configs.push(ColorSelectorConfig {
                    name: format!("Config {}", self.configs.len() + 1),
                    max_plane_size: 512,
                    max_planes_per_row: 2,
                    planes: Vec::new(),
                    bars: Vec::new(),
                    out_of_gamut_color: Rgb::new(0.5, 0.5, 0.5),
                    use_out_of_gamut_color: true,
                    clip_to_gamut: false,
                });
                self.selected_config = Some(self.configs.len() - 1);
            }
            ColorSelectorConfigMessage::RemoveConfig => {
                let Some(index) = self.selected_config else {
                    return;
                };
                self.configs.remove(index);
                self.selected_config = if self.configs.is_empty() {
                    None
                } else {
                    Some(index.min(self.configs.len() - 1))
                };
            }
            ColorSelectorConfigMessage::MoveConfigUp => self.move_config(-1),
            ColorSelectorConfigMessage::MoveConfigDown => self.move_config(1),
            ColorSelectorConfigMessage::OutOfGamutColorToggled(checked) => {
                if let Some(config) = self.config_mut() {
                    config.use_out_of_gamut_color = checked;
                }
            }
            ColorSelectorConfigMessage::OutOfGamutPickerToggled => {
                self.out_of_gamut_picker_open = !self.out_of_gamut_picker_open;
            }
            ColorSelectorConfigMessage::OutOfGamutPickerCancelled => {
                self.out_of_gamut_picker_open = false;
            }
            ColorSelectorConfigMessage::OutOfGamutColorSubmitted(color) => {
                self.out_of_gamut_picker_open = false;
                if let Some(config) = self.config_mut() {
                    config.out_of_gamut_color = Rgb::new(color.r, color.g, color.b);
                }
            }
            ColorSelectorConfigMessage::ClipToGamutToggled(checked) => {
                if let Some(config) = self.config_mut() {
                    config.clip_to_gamut = checked;
                }
            }
            ColorSelectorConfigMessage::AddPlane => {
                if let Some(config) = self.config_mut() {
                    config.planes.push(GradientPlaneConfig {
                        model: ColorModel::Hsv,
                        shape: GradientPlaneShape::Square,
                        variable_channels: 0b110,
                        flip_axis: GradientPlaneFlipAxis::empty(),
                        rotation: 0.0,
                        show_primary_channel_ring: false,
                        primary_channel_ring_width: 20.0,
                        ring_bar_saturated_hue_channel: false,
                        ring_rotation: 0.0,
                        reversed_ring: false,
                    });
                }
            }
            ColorSelectorConfigMessage::RemovePlane(index) => {
                if let Some(config) = self.config_mut() {
                    config.planes.remove(index);
                }
            }
            ColorSelectorConfigMessage::MovePlaneUp(index) => self.move_plane(index, -1),
            ColorSelectorConfigMessage::MovePlaneDown(index) => self.move_plane(index, 1),
            ColorSelectorConfigMessage::PlaneModelChanged(index, model) => {
                if let Some(plane) = self.plane_mut(index) {
                    plane.model = model;
                }
            }
            ColorSelectorConfigMessage::PlaneShapeChanged(index, shape) => {
                if let Some(plane) = self.plane_mut(index) {
                    plane.shape = shape;
                }
            }
            ColorSelectorConfigMessage::PlanePrimaryChannelChanged(index, channel) => {
                if let Some(plane) = self.plane_mut(index) {
                    plane.variable_channels = 0b111u8 & !(1u8 << channel);
                }
            }
            ColorSelectorConfigMessage::PlaneFlipXChanged(index, checked) => {
                if let Some(plane) = self.plane_mut(index) {
                    plane.flip_axis.set(GradientPlaneFlipAxis::X, checked);
                }
            }
            ColorSelectorConfigMessage::PlaneFlipYChanged(index, checked) => {
                if let Some(plane) = self.plane_mut(index) {
                    plane.flip_axis.set(GradientPlaneFlipAxis::Y, checked);
                }
            }
            ColorSelectorConfigMessage::PlaneRotationChanged(index, rotation) => {
                if let Some(plane) = self.plane_mut(index) {
                    plane.rotation = rotation;
                }
            }
            ColorSelectorConfigMessage::PlaneShowRingChanged(index, checked) => {
                if let Some(plane) = self.plane_mut(index) {
                    plane.show_primary_channel_ring = checked;
                }
            }
            ColorSelectorConfigMessage::PlaneSaturatedPrimaryChannelChanged(index, checked) => {
                if let Some(plane) = self.plane_mut(index) {
                    plane.ring_bar_saturated_hue_channel = checked;
                }
            }
            ColorSelectorConfigMessage::PlaneReversedRingChanged(index, checked) => {
                if let Some(plane) = self.plane_mut(index) {
                    plane.reversed_ring = checked;
                }
            }
            ColorSelectorConfigMessage::PlaneRingWidthChanged(index, width) => {
                if let Some(plane) = self.plane_mut(index) {
                    plane.primary_channel_ring_width = width;
                }
            }
            ColorSelectorConfigMessage::PlaneRingRotationChanged(index, rotation) => {
                if let Some(plane) = self.plane_mut(index) {
                    plane.ring_rotation = rotation;
                }
            }
            ColorSelectorConfigMessage::AddBar => {
                if let Some(config) = self.config_mut() {
                    config.bars.push(GradientBarConfig {
                        model: ColorModel::Rgb,
                        channel: 0,
                        bar_height: 20.0,
                        show_channel_label: true,
                        show_precise_spin_box: true,
                        show_primary_channel_lock: false,
                    });
                }
            }
            ColorSelectorConfigMessage::RemoveBar(index) => {
                if let Some(config) = self.config_mut() {
                    config.bars.remove(index);
                }
            }
            ColorSelectorConfigMessage::MoveBarUp(index) => self.move_bar(index, -1),
            ColorSelectorConfigMessage::MoveBarDown(index) => self.move_bar(index, 1),
            ColorSelectorConfigMessage::BarModelChanged(index, model) => {
                if let Some(bar) = self.bar_mut(index) {
                    bar.model = model;
                    bar.channel = bar
                        .channel
                        .min(model.channel_labels().len().saturating_sub(1) as u8);
                }
            }
            ColorSelectorConfigMessage::BarHeightChanged(index, height) => {
                if let Some(bar) = self.bar_mut(index) {
                    bar.bar_height = height;
                }
            }
            ColorSelectorConfigMessage::BarChannelChanged(index, channel) => {
                if let Some(bar) = self.bar_mut(index) {
                    bar.channel = channel as u8;
                }
            }
            ColorSelectorConfigMessage::BarShowChannelLabelChanged(index, checked) => {
                if let Some(bar) = self.bar_mut(index) {
                    bar.show_channel_label = checked;
                }
            }
            ColorSelectorConfigMessage::BarShowPreciseSpinBoxChanged(index, checked) => {
                if let Some(bar) = self.bar_mut(index) {
                    bar.show_precise_spin_box = checked;
                }
            }
            ColorSelectorConfigMessage::BarShowPrimaryChannelLockChanged(index, checked) => {
                if let Some(bar) = self.bar_mut(index) {
                    bar.show_primary_channel_lock = checked;
                }
            }
            ColorSelectorConfigMessage::Confirmed | ColorSelectorConfigMessage::Cancelled => {}
        }
    }
}

type Element<'a> = iced_core::Element<'a, ColorSelectorConfigMessage, Theme, Renderer>;

impl ColorSelectorConfigEditorState {
    pub fn view(&self) -> Element<'_> {
        let config_bar = self.config_bar();
        let active = self.active_content();

        let content = scrollable(column![config_bar, active].gap(16.0).padding(16))
            .width(Length::Fill)
            .height(Length::Fill);

        let footer = row![
            space().width(Length::Fill),
            button(label(t!("cancel"))).on_press(ColorSelectorConfigMessage::Cancelled),
            button(label(t!("confirm")))
                .primary()
                .on_press(ColorSelectorConfigMessage::Confirmed),
        ]
        .gap(10.0)
        .padding(16);

        column![content, footer]
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    fn config_bar(&self) -> Element<'_> {
        let items = self
            .configs
            .iter()
            .enumerate()
            .map(|(index, config)| ConfigItem {
                index,
                name: config.name.clone(),
            })
            .collect::<Vec<_>>();
        let selected = self.selected_config.map(|index| items[index].clone());
        let has_config = self.selected_config.is_some();
        let not_first = self.selected_config.is_some_and(|index| index > 0);
        let not_last = self
            .selected_config
            .is_some_and(|index| index + 1 < self.configs.len());

        row![
            self.column_label(t!("config")),
            combo_box(items, selected, |item| {
                ColorSelectorConfigMessage::ConfigSelected(item.index)
            })
            .placeholder(t!("no_configs"))
            .width(Length::Fill),
            button(label(t!("add_config"))).on_press(ColorSelectorConfigMessage::AddConfig),
            button(label(t!("up")))
                .on_press_maybe(not_first.then_some(ColorSelectorConfigMessage::MoveConfigUp)),
            button(label(t!("down")))
                .on_press_maybe(not_last.then_some(ColorSelectorConfigMessage::MoveConfigDown)),
            button(label(t!("remove")))
                .danger()
                .on_press_maybe(has_config.then_some(ColorSelectorConfigMessage::RemoveConfig)),
        ]
        .gap(10.0)
        .into()
    }

    fn column_label<'a>(&self, label: impl IntoFragment<'a>) -> Element<'a> {
        lapiz_widgets::label(label)
            .width(Length::Fixed(110.0))
            .into()
    }

    fn active_content(&self) -> Element<'_> {
        let Some(index) = self.selected_config else {
            return label(t!("no_configs_hint"))
                .muted()
                .width(Length::Fill)
                .into();
        };
        let config = &self.configs[index];

        let planes = config
            .planes
            .iter()
            .enumerate()
            .map(|(plane_index, _)| self.render_plane(config, plane_index))
            .collect::<Vec<_>>();
        let bars = config
            .bars
            .iter()
            .enumerate()
            .map(|(bar_index, _)| self.render_bar(config, bar_index))
            .collect::<Vec<_>>();

        // The color swatch remains hidden until the color picker is available again.

        let planes_section = column![
            row![
                label(t!("planes")),
                space().width(Length::Fill),
                button(label(t!("add_plane"))).on_press(ColorSelectorConfigMessage::AddPlane),
            ]
            .gap(8.0),
        ]
        .gap(10.0)
        .extend(planes);

        let bars_section = column![
            row![
                label(t!("bars")),
                space().width(Length::Fill),
                button(label(t!("add_bar"))).on_press(ColorSelectorConfigMessage::AddBar),
            ]
            .gap(8.0),
        ]
        .gap(10.0)
        .extend(bars);

        column![
            row![
                self.column_label(t!("name")),
                text_input(&t!("config_name"), &config.name)
                    .on_input(ColorSelectorConfigMessage::ConfigNameChanged)
                    .width(Length::Fill),
            ]
            .gap(10.0),
            spin_slider(128..=512, config.max_plane_size)
                .on_confirm(ColorSelectorConfigMessage::MaxPlaneSizeChanged)
                .prefix(t!("max_plane_size"))
                .suffix(" px"),
            spin_slider(1..=5, config.max_planes_per_row)
                .on_confirm(ColorSelectorConfigMessage::MaxPlanesPerRowChanged)
                .prefix(t!("max_planes_per_row")),
            row![
                checkbox(config.use_out_of_gamut_color)
                    .label(t!("out_of_gamut_color"))
                    .on_toggle(ColorSelectorConfigMessage::OutOfGamutColorToggled),
                // TODO Color picker
                checkbox(config.clip_to_gamut)
                    .label(t!("clip_to_gamut"))
                    .on_toggle(ColorSelectorConfigMessage::ClipToGamutToggled),
            ]
            .gap(16.0),
            planes_section,
            bars_section,
        ]
        .gap(16.0)
        .into()
    }

    fn render_plane(&self, config: &ColorSelectorConfig, index: usize) -> Element<'_> {
        let plane = &config.planes[index];
        let labels = plane.model.channel_labels();
        let primary_channel = (0..3)
            .find(|channel| plane.variable_channels & (1u8 << channel) == 0)
            .unwrap_or(0);
        let is_last = index + 1 == config.planes.len();

        self.panel(
            column![
                row![
                    label(t!("plane", index = index + 1)),
                    space().width(Length::Fill),
                    button(label(t!("up"))).on_press_maybe(
                        (index > 0).then_some(ColorSelectorConfigMessage::MovePlaneUp(index))
                    ),
                    button(label(t!("down"))).on_press_maybe(
                        (!is_last).then_some(ColorSelectorConfigMessage::MovePlaneDown(index))
                    ),
                    button(label(t!("remove")))
                        .danger()
                        .on_press(ColorSelectorConfigMessage::RemovePlane(index)),
                ]
                .gap(8.0),
                row![
                    self.column_label(t!("model")),
                    combo_box(ColorModel::PLANE_MODELS, Some(plane.model), move |model| {
                        ColorSelectorConfigMessage::PlaneModelChanged(index, model)
                    },)
                    .width(Length::Fill),
                ]
                .gap(10.0),
                row![
                    self.column_label(t!("shape")),
                    combo_box(
                        vec![
                            Translated(GradientPlaneShape::Square),
                            Translated(GradientPlaneShape::Triangle),
                        ],
                        Some(Translated(plane.shape)),
                        move |option| {
                            ColorSelectorConfigMessage::PlaneShapeChanged(index, option.0)
                        },
                    )
                    .width(Length::Fill),
                ]
                .gap(10.0),
                row![
                    self.column_label(t!("primary_channel")),
                    Flex::row(labels.iter().copied().enumerate().map(|(channel, label)| {
                        radio(
                            label,
                            channel,
                            (channel == primary_channel).then_some(channel),
                            move |channel| {
                                ColorSelectorConfigMessage::PlanePrimaryChannelChanged(
                                    index, channel,
                                )
                            },
                        )
                        .into()
                    }))
                    .gap(8.0),
                ]
                .gap(10.0),
                row![
                    checkbox(plane.flip_axis.contains(GradientPlaneFlipAxis::X))
                        .label(t!("flip_x"))
                        .on_toggle(move |checked| {
                            ColorSelectorConfigMessage::PlaneFlipXChanged(index, checked)
                        }),
                    checkbox(plane.flip_axis.contains(GradientPlaneFlipAxis::Y))
                        .label(t!("flip_y"))
                        .on_toggle(move |checked| {
                            ColorSelectorConfigMessage::PlaneFlipYChanged(index, checked)
                        }),
                ]
                .gap(16.0),
                spin_slider(0.0..=TAU, plane.rotation.rem_euclid(TAU))
                    .on_confirm(move |rotation| {
                        ColorSelectorConfigMessage::PlaneRotationChanged(index, rotation)
                    })
                    .precision(3)
                    .prefix(t!("plane_rotation"))
                    .suffix(" rad"),
                row![
                    checkbox(plane.show_primary_channel_ring)
                        .label(t!("primary_channel_ring"))
                        .on_toggle(move |checked| {
                            ColorSelectorConfigMessage::PlaneShowRingChanged(index, checked)
                        }),
                    checkbox(plane.ring_bar_saturated_hue_channel)
                        .label(t!("saturated_primary_channel"))
                        .on_toggle(move |checked| {
                            ColorSelectorConfigMessage::PlaneSaturatedPrimaryChannelChanged(
                                index, checked,
                            )
                        }),
                    checkbox(plane.reversed_ring)
                        .label(t!("reversed_ring"))
                        .on_toggle_maybe(plane.show_primary_channel_ring.then_some({
                            move |checked| {
                                ColorSelectorConfigMessage::PlaneReversedRingChanged(index, checked)
                            }
                        })),
                ]
                .gap(16.0),
                spin_slider(10.0..=40.0, plane.primary_channel_ring_width,)
                    .on_confirm(
                        move |width| ColorSelectorConfigMessage::PlaneRingWidthChanged(
                            index, width
                        )
                    )
                    .precision(1)
                    .disabled(!plane.show_primary_channel_ring)
                    .prefix(t!("ring_width"))
                    .suffix(" px"),
                spin_slider(0.0..=TAU, plane.ring_rotation.rem_euclid(TAU),)
                    .on_confirm(move |rotation| {
                        ColorSelectorConfigMessage::PlaneRingRotationChanged(index, rotation)
                    })
                    .precision(3)
                    .disabled(!plane.show_primary_channel_ring)
                    .prefix(t!("ring_rotation"))
                    .suffix(" rad"),
            ]
            .gap(12.0)
            .into(),
        )
    }

    fn render_bar(&self, config: &ColorSelectorConfig, index: usize) -> Element<'_> {
        let bar = &config.bars[index];
        let labels = bar.model.channel_labels();
        let is_last = index + 1 == config.bars.len();

        self.panel(
            column![
                row![
                    label(t!("bar", index = index + 1)),
                    space().width(Length::Fill),
                    button(label(t!("up"))).on_press_maybe(
                        (index > 0).then_some(ColorSelectorConfigMessage::MoveBarUp(index))
                    ),
                    button(label(t!("down"))).on_press_maybe(
                        (!is_last).then_some(ColorSelectorConfigMessage::MoveBarDown(index))
                    ),
                    button(label(t!("remove")))
                        .danger()
                        .on_press(ColorSelectorConfigMessage::RemoveBar(index)),
                ]
                .gap(8.0),
                row![
                    self.column_label(t!("model")),
                    combo_box(ColorModel::ALL, Some(bar.model), move |model| {
                        ColorSelectorConfigMessage::BarModelChanged(index, model)
                    },)
                    .width(Length::Fill),
                ]
                .gap(10.0),
                spin_slider(10.0..=40.0, bar.bar_height)
                    .on_confirm(move |height| ColorSelectorConfigMessage::BarHeightChanged(
                        index, height
                    ))
                    .precision(1)
                    .prefix(t!("bar_height"))
                    .suffix(" px"),
                row![
                    self.column_label(t!("channel")),
                    Flex::row(labels.iter().copied().enumerate().map(|(channel, label)| {
                        radio(
                            label,
                            channel,
                            (channel as u8 == bar.channel).then_some(channel),
                            move |channel| {
                                ColorSelectorConfigMessage::BarChannelChanged(index, channel)
                            },
                        )
                        .into()
                    }))
                    .gap(8.0),
                ]
                .gap(10.0),
                row![
                    checkbox(bar.show_channel_label)
                        .label(t!("channel_label"))
                        .on_toggle(move |checked| {
                            ColorSelectorConfigMessage::BarShowChannelLabelChanged(index, checked)
                        }),
                    checkbox(bar.show_precise_spin_box)
                        .label(t!("precise_spin_box"))
                        .on_toggle(move |checked| {
                            ColorSelectorConfigMessage::BarShowPreciseSpinBoxChanged(index, checked)
                        }),
                    checkbox(bar.show_primary_channel_lock)
                        .label(t!("primary_channel_lock"))
                        .on_toggle(move |checked| {
                            ColorSelectorConfigMessage::BarShowPrimaryChannelLockChanged(
                                index, checked,
                            )
                        }),
                ]
                .gap(16.0),
            ]
            .gap(12.0)
            .into(),
        )
    }

    fn panel<'a>(&self, content: Element<'a>) -> Element<'a> {
        panel(content).width(Length::Fill).padding(12).into()
    }
}
