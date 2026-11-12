use downcast_rs::DowncastSync;
use dyn_clone::DynClone;

/// Trait for thread-safe, cloneable, and downcastable trait objects.
///
/// Automatically implemented for any type satisfying `Clone + Send + Sync + 'static`.
pub trait ClonableAnySync: DynClone + Send + Sync + 'static + DowncastSync {}

impl<T> ClonableAnySync for T where T: Clone + Send + Sync + 'static {}

dyn_clone::clone_trait_object!(ClonableAnySync);

downcast_rs::impl_downcast!(ClonableAnySync);
