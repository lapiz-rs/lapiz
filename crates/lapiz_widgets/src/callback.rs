use std::mem;

/// A deferred message producer invoked by interactive widgets when an event occurs.
///
/// Supports holding no action (`Empty`), a pre-constructed one-shot message value (`Value`),
/// or a repeatable closure (`Func`).
#[derive(Default)]
pub enum Callback<'a, Message> {
    #[default]
    Empty,
    Value(Message),
    Func(Box<dyn Fn() -> Message + 'a>),
}

impl<'a, Message> Callback<'a, Message> {
    pub fn is_set(&self) -> bool {
        !matches!(self, Callback::Empty)
    }
}

pub type CallbackWith<'a, Input, Message> = Option<Box<dyn Fn(Input) -> Message + 'a>>;

pub fn publish<Message>(callback: &mut Callback<'_, Message>) -> Option<Message> {
    match mem::replace(callback, Callback::Empty) {
        Callback::Empty => None,
        Callback::Value(message) => Some(message),
        Callback::Func(func) => {
            let message = func();
            *callback = Callback::Func(func);
            Some(message)
        }
    }
}

pub fn publish_with<Input, Message>(
    callback: &mut CallbackWith<'_, Input, Message>,
    input: Input,
) -> Option<Message> {
    callback.as_ref().map(|callback| callback(input))
}

#[macro_export]
macro_rules! callback_methods {
    ($field:ident) => {
        $crate::__private::paste! {
            pub fn [<on_ $field>](mut self, message: Message) -> Self {
                self.$field = $crate::callback::Callback::Value(message);
                self
            }

            pub fn [<on_ $field _maybe>](mut self, message: Option<Message>) -> Self {
                self.$field = match message {
                    Some(message) => $crate::callback::Callback::Value(message),
                    None => $crate::callback::Callback::Empty,
                };
                self
            }

            pub fn [<on_ $field _with>](mut self, callback: impl Fn() -> Message + 'a) -> Self {
                self.$field = $crate::callback::Callback::Func(Box::new(callback));
                self
            }

            #[allow(dead_code, reason = "generated optional callback setter is not used by every widget")]
            pub(crate) fn [<on_ $field _with_callback>](
                mut self,
                callback: $crate::callback::Callback<'a, Message>,
            ) -> Self {
                self.$field = callback;
                self
            }
        }
    };
    ($field:ident, $input:ty) => {
        $crate::__private::paste! {
            pub fn [<on_ $field>](mut self, callback: impl Fn($input) -> Message + 'a) -> Self {
                self.$field = Some(Box::new(callback));
                self
            }

            pub fn [<on_ $field _maybe>]<F>(mut self, callback: Option<F>) -> Self
            where
                F: Fn($input) -> Message + 'a,
            {
                self.$field = callback.map(|callback| Box::new(callback) as _);
                self
            }
        }
    };
}
