use std::{
    any::{Any, TypeId},
    collections::HashMap,
    sync::Arc,
};

use lapiz_utils::wrapper;
use parking_lot::RwLock;

wrapper! {
    /// Thread-safe shared handle to a typed canvas resource stored in [`CanvasResources`].
    #[derive(Debug)]
    pub CanvasResource<T> : Arc<RwLock<T>>
}

impl<T> Clone for CanvasResource<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

/// Type-indexed container of shared [`CanvasResource`]s associated with a canvas.
pub struct CanvasResources {
    resources: HashMap<TypeId, Box<dyn Any + Send + Sync>>,
}

impl Default for CanvasResources {
    fn default() -> Self {
        Self::new()
    }
}

impl CanvasResources {
    pub fn new() -> Self {
        Self {
            resources: HashMap::new(),
        }
    }

    pub fn resource<T: Send + Sync + 'static>(&self) -> CanvasResource<T> {
        self.resources
            .get(&TypeId::of::<T>())
            .unwrap()
            .downcast_ref::<CanvasResource<T>>()
            .unwrap()
            .clone()
    }

    pub fn set<T>(&mut self, resource: T)
    where
        T: Send + Sync + 'static,
    {
        self.resources.insert(
            TypeId::of::<T>(),
            Box::new(CanvasResource::new(Arc::new(RwLock::new(resource)))),
        );
    }
}
