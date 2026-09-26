//! Multi-window support for Android.
//!
//! An Android application can only own a single native window, which in turn
//! can only back a single graphics surface (see `vkCreateAndroidSurfaceKHR`).
//!
//! This module runs a [`Program`] against that single native window while
//! emulating the multi-window API of [`crate::runtime::window`] with
//! logical windows: every [`window::Id`] is backed by a
//! [`crate::android::container::Windows`] container child that is laid out
//! at a fixed position and size. The whole application is a single
//! [`UserInterface`](crate::runtime::user_interface::UserInterface), so a
//! single graphics surface and a single frame per redraw drive every
//! logical window.
//!
//! Logical windows are inherently undecorated. `window::open` spawns a new
//! logical window, `window::close` removes it, `window::resize`/`move_to`
//! change its geometry, and `window::drag` starts an interactive move
//! session driven by the pointer, mirroring how an undecorated window
//! behaves on the desktop platforms. A pointer press on the border of a
//! resizable logical window starts an interactive resize session, like the
//! resize border of a decorated window.
//!
//! Commands that cannot be emulated—like `window::screenshot` and
//! `window::minimize`—log a warning and are ignored.

mod actions;
mod container;
mod input;
mod instance;
mod manager;
mod render;
mod runner;

pub use runner::run;
