use std::any::{Any, TypeId};
use std::sync::Arc;

/// A service a plugin depends on: its type and a readable name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DepKey {
    pub id: TypeId,
    pub name: &'static str,
}

impl DepKey {
    pub fn of<T: ?Sized + 'static>() -> Self {
        let full = std::any::type_name::<T>();
        DepKey {
            id: TypeId::of::<T>(),
            name: full.rsplit("::").next().unwrap_or(full),
        }
    }
}

/// Looks a service up by type; the value is the `Arc<T>` it was provided as.
pub type Lookup<'a> = &'a dyn Fn(TypeId) -> Option<Arc<dyn Any + Send + Sync>>;

/// What `Plugin::apply` receives: every service in it, or the plugin waits.
pub trait Deps: Sized + Send + 'static {
    fn keys() -> Vec<DepKey>;
    fn resolve(lookup: Lookup<'_>) -> Option<Self>;
}

impl Deps for () {
    fn keys() -> Vec<DepKey> {
        Vec::new()
    }

    fn resolve(_: Lookup<'_>) -> Option<Self> {
        Some(())
    }
}

impl<T: ?Sized + Send + Sync + 'static> Deps for Arc<T> {
    fn keys() -> Vec<DepKey> {
        vec![DepKey::of::<T>()]
    }

    fn resolve(lookup: Lookup<'_>) -> Option<Self> {
        lookup(TypeId::of::<T>())?.downcast_ref::<Arc<T>>().cloned()
    }
}

macro_rules! tuple_deps {
    ($($name:ident),+) => {
        impl<$($name: Deps),+> Deps for ($($name,)+) {
            fn keys() -> Vec<DepKey> {
                let mut keys = Vec::new();
                $(keys.extend($name::keys());)+
                keys
            }

            fn resolve(lookup: Lookup<'_>) -> Option<Self> {
                Some(($($name::resolve(lookup)?,)+))
            }
        }
    };
}

tuple_deps!(A, B);
tuple_deps!(A, B, C);
tuple_deps!(A, B, C, D);
