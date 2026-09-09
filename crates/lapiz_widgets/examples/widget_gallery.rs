use glam::Vec2;
use iced::{Element, Length, Padding, Theme};
use lapiz_math::curve::CubicCurve;
use lapiz_widgets::{
    bar::StatusBar,
    button::{Button, icon_button, toggle_button},
    checkbox::Checkbox,
    collapsible::Collapsible,
    combo_box::{self, ComboBox},
    curve_edit::CurveEdit,
    divider::Divider,
    flex::Flex,
    icon::{self, Icon},
    kbd::Kbd,
    label::Label,
    labeled_frame::LabeledFrame,
    menu::{Menu, MenuBar},
    panel::Panel,
    pick_list::PickList,
    progress::ProgressBar,
    radio::Radio,
    scrollable::Scrollable,
    segmented_control::SegmentedControl,
    slider::Slider,
    spin_box::SpinBox,
    spin_slider::SpinSlider,
    splitter::Splitter,
    switch::Switch,
    tabs::TabBar,
    tag::Tag,
    tag::Tone,
    text_input::TextInput,
    tooltip::{Position, Tooltip},
};

#[derive(Debug, Clone)]
enum Message {
    Toggle(bool),
    Radio(u8),
    Text(String),
    Number(i32),
    Value(f32),
    Resize(f32),
    Curve(CubicCurve),
    Combo(String),
    Tab(usize),
    PickAvailable(usize),
    PickSelected(usize),
    MoveRight,
    MoveLeft,
    Collapse,
    ThemeSelected(Theme),
    Noop,
}

struct Gallery {
    checked: bool,
    radio: u8,
    text: String,
    number: i32,
    value: f32,
    split: f32,
    curve: CubicCurve,
    combo: combo_box::State<String>,
    selected_combo: Option<String>,
    tab: usize,
    available: usize,
    selected: usize,
    collapsed: bool,
    theme: Theme,
}

impl Gallery {
    fn new() -> Self {
        Self {
            checked: true,
            radio: 0,
            text: String::from("Wet bristle"),
            number: 24,
            value: 62.0,
            split: 0.45,
            curve: CubicCurve::new(vec![
                Vec2::new(0.0, 0.0),
                Vec2::new(0.3, 0.18),
                Vec2::new(0.72, 0.86),
                Vec2::new(1.0, 1.0),
            ]),
            combo: combo_box::State::new(vec![
                String::from("Normal"),
                String::from("Multiply"),
                String::from("Screen"),
                String::from("Overlay"),
            ]),
            selected_combo: Some(String::from("Normal")),
            tab: 0,
            available: 0,
            selected: 0,
            collapsed: false,
            theme: Theme::Dark,
        }
    }

    fn update(&mut self, message: Message) {
        match message {
            Message::Toggle(value) => self.checked = value,
            Message::Radio(value) => self.radio = value,
            Message::Text(value) => self.text = value,
            Message::Number(value) => self.number = value,
            Message::Value(value) => self.value = value,
            Message::Resize(value) => self.split = value,
            Message::Curve(value) => self.curve = value,
            Message::Combo(value) => self.selected_combo = Some(value),
            Message::Tab(value) => self.tab = value,
            Message::PickAvailable(value) => self.available = value,
            Message::PickSelected(value) => self.selected = value,
            Message::Collapse => self.collapsed = !self.collapsed,
            Message::ThemeSelected(theme) => self.theme = theme,
            Message::MoveRight | Message::MoveLeft | Message::Noop => {}
        }
    }

    fn view(&self) -> Element<'_, Message> {
        let controls = LabeledFrame::new(
            Label::new("INPUTS").muted(),
            Flex::column([
                TextInput::new("Brush name", &self.text)
                    .on_input(Message::Text)
                    .width(220)
                    .into(),
                Checkbox::new(self.checked)
                    .label("Pressure controls opacity")
                    .on_toggle(Message::Toggle)
                    .into(),
                Switch::new(self.checked, Message::Toggle)
                    .label("Live preview")
                    .into(),
                Flex::row([
                    Radio::new("Pixels", 0, Some(self.radio), Message::Radio).into(),
                    Radio::new("Percent", 1, Some(self.radio), Message::Radio).into(),
                ])
                .gap(12)
                .into(),
                Slider::new(0.0..=100.0, self.value, Message::Value)
                    .width(220)
                    .into(),
                SpinSlider::new_percent(self.value)
                    .on_change(Message::Value)
                    .suffix(" %")
                    .width(220)
                    .into(),
                SpinBox::new(&self.number, 1..=512, Message::Number)
                    .width(110)
                    .into(),
                ComboBox::searchable(
                    &self.combo,
                    "Searchable blend mode",
                    self.selected_combo.clone(),
                    Message::Combo,
                )
                .width(220)
                .into(),
                ComboBox::new(
                    self.combo.options().to_vec(),
                    self.selected_combo.clone(),
                    Message::Combo,
                )
                .placeholder("Selection-only blend mode")
                .width(220)
                .into(),
            ])
            .gap(9)
            .padding(8),
        )
        .width(280);

