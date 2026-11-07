use std::fmt;
use std::sync::Arc;

pub type Result<T = ()> = std::result::Result<T, Error>;

/// What the kernel or a plugin reports. Cheap to clone: a failure is shown
/// to every waiter and kept in diagnostics.
#[derive(Clone)]
pub enum Error {
    /// A plugin's own failure, as it returned it.
    Plugin(Arc<dyn std::error::Error + Send + Sync>),
    /// A plugin panicked; the kernel and the other plugins carry on.
    Panicked { plugin: String, message: String },
    /// A second provider for a service that already has one.
    AlreadyProvided { service: &'static str },
    /// A registration through a context whose plugin is no longer running.
    Inactive,
    /// The plugin was disposed before it became active.
    Disposed,
    /// The kernel is shutting down or its driver has stopped.
    ShuttingDown,
    /// Listeners of a `parallel` dispatch failed, in registration order.
    ListenersFailed(Vec<Error>),
}

impl<E: std::error::Error + Send + Sync + 'static> From<E> for Error {
    fn from(error: E) -> Self {
        Error::Plugin(Arc::new(error))
    }
}

impl From<Error> for Box<dyn std::error::Error + Send + Sync> {
    fn from(error: Error) -> Self {
        error.to_string().into()
    }
}

impl From<Error> for Box<dyn std::error::Error> {
    fn from(error: Error) -> Self {
        error.to_string().into()
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Plugin(error) => write!(f, "{error}"),
            Error::Panicked { plugin, message } => write!(f, "{plugin} panicked: {message}"),
            Error::AlreadyProvided { service } => write!(f, "{service} is already provided"),
            Error::Inactive => write!(f, "the plugin is no longer running"),
            Error::Disposed => write!(f, "the plugin was disposed"),
            Error::ShuttingDown => write!(f, "the kernel is shutting down"),
            Error::ListenersFailed(errors) => {
                let all: Vec<String> = errors.iter().map(ToString::to_string).collect();
                write!(f, "listeners failed: {}", all.join("; "))
            }
        }
    }
}

impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}
