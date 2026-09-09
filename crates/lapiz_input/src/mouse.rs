use glam::Vec2;
use iced_core::{
    Point,
    pointer::{self, button, tablet},
};

#[derive(Debug, Clone)]
pub struct PressedMouseState {
    pub position: Point,
    pub force: f32,
    pub tilt: Vec2,
    pub altitude: f32,
    pub azimuth: f32,
    pub twist: f32,
    pub tangential_force: f32,
}

impl PressedMouseState {
    pub fn without_tablet_data(position: Point) -> Self {
        Self {
            position,
            force: 1.0,
            tilt: Vec2::ZERO,
            altitude: std::f32::consts::FRAC_2_PI,
            azimuth: 0.0,
            twist: 0.0,
            tangential_force: 0.0,
        }
    }

    pub fn from_button(position: Point, source: button::Source) -> Self {
        match source {
            button::Source::Mouse(..)
            | button::Source::Touch { .. }
            | button::Source::Unknown(..) => PressedMouseState::without_tablet_data(position),
            button::Source::TabletTool { data, .. } => {
                PressedMouseState::from_tablet_data(position, data)
            }
        }
    }

    pub fn from_pointer(position: Point, source: pointer::Source) -> Self {
        match source {
            pointer::Source::Mouse | pointer::Source::Touch { .. } | pointer::Source::Unknown => {
                PressedMouseState::without_tablet_data(position)
            }
            pointer::Source::TabletTool { data, .. } => Self::from_tablet_data(position, data),
        }
    }

    pub fn from_tablet_data(position: Point, data: tablet::Data) -> Self {
        let tilt = data.clone().tilt().unwrap_or_default();
        let angle = data.clone().angle().unwrap_or_default();

        PressedMouseState {
            position,
            force: data
                .force
                .map(|f| match f {
                    pointer::touch::Force::Calibrated {
                        force,
                        max_possible_force,
                    } => (force / max_possible_force).clamp(0.0, 1.0) as f32,
                    pointer::touch::Force::Normalized(f) => f as f32,
                })
                .unwrap_or(1.0),
            tilt: Vec2::new((tilt.x as f32).to_radians(), (tilt.y as f32).to_radians()),
            altitude: (angle.altitude as f32).to_radians(),
            azimuth: (angle.azimuth as f32).to_radians(),
            twist: (data.twist.unwrap_or_default() as f32).to_radians(),
            tangential_force: data.tangential_force.unwrap_or_default(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct HoverMouseState {
    pub position: Point,
    pub tilt: Vec2,
    pub altitude: f32,
    pub azimuth: f32,
    pub twist: f32,
}

impl HoverMouseState {
    pub fn without_tablet_data(position: Point) -> Self {
        Self {
            position,
            tilt: Vec2::default(),
            altitude: std::f32::consts::FRAC_2_PI,
            azimuth: 0.0,
            twist: 0.0,
        }
    }

    pub fn from_pointer(position: Point, source: pointer::Source) -> Self {
        match source {
            pointer::Source::Mouse | pointer::Source::Touch { .. } | pointer::Source::Unknown => {
                Self::without_tablet_data(position)
            }
            pointer::Source::TabletTool { data, .. } => Self::from_tablet_data(position, data),
        }
    }

    pub fn from_tablet_data(position: Point, data: tablet::Data) -> Self {
        let tilt = data.clone().tilt().unwrap_or_default();
        let angle = data.clone().angle().unwrap_or_default();

        Self {
            position,
            tilt: Vec2::new((tilt.x as f32).to_radians(), (tilt.y as f32).to_radians()),
            altitude: (angle.altitude as f32).to_radians(),
            azimuth: (angle.azimuth as f32).to_radians(),
            twist: (data.twist.unwrap_or_default() as f32).to_radians(),
        }
    }
}
