use std::{
    fmt::Display,
    ops::{RangeBounds, RangeInclusive},
    str::FromStr,
};

use iced_core::{Element, text};
use iced_widget::svg;
use lapiz_math::curve::CubicCurve;
use lapiz_runtime::{Renderer, Theme};
use num_traits::AsPrimitive;

use crate::{
    button::Button,
    checkbox::Checkbox,
    collapsible::Collapsible,
    combo_box::ComboBox,
    curve_edit::CurveEdit,
    drag_drop_column::DragDropColumn,
    flex::Flex,
    form::Form,
    icon::Icon,
    kbd::Kbd,
    label::Label,
    labeled_frame::LabeledFrame,
    menu::{ContextMenu, Menu, MenuBar},
    panel::Panel,
    pick_list::PickList,
    popover::Popover,
    progress::ProgressBar,
    radio::Radio,
    scrollable::Scrollable,
    segmented_control::SegmentedControl,
    slider::Slider,
    spin_box::SpinBox,
    spin_slider::SpinSlider,
    status_bar::StatusBar,
    switch::Switch,
    tabs::{TabBar, TabbedView},
    tag::Tag,
    text_input::TextInput,
    title_bar::TitleBar,
    tooltip::{Position, Tooltip},
};

pub fn status_bar<'a, Message>(
    children: impl IntoIterator<Item = Element<'a, Message, Theme, Renderer>>,
) -> StatusBar<'a, Message> {
    StatusBar::new(children)
}

pub fn button<'a, Message>(
    content: impl Into<Element<'a, Message, Theme, Renderer>>,
) -> Button<'a, Message> {
    Button::new(content)
}

pub fn icon_button<'a, Message>(
    content: impl Into<Element<'a, Message, Theme, Renderer>>,
) -> Button<'a, Message> {
    Button::icon(content)
}

pub fn checkbox<'a, Message>(checked: bool) -> Checkbox<'a, Message> {
    Checkbox::new(checked)
}

pub fn collapsible<'a, Message>(
    header: impl Into<Element<'a, Message, Theme, Renderer>>,
    content: impl Into<Element<'a, Message, Theme, Renderer>>,
    open: bool,
) -> Collapsible<'a, Message> {
    Collapsible::new(header, content, open)
}

pub fn combo_box<'a, T, Message: 'a>(
    options: impl Into<Vec<T>>,
    selected: Option<T>,
    on_selected: impl Fn(T) -> Message + 'a,
) -> ComboBox<'a, T, Message>
where
    T: Display + Clone,
{
    ComboBox::new(options, selected, on_selected)
}

pub fn curve_edit<'a, Message>(curve: CubicCurve) -> CurveEdit<'a, Message> {
    CurveEdit::new(curve)
}

pub fn drag_drop_column<'a, Message>(
    children: impl Into<Vec<Element<'a, Message, Theme, Renderer>>>,
) -> DragDropColumn<'a, Message> {
    DragDropColumn::new(children)
}

pub fn form<'a, Message>() -> Form<'a, Message> {
    Form::new()
}

pub fn icon<'a>(handle: impl Into<svg::Handle>) -> Icon<'a> {
    Icon::new(handle)
}

pub fn kbd<'a>(content: impl text::IntoFragment<'a>) -> Kbd<'a> {
    Kbd::new(content)
}

pub fn label<'a>(content: impl text::IntoFragment<'a>) -> Label<'a> {
    Label::new(content)
}

pub fn labeled_frame<'a, Message>(
    title: impl Into<Element<'a, Message, Theme, Renderer>>,
    content: impl Into<Element<'a, Message, Theme, Renderer>>,
) -> LabeledFrame<'a, Message> {
    LabeledFrame::new(title, content)
}

pub fn menu<'a, Message>() -> Menu<'a, Message> {
    Menu::new()
}

pub fn menu_bar<'a, Message>() -> MenuBar<'a, Message> {
    MenuBar::new()
}

pub fn context_menu<'a, Message>(
    underlay: impl Into<Element<'a, Message, Theme, Renderer>>,
    menu: Menu<'a, Message>,
) -> ContextMenu<'a, Message> {
    ContextMenu::new(underlay, menu)
}

pub fn panel<'a, Message>(
    content: impl Into<Element<'a, Message, Theme, Renderer>>,
) -> Panel<'a, Message> {
    Panel::new(content)
}

