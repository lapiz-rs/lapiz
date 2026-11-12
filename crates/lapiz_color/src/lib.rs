use lapiz_runtime::{
    Runtime,
    event::Event,
    global::{Global, Globals},
    plugin::Plugin,
};
use lapiz_utils::wrapper;
use moxcms::Matrix3f;

use crate::model::{
    gray::Gray, hsl::Hsl, hsv::Hsv, lab::Lab, lch::Lch, okhsl::OkHsl, okhsv::OkHsv, oklab::OkLab,
    oklch::OkLch, rgb::Rgb, xyz::Xyz,
};

wesl::wesl_pkg!(pub color);

pub mod model;
pub mod platform;
pub mod shader;

/// A polymorphic color value represented in one of the supported color models.
///
/// `Color` acts as a unified interchange representation across perceptual, cylindrical,
/// linear-light RGB, and CIE tristimulus color spaces. It can be converted into linear
/// working-space [`Rgb`] via [`Color::into_rgb`] given the appropriate [`Xyz`]-to-RGB
/// transformation matrix.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Color {
    Gray(Gray),
    Hsl(Hsl),
    Hsv(Hsv),
    Lab(Lab),
    Lch(Lch),
    OkHsl(OkHsl),
    OkHsv(OkHsv),
    OkLab(OkLab),
    OkLch(OkLch),
    Rgb(Rgb),
    Xyz(Xyz),
}

impl Color {
    pub fn into_rgb(self, xyz_to_rgb: Matrix3f) -> Rgb {
        match self {
            Color::Gray(gray) => Rgb::from_xyz(gray.into_xyz(), xyz_to_rgb),
            Color::Hsl(hsl) => hsl.into_rgb(),
            Color::Hsv(hsv) => hsv.into_rgb(),
            Color::Lab(lab) => Rgb::from_xyz(lab.into_xyz(), xyz_to_rgb),
            Color::Lch(lch) => Rgb::from_xyz(lch.into_xyz(), xyz_to_rgb),
            Color::OkHsl(ok_hsl) => Rgb::from_xyz(ok_hsl.into_xyz(), xyz_to_rgb),
            Color::OkHsv(ok_hsv) => Rgb::from_xyz(ok_hsv.into_xyz(), xyz_to_rgb),
            Color::OkLab(ok_lab) => Rgb::from_xyz(ok_lab.into_xyz(), xyz_to_rgb),
            Color::OkLch(ok_lch) => Rgb::from_xyz(ok_lch.into_xyz(), xyz_to_rgb),
            Color::Rgb(rgb) => rgb,
            Color::Xyz(xyz) => Rgb::from_xyz(xyz, xyz_to_rgb),
        }
    }
}

macro_rules! impl_from {
    ($variant:ident, $ty:ty) => {
        impl From<$ty> for Color {
            fn from(value: $ty) -> Self {
                Color::$variant(value)
            }
        }
    };
}
impl_from!(Gray, Gray);
impl_from!(Hsl, Hsl);
impl_from!(Hsv, Hsv);
impl_from!(Lab, Lab);
impl_from!(Lch, Lch);
impl_from!(OkHsl, OkHsl);
impl_from!(OkHsv, OkHsv);
impl_from!(OkLab, OkLab);
impl_from!(OkLch, OkLch);
impl_from!(Rgb, Rgb);
impl_from!(Xyz, Xyz);

wrapper! {
    /// Global state storing the active primary (foreground) [`Color`] used by tools and brushes.
    #[derive(Debug, Clone, Copy)]
    pub mut ForegroundColor : Color
}

impl Global for ForegroundColor {}

impl ForegroundColor {
    pub fn get(&self) -> Color {
        self.0
    }

    pub fn set(&mut self, color: Color) {
        self.0 = color;
    }
}

wrapper! {
    /// Global state storing the active secondary (background) [`Color`] used by tools and canvas operations.
    #[derive(Debug, Clone, Copy)]
    pub mut BackgroundColor : Color
}

impl BackgroundColor {
    pub fn get(&self) -> Color {
        self.0
    }

    pub fn set(&mut self, color: Color) {
        self.0 = color;
    }
}

impl Global for BackgroundColor {}

/// Extension trait on [`Globals`] providing convenient access to [`ForegroundColor`] and [`BackgroundColor`].
pub trait ForegroundBackgroundColorExt {
    fn foreground_color(&self) -> &ForegroundColor;
    fn foreground_color_mut(&mut self) -> &mut ForegroundColor;
    fn background_color(&self) -> &BackgroundColor;
    fn background_color_mut(&mut self) -> &mut BackgroundColor;
}

impl ForegroundBackgroundColorExt for Globals {
    fn foreground_color(&self) -> &ForegroundColor {
        self.global::<ForegroundColor>()
    }

    fn foreground_color_mut(&mut self) -> &mut ForegroundColor {
        self.global_mut::<ForegroundColor>()
    }

    fn background_color(&self) -> &BackgroundColor {
        self.global::<BackgroundColor>()
    }

    fn background_color_mut(&mut self) -> &mut BackgroundColor {
        self.global_mut::<BackgroundColor>()
    }
}

/// Event emitted when the global [`ForegroundColor`] changes.
#[derive(Event, Debug, Clone)]
pub struct ForegroundColorChanged {
    pub old: Color,
    pub new: Color,
}

impl ForegroundColorChanged {
    pub fn new(old: Color, new: Color) -> Self {
        Self { old, new }
    }
}

/// Event emitted when the global [`BackgroundColor`] changes.
#[derive(Event, Debug, Clone)]
pub struct BackgroundColorChanged {
    pub old: Color,
    pub new: Color,
}

impl BackgroundColorChanged {
    pub fn new(old: Color, new: Color) -> Self {
        Self { old, new }
    }
}

/// Runtime plugin that initializes the default [`ForegroundColor`] and [`BackgroundColor`] globals.
pub struct ColorPlugin;

impl Plugin for ColorPlugin {
    fn build(&self, app: &mut Runtime) {
        let globals = app.globals_mut();

        // TODO: Remember the color before the app was closed and restore it on startup.
        let default_color = Color::Rgb(Rgb::new(0.0, 0.0, 0.0));
        globals.insert_global(ForegroundColor::new(default_color));
        globals.insert_global(BackgroundColor::new(default_color));
    }
}
