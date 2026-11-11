use bevy_math::{IRect, Rect, URect};
use glam::{IVec2, UVec2, Vec2};
use iced_core::Rectangle;

/// Extension trait for converting UI rectangle types into `bevy_math` rectangle representations.
pub trait IntoRect<T> {
    fn into_rect(self) -> T;
}

macro_rules! impl_into_rect {
    ($bevy_ty:ident, $bevy_vec:ident, $primitive:ident) => {
        impl IntoRect<$bevy_ty> for Rectangle<$primitive> {
            fn into_rect(self) -> $bevy_ty {
                $bevy_ty {
                    min: $bevy_vec {
                        x: self.x,
                        y: self.y,
                    },
                    max: $bevy_vec {
                        x: self.x + self.width,
                        y: self.y + self.height,
                    },
                }
            }
        }
    };
}

impl_into_rect!(Rect, Vec2, f32);
impl_into_rect!(URect, UVec2, u32);
impl_into_rect!(IRect, IVec2, i32);
