use std::{
    any::{Any, TypeId, type_name},
    collections::HashMap,
};

/// Marker trait for singleton resources stored in [`Globals`].
///
/// Any `'static` type can implement `Global` to be managed by [`Globals`] and [`Runtime`](crate::Runtime).
/// Plugins typically register globals during startup via [`Runtime::add_global`](crate::Runtime::add_global)
/// or [`Runtime::add_global_instance`](crate::Runtime::add_global_instance).
pub trait Global: 'static {}

/// Trait for constructing a [`Global`] resource from existing state in [`Globals`].
///
/// Automatically implemented for all types that implement [`Default`]. Types that depend on
/// previously registered globals can implement `FromGlobals` manually to initialize themselves
/// when added via [`Runtime::add_global`](crate::Runtime::add_global).
pub trait FromGlobals {
    fn from_globals(globals: &Globals) -> Self;
}

impl<T: Default> FromGlobals for T {
    fn from_globals(_globals: &Globals) -> Self {
        Self::default()
    }
}

/// Type-indexed container storing singleton [`Global`] resources for the application.
///
/// `Globals` acts as the central shared state store passed to [`Plugin`](crate::plugin::Plugin)s,
/// [`WindowView`](crate::windows::WindowView)s, action functions, and undo commands. Each [`Global`]
/// type is keyed by its [`TypeId`].
///
/// # Access Patterns
///
/// - **Direct Borrowing**: Use [`global`](Globals::global) and [`global_mut`](Globals::global_mut)
///   when the resource is guaranteed to be registered, or [`get_global`](Globals::get_global) and
///   [`get_global_mut`](Globals::get_global_mut) for fallible lookups.
/// - **Scoped Updates**: Use [`update_global`](Globals::update_global) or
///   [`try_update_scope`](Globals::try_update_scope) to temporarily remove a global, mutate it
///   alongside `&mut Globals`, and reinsert it automatically.
#[derive(Default)]
pub struct Globals {
    globals: HashMap<TypeId, Box<dyn Any>>,
}

impl Globals {
    pub fn global<T: Global>(&self) -> &T {
        self.globals
            .get(&TypeId::of::<T>())
            .unwrap_or_else(|| panic!("Global of type {} not found", type_name::<T>()))
            .downcast_ref()
            .unwrap_or_else(|| {
                panic!(
                    "Global of type {} has wrong type. This should not happen.",
                    type_name::<T>()
                )
            })
    }

    pub fn global_mut<T: Global>(&mut self) -> &mut T {
        self.globals
            .get_mut(&TypeId::of::<T>())
            .unwrap_or_else(|| panic!("Global of type {} not found", type_name::<T>()))
            .downcast_mut()
            .unwrap_or_else(|| {
                panic!(
                    "Global of type {} has wrong type. This should not happen.",
                    type_name::<T>()
                )
            })
    }

    pub fn has_global<T: Global>(&self) -> bool {
        self.globals.contains_key(&TypeId::of::<T>())
    }

    pub fn get_global<T: Global>(&self) -> Option<&T> {
        self.globals
            .get(&TypeId::of::<T>())
            .and_then(|value| value.downcast_ref())
    }

    pub fn get_global_mut<T: Global>(&mut self) -> Option<&mut T> {
        self.globals
            .get_mut(&TypeId::of::<T>())
            .and_then(|value| value.downcast_mut())
    }

    pub fn remove_global<T: Global>(&mut self) -> T {
        let s = self
            .globals
            .remove(&TypeId::of::<T>())
            .unwrap_or_else(|| panic!("Global of type {} not found", type_name::<T>()));

        match s.downcast() {
            Ok(s) => *s,
            Err(_) => {
                panic!(
                    "Global of type {} has wrong type. This should not happen.",
                    type_name::<T>()
                )
            }
        }
    }

    pub fn try_remove_global<T: Global>(&mut self) -> Option<T> {
        let s = self.globals.remove(&TypeId::of::<T>())?;

        match s.downcast() {
            Ok(s) => Some(*s),
            Err(_) => {
                panic!(
                    "Global of type {} has wrong type. This should not happen.",
                    type_name::<T>()
                )
            }
        }
    }

    pub fn insert_global<T: Global>(&mut self, value: T) {
        self.globals.insert(TypeId::of::<T>(), Box::new(value));
    }

    pub fn update_global<T: Global, O>(&mut self, f: impl FnOnce(&mut T, &mut Self) -> O) -> O {
        let mut s = self.remove_global::<T>();
        let result = f(&mut s, self);
        self.insert_global(s);
        result
    }

    pub fn try_update_scope<T: Global, O>(
        &mut self,
        f: impl FnOnce(&mut T, &mut Self) -> O,
    ) -> Option<O> {
        let mut s = self.try_remove_global::<T>()?;
        let result = f(&mut s, self);
        self.insert_global(s);
        Some(result)
    }
}
