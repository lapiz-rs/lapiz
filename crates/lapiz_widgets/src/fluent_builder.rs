/// Fluent builder extension trait for conditionally applying a transformation when an [`Option`] is [`Some`].
///
/// Implemented for all [`Sized`] types, allowing method chains to unwrap and apply optional
/// configuration inline without breaking builder flow.
pub trait WhenSome {
    fn when_some<T>(self, val: Option<T>, f: impl FnOnce(Self, T) -> Self) -> Self
    where
        Self: Sized;
}

impl<W> WhenSome for W {
    fn when_some<T>(self, val: Option<T>, f: impl FnOnce(Self, T) -> Self) -> Self
    where
        Self: Sized,
    {
        if let Some(val) = val {
            f(self, val)
        } else {
            self
        }
    }
}

/// Fluent builder extension trait for conditionally applying a transformation when a boolean predicate holds.
///
/// Implemented for all [`Sized`] types, enabling concise conditional configuration inside widget
/// builder chains.
pub trait When {
    fn when(self, condition: bool, f: impl FnOnce(Self) -> Self) -> Self
    where
        Self: Sized;
}

impl<W> When for W {
    fn when(self, condition: bool, f: impl FnOnce(Self) -> Self) -> Self
    where
        Self: Sized,
    {
        if condition { f(self) } else { self }
    }
}
