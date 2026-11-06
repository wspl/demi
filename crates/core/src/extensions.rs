//! What whoever starts a core keeps in it for its own use, such as the
//! server's state of the user's Cloud: one value of each type, given at the
//! start and never replaced, which the core itself never reads.

use std::any::{Any, TypeId};
use std::collections::HashMap;

/// The values a core keeps for whoever started it, by type.
#[derive(Default)]
pub struct Extensions {
    values: HashMap<TypeId, Box<dyn Any>>,
}

impl Extensions {
    /// Keeps `value`, in place of the one of its type kept before.
    pub fn with<T: Any>(mut self, value: T) -> Self {
        self.values.insert(TypeId::of::<T>(), Box::new(value));
        self
    }

    /// The value of type `T`, if one was given.
    pub fn get<T: Any>(&self) -> Option<&T> {
        self.values.get(&TypeId::of::<T>())?.downcast_ref()
    }
}
