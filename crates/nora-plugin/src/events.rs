//! Events: typed, dispatched in one of four modes chosen by the emitter.
//!
//! | Mode | Order | Result |
//! |---|---|---|
//! | `emit` | every listener starts at once | ignored; failures only logged |
//! | `parallel` | concurrent, waits for all | failures collected in registration order |
//! | `serial` | one at a time | the first `Some` stops the dispatch |
//! | `waterfall` | first registered is outermost | each listener calls `next` for the rest |

use std::future::{Future, IntoFuture};
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};

use crate::{Error, Result};

pub(crate) type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send + 'static>>;

/// A type that can be emitted. `Value` is what `serial` and `waterfall` return.
pub trait Event: Send + Sync + 'static {
    const NAME: &'static str;
    type Value: Send + 'static;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Emit,
    Parallel,
    Serial,
    Waterfall,
}

/// A listener of `E`: `|event, call| async move { … }`.
pub trait Listener<E: Event>:
    Fn(Arc<E>, Call<E>) -> <Self as Listener<E>>::Fut + Send + Sync + 'static
{
    type Fut: Future<Output = Result<Option<E::Value>>> + Send + 'static;
}

impl<E, F, Fut> Listener<E> for F
where
    E: Event,
    F: Fn(Arc<E>, Call<E>) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<Option<E::Value>>> + Send + 'static,
{
    type Fut = Fut;
}

pub(crate) type ListenerFn<E> =
    Arc<dyn Fn(Arc<E>, Call<E>) -> BoxFuture<Result<Option<<E as Event>::Value>>> + Send + Sync>;

type TerminalFn<E> =
    Box<dyn FnOnce(Arc<E>) -> BoxFuture<Result<Option<<E as Event>::Value>>> + Send>;

/// One call of a listener: the mode, and in a waterfall the rest of the chain.
pub struct Call<E: Event> {
    mode: Mode,
    event: Arc<E>,
    chain: Arc<Vec<ListenerFn<E>>>,
    index: usize,
    terminal: Arc<Mutex<Option<TerminalFn<E>>>>,
}

impl<E: Event> Call<E> {
    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// Runs the rest of a waterfall and returns its result. Outside a
    /// waterfall there is no rest: `None`.
    pub async fn next(self) -> Result<Option<E::Value>> {
        if self.mode != Mode::Waterfall {
            return Ok(None);
        }
        if let Some(listener) = self.chain.get(self.index) {
            let rest = Call {
                index: self.index + 1,
                mode: self.mode,
                event: Arc::clone(&self.event),
                chain: Arc::clone(&self.chain),
                terminal: Arc::clone(&self.terminal),
            };
            return listener(Arc::clone(&self.event), rest).await;
        }
        let terminal = self
            .terminal
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        match terminal {
            Some(terminal) => terminal(self.event).await,
            None => Ok(None),
        }
    }
}

fn call<E: Event>(
    mode: Mode,
    event: &Arc<E>,
    chain: &Arc<Vec<ListenerFn<E>>>,
    index: usize,
) -> Call<E> {
    Call {
        mode,
        event: Arc::clone(event),
        chain: Arc::clone(chain),
        index,
        terminal: Arc::new(Mutex::new(None)),
    }
}

pub(crate) fn emit<E: Event>(event: E, listeners: Vec<ListenerFn<E>>) {
    let event = Arc::new(event);
    let chain = Arc::new(listeners);
    for (index, listener) in chain.iter().enumerate() {
        let run = listener(
            Arc::clone(&event),
            call(Mode::Emit, &event, &chain, index + 1),
        );
        tokio::spawn(async move {
            if let Err(error) = run.await {
                eprintln!("{}: a listener failed: {error}", E::NAME);
            }
        });
    }
}

pub(crate) async fn parallel<E: Event>(event: E, listeners: Vec<ListenerFn<E>>) -> Result {
    let event = Arc::new(event);
    let chain = Arc::new(listeners);
    let runs: Vec<_> = chain
        .iter()
        .enumerate()
        .map(|(index, listener)| {
            tokio::spawn(listener(
                Arc::clone(&event),
                call(Mode::Parallel, &event, &chain, index + 1),
            ))
        })
        .collect();
    let mut failures = Vec::new();
    for run in runs {
        match run.await {
            Ok(Ok(_)) => {}
            Ok(Err(error)) => failures.push(error),
            Err(join) => failures.push(Error::Panicked {
                plugin: E::NAME.into(),
                message: join.to_string(),
            }),
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(Error::ListenersFailed(failures))
    }
}

pub(crate) async fn serial<E: Event>(
    event: E,
    listeners: Vec<ListenerFn<E>>,
) -> Result<Option<E::Value>> {
    let event = Arc::new(event);
    let chain = Arc::new(listeners);
    for (index, listener) in chain.iter().enumerate() {
        let result = listener(
            Arc::clone(&event),
            call(Mode::Serial, &event, &chain, index + 1),
        )
        .await?;
        if result.is_some() {
            return Ok(result);
        }
    }
    Ok(None)
}

/// A waterfall to run: optionally `.terminal(…)`, then `.await`.
pub struct Waterfall<E: Event> {
    event: E,
    listeners: Vec<ListenerFn<E>>,
    terminal: Option<TerminalFn<E>>,
}

impl<E: Event> Waterfall<E> {
    pub(crate) fn new(event: E, listeners: Vec<ListenerFn<E>>) -> Self {
        Waterfall {
            event,
            listeners,
            terminal: None,
        }
    }

    /// What the innermost `next` returns; without one, `None`.
    pub fn terminal<F, Fut>(mut self, terminal: F) -> Self
    where
        F: FnOnce(Arc<E>) -> Fut + Send + 'static,
        Fut: Future<Output = Result<Option<E::Value>>> + Send + 'static,
    {
        self.terminal = Some(Box::new(move |event| Box::pin(terminal(event))));
        self
    }
}

impl<E: Event> IntoFuture for Waterfall<E> {
    type Output = Result<Option<E::Value>>;
    type IntoFuture = BoxFuture<Self::Output>;

    fn into_future(self) -> Self::IntoFuture {
        let start = Call {
            mode: Mode::Waterfall,
            event: Arc::new(self.event),
            chain: Arc::new(self.listeners),
            index: 0,
            terminal: Arc::new(Mutex::new(self.terminal)),
        };
        Box::pin(start.next())
    }
}