        let buttons = LabeledFrame::new(
            Label::new("BUTTONS & STATUS").muted(),
            Flex::column([
                Flex::row([
                    Button::new(Label::new("Default"))
                        .on_press(Message::Noop)
                        .into(),
                    Button::new(Label::new("Primary"))
                        .primary()
                        .on_press(Message::Noop)
                        .into(),
                    Button::new(Label::new("Danger"))
                        .danger()
                        .on_press(Message::Noop)
                        .into(),
                    Button::new(Label::new("Disabled")).into(),
                ])
                .gap(6)
                .into(),
                Flex::row([
                    icon_button(icon::brush()).on_press(Message::Noop).into(),
                    icon_button(icon::eraser()).on_press(Message::Noop).into(),
                    toggle_button(icon::symmetry(), true)
                        .on_press(Message::Noop)
                        .into(),
                    Tooltip::new(
                        icon_button(icon::info()).on_press(Message::Noop),
                        Label::new("Tooltips use square industrial surfaces"),
                        Position::Bottom,
                    )
                    .into(),
                    Kbd::new("Ctrl").into(),
                    Kbd::new("Z").into(),
                ])
                .gap(4)
                .into(),
                Flex::row([
                    Tag::new("PRIMARY").tone(Tone::Primary).into(),
                    Tag::new("READY").tone(Tone::Success).into(),
                    Tag::new("CAUTION").tone(Tone::Warning).into(),
                    Tag::new("ERROR").tone(Tone::Danger).into(),
                ])
                .gap(5)
                .into(),
                ProgressBar::new(0.0..=100.0, self.value)
                    .length(220)
                    .girth(8)
                    .into(),
                ProgressBar::new(0.0..=100.0, 78.0)
                    .success()
                    .length(220)
                    .girth(8)
                    .into(),
            ])
            .gap(9)
            .padding(8),
        )
        .width(430);

        let navigation = LabeledFrame::new(
            Label::new("NAVIGATION & GROUPS").muted(),
            Flex::column([
                TabBar::new()
                    .push(Label::new("Brushes"), self.tab == 0, Message::Tab(0))
                    .push(Label::new("Layers"), self.tab == 1, Message::Tab(1))
                    .push(Label::new("History"), self.tab == 2, Message::Tab(2))
                    .width(Length::Fill)
                    .into(),
                SegmentedControl::new()
                    .push(icon::brush(), self.tab == 0, Message::Tab(0))
                    .push(icon::eraser(), self.tab == 1, Message::Tab(1))
                    .push(icon::smudge(), self.tab == 2, Message::Tab(2))
                    .into(),
                Collapsible::new(
                    Label::new("Advanced brush dynamics"),
                    Flex::column([
                        Label::new("Jitter and scatter controls").muted().into(),
                        Slider::new(0.0..=100.0, 35.0, |_| Message::Noop).into(),
                    ])
                    .gap(6)
                    .padding(8),
                    !self.collapsed,
                )
                .on_toggle(Message::Collapse)
                .into(),
            ])
            .gap(10)
            .padding(8),
        )
        .width(430);

        let curve = LabeledFrame::new(
            Label::new("CURVE EDITOR").muted(),
            CurveEdit::new(self.curve.clone())
                .on_change(Message::Curve)
                .on_release(Message::Curve)
                .height(165),
        )
        .width(280)
        .height(195);

        let pick_list = PickList::new()
            .labels("Available presets", "Active presets")
            .available_item(
                Label::new("Charcoal Grain"),
                self.available == 0,
                Message::PickAvailable(0),
            )
            .available_item(
                Label::new("Dry Bristle"),
                self.available == 1,
                Message::PickAvailable(1),
            )
            .available_item(
                Label::new("Ink Bleed"),
                self.available == 2,
                Message::PickAvailable(2),
            )
            .selected_item(
                Label::new("Soft Round"),
                self.selected == 0,
                Message::PickSelected(0),
            )
            .selected_item(
                Label::new("Wet Flat"),
                self.selected == 1,
                Message::PickSelected(1),
            )
            .on_move_to_selected(Message::MoveRight)
            .on_move_to_available(Message::MoveLeft)
            .height(155);

        let splitter = Splitter::horizontal(
            Panel::new(Label::new("CANVAS").muted()).inset().padding(10),
            Panel::new(Label::new("INSPECTOR").muted())
                .inset()
                .padding(10),
            self.split,
        )
        .height(90)
        .on_resize(Message::Resize);

        let icon_rows = icon::ALL.chunks(12).map(|icons| {
            let row = icons.iter().map(|(name, bytes)| {
                Tooltip::new(
                    Panel::new(Icon::new(iced::widget::svg::Handle::from_memory(*bytes)).size(17))
                        .inset()
                        .padding(7),
                    Label::new(*name),
                    Position::Bottom,
                )
                .into()
            });
            Flex::row(row).gap(4).into()
        });
        let glyphs = LabeledFrame::new(
            Label::new(format!("GLYPHS / {}", icon::ALL.len())).muted(),
            Flex::column(icon_rows).gap(4).padding(8),
        );

