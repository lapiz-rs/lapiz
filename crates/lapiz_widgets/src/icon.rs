use iced_core::{Element, Length, Theme};
use iced_widget::{Svg, svg};
use lapiz_runtime::Renderer;

pub use iced_widget::svg::{Catalog, Status, Style, StyleFn};

pub struct Icon<'a> {
    inner: Svg<'a, Theme>,
}

impl<'a> Icon<'a> {
    pub fn new(handle: impl Into<svg::Handle>) -> Self {
        Self {
            inner: Svg::new(handle).width(16).height(16).style(default),
        }
    }

    pub fn size(mut self, size: impl Into<Length> + Copy) -> Self {
        self.inner = self.inner.width(size).height(size);
        self
    }

    pub fn width(mut self, width: impl Into<Length>) -> Self {
        self.inner = self.inner.width(width);
        self
    }

    pub fn height(mut self, height: impl Into<Length>) -> Self {
        self.inner = self.inner.height(height);
        self
    }

    pub fn style(mut self, style: impl Fn(&Theme, Status) -> Style + 'a) -> Self {
        self.inner = self.inner.style(style);
        self
    }

    pub fn class(mut self, class: impl Into<<Theme as Catalog>::Class<'a>>) -> Self {
        self.inner = self.inner.class(class);
        self
    }

    pub fn muted(self) -> Self {
        self.style(muted)
    }

    pub fn accent(self) -> Self {
        self.style(accent)
    }

    pub fn danger(self) -> Self {
        self.style(danger)
    }
}

impl<'a, Message: 'a> From<Icon<'a>> for Element<'a, Message, Theme, Renderer> {
    fn from(value: Icon<'a>) -> Self {
        value.inner.into()
    }
}

pub fn default(theme: &Theme, _status: Status) -> Style {
    Style {
        color: Some(theme.palette().background.base.text),
    }
}

pub fn muted(theme: &Theme, _status: Status) -> Style {
    Style {
        color: Some(theme.palette().background.weak.text),
    }
}

pub fn accent(theme: &Theme, _status: Status) -> Style {
    Style {
        color: Some(theme.palette().primary.strong.color),
    }
}

pub fn danger(theme: &Theme, _status: Status) -> Style {
    Style {
        color: Some(theme.palette().danger.base.color),
    }
}

macro_rules! icons {
    ($($name:ident => $path:literal),* $(,)?) => {
        $(
            pub fn $name<'a>() -> Icon<'a> {
                Icon::new(svg::Handle::from_memory(include_bytes!($path).as_slice()))
            }
        )*

        pub const ALL: &[(&str, &[u8])] = &[
            $((stringify!($name), include_bytes!($path).as_slice()),)*
        ];
    };
}

