use crate::Services;

pub trait Service: 'static {}

pub trait FromServices {
    fn from_services(services: &Services) -> Self;
}

impl<T: Default> FromServices for T {
    fn from_services(_services: &Services) -> Self {
        Self::default()
    }
}
