use iced_core::{Background, Border, Color, Padding, Theme};

#[derive(Debug, Clone)]
pub struct DockStyle {
    pub tab_bar: TabBarStyle,
    pub divider: DividerStyle,
    pub drop_indicator: DropIndicatorStyle,
}

#[derive(Debug, Clone)]
pub struct TabBarStyle {
    pub background: Background,
    pub top_gap_height: f32,
    pub tab_height: f32,
    pub tab_padding: Padding,
    pub active_tab: TabStyle,
    pub inactive_tab: TabStyle,
    pub hovered_tab: TabStyle,
    pub close_button_size: f32,
    pub close_button_color: Color,
    pub close_button_hover_color: Color,
}

#[derive(Debug, Clone)]
pub struct TabStyle {
    pub background: Background,
    pub text_color: Color,
    pub border: Border,
}

#[derive(Debug, Clone)]
pub struct DividerStyle {
    pub width: f32,
    pub color: Color,
    pub hover_color: Color,
}

#[derive(Debug, Clone)]
pub struct DropIndicatorStyle {
    pub edge_color: Color,
    pub edge_thickness: f32,
    pub merge_overlay_color: Color,
}

pub fn default_style(theme: &Theme) -> DockStyle {
    let palette = theme.palette();

    DockStyle {
        tab_bar: TabBarStyle {
            background: Background::Color(palette.background.base.color),
            top_gap_height: 0.0,
            tab_height: 25.0,
            tab_padding: Padding::new(7.0),
            active_tab: TabStyle {
                background: Background::Color(palette.background.weakest.color),
                text_color: palette.background.base.text,
                border: Border {
                    radius: 0.0.into(),
                    width: 0.0,
                    color: palette.primary.base.color,
                },
            },
            inactive_tab: TabStyle {
                background: Background::Color(palette.background.base.color),
                text_color: palette.background.weak.text,
                border: Border {
                    radius: 0.0.into(),
                    width: 0.0,
                    color: palette.background.strong.color,
                },
            },
            hovered_tab: TabStyle {
                background: Background::Color(palette.primary.weak.color),
                text_color: palette.primary.weak.text,
                border: Border {
                    radius: 0.0.into(),
                    width: 0.0,
                    color: palette.primary.base.color,
                },
            },
            close_button_size: 16.0,
            close_button_color: palette.background.weak.text,
            close_button_hover_color: palette.background.base.text,
        },
        divider: DividerStyle {
            width: 2.0,
            color: palette.background.strong.color,
            hover_color: palette.primary.base.color,
        },
        drop_indicator: DropIndicatorStyle {
            edge_color: palette.primary.base.color,
            edge_thickness: 4.0,
            merge_overlay_color: Color {
                a: 0.15,
                ..palette.primary.base.color
            },
        },
    }
}