icons! {
    airbrush => "../assets/icons/airbrush.svg",
    alpha_lock => "../assets/icons/alpha_lock.svg",
    arrow_right => "../assets/icons/arrow_right.svg",
    blend => "../assets/icons/blend.svg",
    blender => "../assets/icons/blender.svg",
    blur => "../assets/icons/blur.svg",
    brush => "../assets/icons/brush.svg",
    burn => "../assets/icons/burn.svg",
    canvas_size => "../assets/icons/canvas_size.svg",
    caret_down => "../assets/icons/caret_down.svg",
    caret_right => "../assets/icons/caret_right.svg",
    check => "../assets/icons/check.svg",
    chevron_down => "../assets/icons/chevron_down.svg",
    chevron_left => "../assets/icons/chevron_left.svg",
    chevron_right => "../assets/icons/chevron_right.svg",
    chevron_up => "../assets/icons/chevron_up.svg",
    clock => "../assets/icons/clock.svg",
    close => "../assets/icons/close.svg",
    cloud => "../assets/icons/cloud.svg",
    copy => "../assets/icons/copy.svg",
    cpu => "../assets/icons/cpu.svg",
    crop => "../assets/icons/crop.svg",
    curve => "../assets/icons/curve.svg",
    dock_bottom => "../assets/icons/dock_bottom.svg",
    dock_left => "../assets/icons/dock_left.svg",
    dock_right => "../assets/icons/dock_right.svg",
    dodge => "../assets/icons/dodge.svg",
    ellipse => "../assets/icons/ellipse.svg",
    ellipse_select => "../assets/icons/ellipse_select.svg",
    eraser => "../assets/icons/eraser.svg",
    export => "../assets/icons/export.svg",
    eye => "../assets/icons/eye.svg",
    eye_off => "../assets/icons/eye_off.svg",
    eyedropper => "../assets/icons/eyedropper.svg",
    file => "../assets/icons/file.svg",
    file_new => "../assets/icons/file_new.svg",
    fill => "../assets/icons/fill.svg",
    filter => "../assets/icons/filter.svg",
    folder => "../assets/icons/folder.svg",
    folder_open => "../assets/icons/folder_open.svg",
    gradient => "../assets/icons/gradient.svg",
    grid => "../assets/icons/grid.svg",
    grip => "../assets/icons/grip.svg",
    group => "../assets/icons/group.svg",
    hand => "../assets/icons/hand.svg",
    history => "../assets/icons/history.svg",
    import => "../assets/icons/import.svg",
    info => "../assets/icons/info.svg",
    keyboard => "../assets/icons/keyboard.svg",
    lasso => "../assets/icons/lasso.svg",
    layers => "../assets/icons/layers.svg",
    line => "../assets/icons/line.svg",
    link => "../assets/icons/link.svg",
    lock => "../assets/icons/lock.svg",
    magic_wand => "../assets/icons/magic_wand.svg",
    magnet => "../assets/icons/magnet.svg",
    mask => "../assets/icons/mask.svg",
    merge => "../assets/icons/merge.svg",
    minus => "../assets/icons/minus.svg",
    monitor => "../assets/icons/monitor.svg",
    moon => "../assets/icons/moon.svg",
    more => "../assets/icons/more.svg",
    move_tool => "../assets/icons/move.svg",
    nodes => "../assets/icons/nodes.svg",
    inherit_alpha => "../assets/icons/inherit_alpha.svg",
    palette => "../assets/icons/palette.svg",
    pencil => "../assets/icons/pencil.svg",
    perspective => "../assets/icons/perspective.svg",
    pin => "../assets/icons/pin.svg",
    play => "../assets/icons/play.svg",
    plugin => "../assets/icons/plugin.svg",
    plus => "../assets/icons/plus.svg",
    poly_lasso => "../assets/icons/poly_lasso.svg",
    polygon => "../assets/icons/polygon.svg",
    rect => "../assets/icons/rect.svg",
    rect_select => "../assets/icons/rect_select.svg",
    redo => "../assets/icons/redo.svg",
    reference => "../assets/icons/reference.svg",
    refresh => "../assets/icons/refresh.svg",
    ruler => "../assets/icons/ruler.svg",
    save => "../assets/icons/save.svg",
    search => "../assets/icons/search.svg",
    settings => "../assets/icons/settings.svg",
    sharpen => "../assets/icons/sharpen.svg",
    sliders => "../assets/icons/sliders.svg",
    smudge => "../assets/icons/smudge.svg",
    stamp => "../assets/icons/stamp.svg",
    star => "../assets/icons/star.svg",
    sun => "../assets/icons/sun.svg",
    swatches => "../assets/icons/swatches.svg",
    symmetry => "../assets/icons/symmetry.svg",
    target => "../assets/icons/target.svg",
    text => "../assets/icons/text.svg",
    transform => "../assets/icons/transform.svg",
    trash => "../assets/icons/trash.svg",
    undo => "../assets/icons/undo.svg",
    unlock => "../assets/icons/unlock.svg",
    user => "../assets/icons/user.svg",
    warning => "../assets/icons/warning.svg",
    win_close => "../assets/icons/win_close.svg",
    win_maximize => "../assets/icons/win_maximize.svg",
    win_minimize => "../assets/icons/win_minimize.svg",
    win_restore => "../assets/icons/win_restore.svg",
    zoom => "../assets/icons/zoom.svg",
}