        let recent_menu = Menu::new()
            .item("concept-01.lapiz", Message::Noop)
            .item("character-study.lapiz", Message::Noop)
            .item("untitled-4.lapiz", Message::Noop)
            .min_width(220.0);
        let file_menu = Menu::new()
            .item_shortcut("New", "Ctrl+N", Message::Noop)
            .item_shortcut("Open", "Ctrl+O", Message::Noop)
            .submenu("Open Recent", recent_menu)
            .separator()
            .item_shortcut("Save", "Ctrl+S", Message::Noop)
            .item_shortcut("Export", "Ctrl+Shift+S", Message::Noop)
            .min_width(220.0);
        let edit_menu = Menu::new()
            .item_shortcut("Undo", "Ctrl+Z", Message::Noop)
            .item_shortcut("Redo", "Ctrl+Shift+Z", Message::Noop)
            .separator()
            .item_shortcut("Cut", "Ctrl+X", Message::Noop)
            .item_shortcut("Copy", "Ctrl+C", Message::Noop)
            .item_shortcut("Paste", "Ctrl+V", Message::Noop)
            .min_width(220.0);
        let view_menu = Menu::new()
            .item_shortcut("Zoom In", "Ctrl++", Message::Noop)
            .item_shortcut("Zoom Out", "Ctrl+-", Message::Noop)
            .item_shortcut("Fit Canvas", "1", Message::Noop)
            .separator()
            .item_shortcut("Mirror Canvas", "M", Message::Noop)
            .min_width(220.0);
        let layer_menu = Menu::new()
            .item_shortcut("New Layer", "Insert", Message::Noop)
            .item_shortcut("Duplicate Layer", "Ctrl+J", Message::Noop)
            .separator()
            .item_shortcut("Merge Down", "Ctrl+E", Message::Noop)
            .item_shortcut("Delete Layer", "Del", Message::Noop)
            .min_width(220.0);
        let select_menu = Menu::new()
            .item_shortcut("Select All", "Ctrl+A", Message::Noop)
            .item_shortcut("Deselect", "Ctrl+Shift+A", Message::Noop)
            .item_shortcut("Invert Selection", "Ctrl+I", Message::Noop)
            .min_width(220.0);
        let filter_menu = Menu::new()
            .item("Gaussian Blur", Message::Noop)
            .item("Sharpen", Message::Noop)
            .item_shortcut("Color Balance", "Ctrl+B", Message::Noop)
            .min_width(220.0);
        let theme_menu =
            Theme::ALL
                .iter()
                .cloned()
                .fold(Menu::new().min_width(220.0), |menu, theme| {
                    if theme == self.theme {
                        menu.selected_item(theme.to_string(), Message::ThemeSelected(theme))
                    } else {
                        menu.item(theme.to_string(), Message::ThemeSelected(theme))
                    }
                });
        let window_menu = Menu::new()
            .selected_item_shortcut("Canvas", "Tab", Message::Noop)
            .item_shortcut("Layers", "F7", Message::Noop)
            .item_shortcut("Brush Editor", "F5", Message::Noop)
            .separator()
            .submenu("Theme", theme_menu)
            .min_width(220.0);
        let help_menu = Menu::new()
            .item_shortcut("Keyboard Shortcuts", "F1", Message::Noop)
            .separator()
            .item("About Lapiz", Message::Noop)
            .min_width(220.0);
        let menu_bar = MenuBar::new()
            .menu("File", file_menu)
            .menu("Edit", edit_menu)
            .menu("View", view_menu)
            .menu("Layer", layer_menu)
            .menu("Select", select_menu)
            .menu("Filter", filter_menu)
            .menu("Window", window_menu)
            .menu("Help", help_menu);
        let toolbar = Flex::row([
            Label::new("LAPIZ WIDGET GALLERY").strong().into(),
            menu_bar.into(),
        ])
        .width(Length::Fill)
        .height(36)
        .padding([0, 8])
        .gap(6);

        let content = Flex::column([
            toolbar.into(),
            Flex::row([controls.into(), buttons.into()])
                .gap(12)
                .padding(Padding {
                    top: 12.0,
                    right: 12.0,
                    bottom: 0.0,
                    left: 12.0,
                })
                .into(),
            Flex::row([navigation.into(), curve.into(), pick_list.into()])
                .gap(12)
                .padding([0, 12])
                .into(),
            Flex::column([Label::new("SPLITTER").muted().into(), splitter.into()])
                .gap(5)
                .padding([0, 12])
                .into(),
            glyphs.into(),
            Divider::horizontal(1).into(),
            StatusBar::new([
                Label::new("READY").accent().into(),
                Label::new("105 glyphs").muted().into(),
                Label::new("iced + wgpu").muted().into(),
            ])
            .into(),
        ])
        .width(Length::Fill)
        .gap(12);

        Scrollable::new(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }
}

fn main() -> iced::Result {
    iced::application(Gallery::new, Gallery::update, Gallery::view)
        .title("Lapiz Widget Gallery")
        .theme(|gallery: &Gallery| gallery.theme.clone())
        .window_size((980.0, 900.0))
        .run()
}