pub fn pick_list<'a, Message>() -> PickList<'a, Message> {
    PickList::new()
}

pub fn popover<'a, Message>(
    trigger: impl Into<Element<'a, Message, Theme, Renderer>>,
) -> Popover<'a, Message> {
    Popover::new(trigger)
}

pub fn progress_bar<'a>(range: RangeInclusive<f32>, value: f32) -> ProgressBar<'a> {
    ProgressBar::new(range, value)
}

pub fn radio<'a, Message, V: Eq + Copy>(
    label: impl Into<String>,
    value: V,
    selected: Option<V>,
    on_click: impl FnOnce(V) -> Message,
) -> Radio<'a, Message> {
    Radio::new(label, value, selected, on_click)
}

pub fn scrollable<'a, Message>(
    content: impl Into<Element<'a, Message, Theme, Renderer>>,
) -> Scrollable<'a, Message> {
    Scrollable::new(content)
}

pub fn segmented_control<'a, Message: 'a>() -> SegmentedControl<'a, Message> {
    SegmentedControl::new()
}

pub fn slider<'a, T, Message>(
    range: RangeInclusive<T>,
    value: T,
    on_change: impl Fn(T) -> Message + 'a,
) -> Slider<'a, T, Message>
where
    T: Copy + From<u8> + PartialOrd,
    Message: Clone,
{
    Slider::new(range, value, on_change)
}

pub fn spin_box<'a, T, Message>(
    value: &T,
    bounds: impl RangeBounds<T>,
    on_change: impl Fn(T) -> Message + 'a,
) -> SpinBox<'a, T, Message>
where
    T: Copy + num_traits::Num + PartialOrd + Display + FromStr,
{
    SpinBox::new(value, bounds, on_change)
}

pub fn spin_slider<'a, T, Message>(
    range: RangeInclusive<T>,
    value: T,
) -> SpinSlider<'a, T, Message, Theme>
where
    T: Copy + PartialOrd + Display + FromStr + AsPrimitive<f64>,
    f64: AsPrimitive<T>,
{
    SpinSlider::new(range, value)
}

pub fn switch<'a, Message>(
    checked: bool,
    on_toggle: impl Fn(bool) -> Message + 'a,
) -> Switch<'a, Message> {
    Switch::new(checked, on_toggle)
}

pub fn tab_bar<'a, Message: 'a>(
    tabs: impl IntoIterator<Item = String>,
    selected: usize,
) -> TabBar<'a, Message> {
    TabBar::tabs(tabs, selected)
}

pub fn tabbed_view<'a, Message: 'a>(
    tabs: impl IntoIterator<Item = (String, Element<'a, Message, Theme, Renderer>)>,
    selected: usize,
) -> TabbedView<'a, Message> {
    TabbedView::tabs(tabs, selected)
}

pub fn tag<'a, Message>(content: impl text::IntoFragment<'a>) -> Tag<'a, Message> {
    Tag::new(content)
}

pub fn text_input<'a, Message: Clone>(placeholder: &str, value: &str) -> TextInput<'a, Message> {
    TextInput::new(placeholder, value)
}

pub fn title_bar<'a, Message>(
    content: impl Into<Element<'a, Message, Theme, Renderer>>,
) -> TitleBar<'a, Message> {
    TitleBar::new(content)
}

pub fn tooltip<'a, Message>(
    content: impl Into<Element<'a, Message, Theme, Renderer>>,
    tooltip: impl Into<Element<'a, Message, Theme, Renderer>>,
    position: Position,
) -> Tooltip<'a, Message> {
    Tooltip::new(content, tooltip, position)
}

#[macro_export]
macro_rules! column {
    () => (
        $crate::flex::Flex::column([])
    );
    ($($x:expr),+ $(,)?) => (
        $crate::flex::Flex::column([$($x.into()),+])
    );
}

#[macro_export]
macro_rules! row {
    () => (
        $crate::flex::Flex::row([])
    );
    ($($x:expr),+ $(,)?) => (
        $crate::flex::Flex::row([$($x.into()),+])
    );
}

pub fn column<'a, Message>(
    children: impl IntoIterator<Item = Element<'a, Message, Theme, Renderer>>,
) -> Flex<'a, Message> {
    Flex::column(children)
}

pub fn row<'a, Message>(
    children: impl IntoIterator<Item = Element<'a, Message, Theme, Renderer>>,
) -> Flex<'a, Message> {
    Flex::row(children)
}
