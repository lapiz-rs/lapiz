use std::{any::Any, collections::HashMap};

use iced_core::{
    Border, Element, Length, Point, Rectangle, Size, Widget,
    alignment::Vertical,
    layout, mouse,
    renderer::{self, Quad},
    text::IntoFragment,
    widget::{Operation, Tree},
};
use iced_widget::{row, text};

use crate::{
    GraphRenderer, GraphTheme,
    editor::themed_color,
    graph::slot::{
        ErasedGraphLiteralUpdateMessage, GraphInputSlotData, GraphInputSlotId, GraphOutputSlotData,
        GraphOutputSlotId,
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GraphSlotId {
    Input(GraphInputSlotId),
    Output(GraphOutputSlotId),
}

impl From<GraphInputSlotId> for GraphSlotId {
    fn from(value: GraphInputSlotId) -> Self {
        GraphSlotId::Input(value)
    }
}

impl From<GraphOutputSlotId> for GraphSlotId {
    fn from(value: GraphOutputSlotId) -> Self {
        GraphSlotId::Output(value)
    }
}

#[derive(Default)]
pub struct GraphSlotPinPositionCollection {
    slots: HashMap<GraphSlotId, Point>,
}

impl Operation for GraphSlotPinPositionCollection {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<()>)) {
        operate(self);
    }

    fn custom(&mut self, _id: Option<&iced_widget::Id>, bounds: Rectangle, state: &mut dyn Any) {
        if let Some(state) = state.downcast_ref::<SlotPinState>() {
            self.slots.insert(state.id, bounds.center());
        }
    }
}

impl GraphSlotPinPositionCollection {
    pub fn get(&self, slot_id: &GraphSlotId) -> Option<&Point> {
        self.slots.get(slot_id)
    }

    pub fn get_input(&self, slot_id: &GraphInputSlotId) -> Option<&Point> {
        self.slots.get(&GraphSlotId::Input(*slot_id))
    }

    pub fn get_output(&self, slot_id: &GraphOutputSlotId) -> Option<&Point> {
        self.slots.get(&GraphSlotId::Output(*slot_id))
    }

    pub fn clear(&mut self) {
        self.slots.clear();
    }

    pub fn all(&self) -> impl Iterator<Item = (&GraphSlotId, &Point)> {
        self.slots.iter()
    }
}

pub struct SlotPin {
    pub id: GraphSlotId,
    pub size: f32,
    pub side: SlotSide,
    pub hue: f32,
    pub chroma: f32,
}

impl SlotPin {
    fn overhanging_bounds(&self, bounds: Rectangle) -> Rectangle {
        let mut bounds = bounds;
        bounds.x += match self.side {
            SlotSide::Left => -self.size / 2.0,
            SlotSide::Right => self.size / 2.0,
        };
        bounds
    }
}

impl<Message> Widget<Message, GraphTheme, GraphRenderer> for SlotPin {
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fixed(self.size), Length::Fixed(self.size))
    }

    fn layout(
        &mut self,
        _tree: &mut Tree,
        _renderer: &GraphRenderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        layout::atomic(limits, self.size, self.size)
    }

    fn draw(
        &self,
        _tree: &Tree,
        renderer: &mut GraphRenderer,
        theme: &GraphTheme,
        _style: &renderer::Style,
        layout: layout::Layout<'_>,
        _cursor: mouse::Cursor,
        _viewport: &Rectangle,
    ) {
        iced_core::Renderer::fill_quad(
            renderer,
            Quad {
                bounds: self.overhanging_bounds(layout.bounds()),
                border: Border::default().width(1.0).color(
                    theme
                        .extended_palette()
                        .background
                        .base
                        .text
                        .scale_alpha(0.4),
                ),
                ..Default::default()
            },
            themed_color(theme, self.hue, self.chroma),
        );
    }

    fn operate(
        &mut self,
        _tree: &mut Tree,
        layout: layout::Layout<'_>,
        _renderer: &GraphRenderer,
        operation: &mut dyn Operation,
    ) {
        operation.custom(
            None,
            self.overhanging_bounds(layout.bounds()),
            &mut SlotPinState { id: self.id },
        );
    }
}

struct SlotPinState {
    id: GraphSlotId,
}

#[derive(Clone, Copy)]
pub enum SlotSide {
    Left,
    Right,
}

pub fn input_slot<'a>(
    slot_id: GraphInputSlotId,
    slot_name: impl IntoFragment<'a>,
    slot: &GraphInputSlotData,
) -> Element<'a, ErasedGraphLiteralUpdateMessage, GraphTheme, GraphRenderer> {
    let (hue, chroma) = slot.data.ty().hue_chroma();
    match slot.connected {
        Some(_) => empty_slot(slot_id.into(), hue, chroma, slot_name, SlotSide::Left),
        None => valued_slot(
            slot_id.into(),
            hue,
            chroma,
            slot_name,
            SlotSide::Left,
            slot.data.ty().view_literal(slot_id, slot.data.value()),
        ),
    }
}

pub fn output_slot<'a, Message>(
    slot_id: GraphOutputSlotId,
    slot_name: impl IntoFragment<'a>,
    slot: &GraphOutputSlotData,
) -> Element<'a, Message, GraphTheme, GraphRenderer>
where
    Message: 'a,
{
    let (hue, chroma) = slot.data_ty.hue_chroma();
    empty_slot(slot_id.into(), hue, chroma, slot_name, SlotSide::Right)
}

pub fn empty_slot<'a, Message>(
    id: GraphSlotId,
    hue: f32,
    chroma: f32,
    name: impl IntoFragment<'a>,
    slot_side: SlotSide,
) -> Element<'a, Message, GraphTheme, GraphRenderer>
where
    Message: 'a,
{
    let text = text(name).size(12);
    let pin = Element::new(SlotPin {
        id,
        hue,
        chroma,
        size: 9.0,
        side: slot_side,
    });

    match slot_side {
        SlotSide::Left => row![pin, text]
            .width(Length::Fill)
            .align_y(Vertical::Center)
            .spacing(4)
            .into(),
        SlotSide::Right => row![iced_widget::space().width(Length::Fill), text, pin]
            .width(Length::Fill)
            .align_y(Vertical::Center)
            .spacing(4)
            .into(),
    }
}

pub fn valued_slot<'a, Message>(
    id: GraphSlotId,
    hue: f32,
    chroma: f32,
    name: impl IntoFragment<'a>,
    slot_side: SlotSide,
    widget: Element<'a, Message, GraphTheme, GraphRenderer>,
) -> Element<'a, Message, GraphTheme, GraphRenderer>
where
    Message: 'a,
{
    let text = text(name).size(12);
    let pin = Element::new(SlotPin {
        id,
        hue,
        chroma,
        size: 9.0,
        side: slot_side,
    });

    match slot_side {
        SlotSide::Left => row![pin, text, widget]
            .width(Length::Fill)
            .align_y(Vertical::Center)
            .spacing(4)
            .into(),
        SlotSide::Right => row![iced_widget::space().width(Length::Fill), widget, text, pin]
            .width(Length::Fill)
            .align_y(Vertical::Center)
            .spacing(4)
            .into(),
    }
}
