use std::convert::Infallible;

use iced_core::{Element, Length, Theme};
use lapiz_runtime::Renderer;
use lapiz_widgets::{component::Component, space};

pub struct ToolBindingsPage {}

impl<'a, Message> Component<'a, Message, Theme, Renderer> for ToolBindingsPage {
    type State = ();
    type Event = Infallible;

    fn update(&mut self, _: &mut Self::State, event: Self::Event, _: &Renderer) -> Option<Message> {
        match event {}
    }

    fn view(&self, _: &Self::State) -> Element<'a, Self::Event, Theme, Renderer> {
        space().width(Length::Fill).height(Length::Fill).into()
    }
}
