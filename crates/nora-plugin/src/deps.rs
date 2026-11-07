use std::any::{Any, TypeId};
use std::hash::{Hash, Hasher};
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

/// A service identity: its Rust type, optionally qualified by an owned name.
/// Display names never take part in identity, so custom `Deps` keep working.
#[derive(Clone, Debug)]
pub struct ServiceKey {
    id: TypeId,
    name: Option<Arc<str>>,
    type_name: &'static str,
}

impl ServiceKey {
    pub fn of<T: ?Sized + 'static>() -> Self {
        DepKey::of::<T>().into()
    }

    pub fn named<T: ?Sized + 'static>(name: impl Into<String>) -> Self {
        Self {
            name: Some(Arc::from(name.into())),
            ..Self::of::<T>()
        }
    }

    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    pub fn type_id(&self) -> TypeId {
        self.id
    }

    pub fn type_name(&self) -> &'static str {
        self.type_name
    }

    pub(crate) fn from_type_id(id: TypeId) -> Self {
        Self {
            id,
            name: None,
            type_name: "service",
        }
    }
}

impl From<DepKey> for ServiceKey {
    fn from(key: DepKey) -> Self {
        Self {
            id: key.id,
            name: None,
            type_name: key.name,
        }
    }
}

impl PartialEq for ServiceKey {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id && self.name == other.name
    }
}

impl Eq for ServiceKey {}

impl Hash for ServiceKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id.hash(state);
        self.name.hash(state);
    }
}

impl std::fmt::Display for ServiceKey {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.name() {
            Some(name) => write!(formatter, "{name} ({})", self.type_name),
            None => formatter.write_str(self.type_name),
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
