use std::fmt::Display;

use tracing::error;

/// Extension trait for logging `Err` variants of a [`Result`] via `tracing::error!`.
pub trait LogErr {
    fn log_err(self);
    fn logged_err(self) -> Self;
}

impl<T, E: Display> LogErr for Result<T, E> {
    fn logged_err(self) -> Self {
        if let Err(e) = &self {
            error!("{}", e);
        }
        self
    }

    fn log_err(self) {
        if let Err(e) = &self {
            error!("{}", e);
        }
    }
}

/// Extension trait for logging `None` variants of an [`Option`] via `tracing::error!`.
pub trait LogNone {
    fn log_none(self, msg: &str);
    fn logged_none(self, msg: &str) -> Self;
}

impl<T: Display> LogNone for Option<T> {
    fn log_none(self, msg: &str) {
        if self.is_none() {
            error!("{}", msg);
        }
    }

    fn logged_none(self, msg: &str) -> Self {
        if self.is_none() {
            error!("{}", msg);
        }
        self
    }
}
