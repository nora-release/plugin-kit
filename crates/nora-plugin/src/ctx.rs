//! The kernel: plugin states, the driver task and the service registry.
//!
//! Every lifecycle transition (start, stop, dispose) runs on one driver task,
//! one at a time. Registrations made through a `Ctx` only change the registry
//! and wake the driver, so `apply` can provide services and mount children
//! without re-entering the driver.

use std::any::{Any, TypeId};
use std::collections::{BTreeMap, HashMap};
use std::future::{Future, IntoFuture};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use tokio::sync::{mpsc, oneshot, watch};

use crate::deps::{DepKey, Deps, ServiceKey};
use crate::events::{self, BoxFuture, Event, Listener, ListenerFn, Waterfall};
use crate::gate::Gate;
use crate::{Error, Plugin, Result};

type FiberId = u64;
const ROOT: FiberId = 0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    /// Waiting for a dependency.
    Pending,
    /// `apply` is running.
    Loading,
    Active,
    /// `apply` failed; retried when a dependency changes.
    Failed,
    /// Its cleanups are running.
    Unloading,
    Disposed,
}

#[derive(Clone, Debug)]
pub struct Snapshot {
    pub state: State,
    /// How many times `apply` has started.
    pub generation: u32,
    pub error: Option<Error>,
}

#[derive(Clone, Debug)]
pub struct PluginInfo {
    pub id: u64,
    pub name: String,
    pub parent: Option<u64>,
    pub state: State,
    pub generation: u32,
    /// Each dependency and whether a provider exists for it now.
    pub deps: Vec<(&'static str, bool)>,
    pub error: Option<Error>,
}

/// A required service and whether its provider is currently available.
#[derive(Clone, Debug)]
pub struct DependencyDiagnostic {
    pub plugin_id: u64,
    pub service: ServiceKey,
    pub available: bool,
}

type Cleanup = Box<dyn FnOnce() -> BoxFuture<Result> + Send>;

/// A listener's id and its `ListenerFn<E>`, erased to share one map.
type ErasedListener = (u64, Arc<dyn Any + Send + Sync>);

trait DynPlugin: Send + Sync + 'static {
    fn name(&self) -> &str;
    fn keys(&self) -> Vec<ServiceKey>;
    fn static_keys(&self) -> Vec<DepKey>;
    /// The `apply` future, or `None` while a dependency is missing.
    fn start(self: Arc<Self>, ctx: Ctx, lookup: deps::Lookup<'_>) -> Option<BoxFuture<Result>>;
}

struct Erased<P> {
    plugin: P,
    static_keys: Vec<DepKey>,
    keys: Vec<ServiceKey>,
}

impl<P: Plugin> Erased<P> {
    fn new(plugin: P) -> Self {
        let static_keys = P::Deps::keys();
        let mut keys: Vec<ServiceKey> = static_keys.iter().copied().map(ServiceKey::from).collect();
        for key in plugin.dependency_keys() {
            if !keys.contains(&key) {
                keys.push(key);
            }
        }
        Self {
            plugin,
            static_keys,
            keys,
        }
    }
}

impl<P: Plugin> DynPlugin for Erased<P> {
    fn name(&self) -> &str {
        self.plugin.name()
    }

    fn static_keys(&self) -> Vec<DepKey> {
        self.static_keys.clone()
    }

    fn keys(&self) -> Vec<ServiceKey> {
        self.keys.clone()
    }

    fn start(self: Arc<Self>, ctx: Ctx, lookup: deps::Lookup<'_>) -> Option<BoxFuture<Result>> {
        let deps = P::Deps::resolve(lookup)?;
        Some(Box::pin(async move { self.plugin.apply(&ctx, deps).await }))
    }
}

use crate::deps;

struct Binding {
    value: Arc<dyn Any + Send + Sync>,
    serial: u64,
}

struct Fiber {
    name: String,
    parent: Option<FiberId>,
    children: Vec<FiberId>,
    plugin: Option<Arc<dyn DynPlugin>>,
    state: State,
    generation: u32,
    /// Bumped on every start and stop; a `Ctx` from an older epoch is stale.
    epoch: u64,
    error: Option<Error>,
    /// The provider serials `apply` received, to detect a replacement.
    resolved: Vec<(ServiceKey, Option<u64>)>,
    cleanups: Vec<Cleanup>,
    watch: watch::Sender<Snapshot>,
}

impl Fiber {
    fn snapshot(&self) -> Snapshot {
        Snapshot {
            state: self.state,
            generation: self.generation,
            error: self.error.clone(),
        }
    }

    fn publish(&self) {
        self.watch.send_replace(self.snapshot());
    }
}

#[derive(Default)]
struct Registry {
    next_id: FiberId,
    next_serial: u64,
    fibers: BTreeMap<FiberId, Fiber>,
    services: HashMap<ServiceKey, Binding>,
    listeners: HashMap<TypeId, Vec<ErasedListener>>,
    shutting_down: bool,
}

impl Registry {
    fn serial_of(&self, key: &ServiceKey) -> Option<u64> {
        self.services.get(key).map(|binding| binding.serial)
    }

    fn resolution(&self, keys: &[ServiceKey]) -> Vec<(ServiceKey, Option<u64>)> {
        keys.iter()
            .map(|key| (key.clone(), self.serial_of(key)))
            .collect()
    }

    fn is_running(&self, fiber: FiberId, epoch: u64) -> bool {
        self.fibers
            .get(&fiber)
            .is_some_and(|f| f.epoch == epoch && matches!(f.state, State::Loading | State::Active))
    }
}

enum Command {
    Wake,
    Dispose(FiberId, oneshot::Sender<()>),
    Shutdown(oneshot::Sender<()>),
}

struct Shared {
    registry: Mutex<Registry>,
    commands: mpsc::UnboundedSender<Command>,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, Registry> {
        self.registry.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn wake(&self) {
        let _ = self.commands.send(Command::Wake);
    }
}

/// A plugin's view of the kernel. What it registers is released when the
/// plugin stops; registrations through a stale context are refused.
#[derive(Clone)]
pub struct Ctx {
    shared: Arc<Shared>,
    fiber: FiberId,
    epoch: u64,
    injected: Option<Arc<HashMap<ServiceKey, Arc<dyn Any + Send + Sync>>>>,
}

impl Ctx {
    /// Starts a kernel and returns its root context. Needs a tokio runtime.
    pub fn root() -> Result<Ctx> {
        let (commands, receiver) = mpsc::unbounded_channel();
        let mut registry = Registry {
            next_id: ROOT + 1,
            ..Registry::default()
        };
        let root = Fiber {
            name: "root".into(),
            parent: None,
            children: Vec::new(),
            plugin: None,
            state: State::Active,
            generation: 0,
            epoch: 0,
            error: None,
            resolved: Vec::new(),
            cleanups: Vec::new(),
            watch: watch::channel(Snapshot {
                state: State::Active,
                generation: 0,
                error: None,
            })
            .0,
        };
        registry.fibers.insert(ROOT, root);
        let shared = Arc::new(Shared {
            registry: Mutex::new(registry),
            commands,
        });
        tokio::spawn(
            Driver {
                shared: Arc::clone(&shared),
            }
            .run(receiver),
        );
        Ok(Ctx {
            shared,
            fiber: ROOT,
            epoch: 0,
            injected: None,
        })
    }

    /// Mounts `plugin` under this context; it unloads with it.
    pub fn plugin(&self, plugin: impl Plugin) -> PluginHandle {
        let plugin: Arc<dyn DynPlugin> = Arc::new(Erased::new(plugin));
        let (sender, receiver) = watch::channel(Snapshot {
            state: State::Pending,
            generation: 0,
            error: None,
        });
        let mut registry = self.shared.lock();
        let id = registry.next_id;
        registry.next_id += 1;
        let accepted = !registry.shutting_down && registry.is_running(self.fiber, self.epoch);
        if accepted {
            registry.fibers.insert(
                id,
                Fiber {
                    name: plugin.name().to_string(),
                    parent: Some(self.fiber),
                    children: Vec::new(),
                    plugin: Some(plugin),
                    state: State::Pending,
                    generation: 0,
                    epoch: 0,
                    error: None,
                    resolved: Vec::new(),
                    cleanups: Vec::new(),
                    watch: sender,
                },
            );
            if let Some(parent) = registry.fibers.get_mut(&self.fiber) {
                parent.children.push(id);
            }
        } else {
            sender.send_replace(Snapshot {
                state: State::Disposed,
                generation: 0,
                error: Some(Error::Inactive),
            });
        }
        drop(registry);
        self.shared.wake();
        PluginHandle {
            id,
            shared: Arc::clone(&self.shared),
            state: receiver,
        }
    }

    /// Provides `value` as the service `T` until this plugin stops.
    pub fn provide<T: Send + Sync + 'static>(&self, value: T) -> Result {
        self.provide_as::<T>(Arc::new(value))
    }

    /// Provides `value` under the type `T`, typically a trait object.
    pub fn provide_as<T: ?Sized + Send + Sync + 'static>(&self, value: Arc<T>) -> Result {
        self.provide_at(ServiceKey::of::<T>(), value)
    }

    /// Provides one named service until this plugin stops.
    pub fn provide_named<T: Send + Sync + 'static>(
        &self,
        name: impl Into<String>,
        value: T,
    ) -> Result {
        self.provide_named_as::<T>(name, Arc::new(value))
    }

    pub fn provide_named_as<T: ?Sized + Send + Sync + 'static>(
        &self,
        name: impl Into<String>,
        value: Arc<T>,
    ) -> Result {
        self.provide_at(ServiceKey::named::<T>(name), value)
    }

    fn provide_at<T: ?Sized + Send + Sync + 'static>(
        &self,
        key: ServiceKey,
        value: Arc<T>,
    ) -> Result {
        let mut registry = self.shared.lock();
        if !registry.is_running(self.fiber, self.epoch) {
            return Err(Error::Inactive);
        }
        if registry.services.contains_key(&key) {
            if key.name().is_some() {
                return Err(crate::NamedServiceAlreadyProvided { service: key }.into());
            }
            return Err(Error::AlreadyProvided {
                service: DepKey::of::<T>().name,
            });
        }
        registry.next_serial += 1;
        let serial = registry.next_serial;
        registry.services.insert(
            key.clone(),
            Binding {
                value: Arc::new(value),
                serial,
            },
        );
        let shared = Arc::clone(&self.shared);
        push_cleanup(&mut registry, self.fiber, move || {
            Box::pin(async move {
                let mut registry = shared.lock();
                if registry
                    .services
                    .get(&key)
                    .is_some_and(|b| b.serial == serial)
                {
                    registry.services.remove(&key);
                }
                Ok(())
            })
        });
        drop(registry);
        self.shared.wake();
        Ok(())
    }

    /// The current provider of `T`, if any.
    pub fn get<T: ?Sized + Send + Sync + 'static>(&self) -> Option<Arc<T>> {
        let registry = self.shared.lock();
        let binding = registry.services.get(&ServiceKey::of::<T>())?;
        binding.value.downcast_ref::<Arc<T>>().cloned()
    }

    /// The binding this instance declared and received when it started.
    /// The root context can inspect current bindings without injecting them.
    pub fn get_named<T: ?Sized + Send + Sync + 'static>(
        &self,
        name: impl Into<String>,
    ) -> Option<Arc<T>> {
        let key = ServiceKey::named::<T>(name);
        if let Some(injected) = &self.injected {
            return injected.get(&key)?.downcast_ref::<Arc<T>>().cloned();
        }
        self.shared
            .lock()
            .services
            .get(&key)?
            .value
            .downcast_ref::<Arc<T>>()
            .cloned()
    }

    /// Runs `acquire` now and registers the cleanup it returns, which runs
    /// when this plugin stops (newest first).
    pub fn effect<F, Fut>(&self, acquire: F) -> Result
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result> + Send + 'static,
    {
        if !self.shared.lock().is_running(self.fiber, self.epoch) {
            return Err(Error::Inactive);
        }
        let cleanup = acquire();
        let mut registry = self.shared.lock();
        if !registry.is_running(self.fiber, self.epoch) {
            drop(registry);
            tokio::spawn(cleanup);
            return Err(Error::Inactive);
        }
        push_cleanup(&mut registry, self.fiber, move || Box::pin(cleanup));
        Ok(())
    }

    /// A gate whose passes this plugin's stop waits for.
    pub fn gate(&self) -> Result<Arc<Gate>> {
        let gate = Arc::new(Gate::default());
        let closing = Arc::clone(&gate);
        self.effect(move || async move {
            closing.close_and_drain().await;
            Ok(())
        })?;
        Ok(gate)
    }

    /// Listens to `E` until this plugin stops. Listeners run in registration order.
    pub fn on<E: Event>(&self, listener: impl Listener<E>) -> Result {
        let listener: ListenerFn<E> = Arc::new(move |event, call| Box::pin(listener(event, call)));
        let key = TypeId::of::<E>();
        let mut registry = self.shared.lock();
        if !registry.is_running(self.fiber, self.epoch) {
            return Err(Error::Inactive);
        }
        registry.next_serial += 1;
        let id = registry.next_serial;
        registry
            .listeners
            .entry(key)
            .or_default()
            .push((id, Arc::new(listener)));
        let shared = Arc::clone(&self.shared);
        push_cleanup(&mut registry, self.fiber, move || {
            Box::pin(async move {
                if let Some(list) = shared.lock().listeners.get_mut(&key) {
                    list.retain(|(listener, _)| *listener != id);
                }
                Ok(())
            })
        });
        Ok(())
    }

    pub fn emit<E: Event>(&self, event: E) {
        events::emit(event, self.listeners::<E>());
    }

    pub async fn parallel<E: Event>(&self, event: E) -> Result {
        events::parallel(event, self.listeners::<E>()).await
    }

    pub async fn serial<E: Event>(&self, event: E) -> Result<Option<E::Value>> {
        events::serial(event, self.listeners::<E>()).await
    }

    pub fn waterfall<E: Event>(&self, event: E) -> Waterfall<E> {
        Waterfall::new(event, self.listeners::<E>())
    }

    fn listeners<E: Event>(&self) -> Vec<ListenerFn<E>> {
        let registry = self.shared.lock();
        registry
            .listeners
            .get(&TypeId::of::<E>())
            .map(|list| {
                list.iter()
                    .filter_map(|(_, any)| any.downcast_ref::<ListenerFn<E>>().cloned())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Every mounted plugin, in mount order.
    pub fn diagnostics(&self) -> Vec<PluginInfo> {
        let registry = self.shared.lock();
        registry
            .fibers
            .iter()
            .map(|(id, fiber)| PluginInfo {
                id: *id,
                name: fiber.name.clone(),
                parent: fiber.parent,
                state: fiber.state,
                generation: fiber.generation,
                deps: fiber
                    .plugin
                    .as_ref()
                    .map(|plugin| plugin.static_keys())
                    .unwrap_or_default()
                    .into_iter()
                    .map(|key| {
                        (
                            key.name,
                            registry.services.contains_key(&ServiceKey::from(key)),
                        )
                    })
                    .collect(),
                error: fiber.error.clone(),
            })
            .collect()
    }

    /// All required keys, including instance-selected names. The existing
    /// `PluginInfo::deps` remains the static dependency view.
    pub fn dependency_diagnostics(&self) -> Vec<DependencyDiagnostic> {
        let registry = self.shared.lock();
        let mut diagnostics = Vec::new();
        for (id, fiber) in &registry.fibers {
            let Some(plugin) = &fiber.plugin else {
                continue;
            };
            for service in plugin.keys() {
                diagnostics.push(DependencyDiagnostic {
                    plugin_id: *id,
                    available: registry.services.contains_key(&service),
                    service,
                });
            }
        }
        diagnostics
    }

    /// Disposes every plugin, newest first, and refuses new ones.
    pub async fn shutdown(&self) -> Result {
        let (done, wait) = oneshot::channel();
        self.shared
            .commands
            .send(Command::Shutdown(done))
            .map_err(|_| Error::ShuttingDown)?;
        wait.await.map_err(|_| Error::ShuttingDown)
    }
}

fn push_cleanup(
    registry: &mut Registry,
    fiber: FiberId,
    cleanup: impl FnOnce() -> BoxFuture<Result> + Send + 'static,
) {
    if let Some(fiber) = registry.fibers.get_mut(&fiber) {
        fiber.cleanups.push(Box::new(cleanup));
    }
}

/// A mounted plugin. `(&handle).await` waits until it is active.
pub struct PluginHandle {
    id: FiberId,
    shared: Arc<Shared>,
    state: watch::Receiver<Snapshot>,
}

impl PluginHandle {
    pub fn state(&self) -> Snapshot {
        self.state.borrow().clone()
    }

    /// Stops the plugin and everything mounted under it, then forgets it.
    pub async fn dispose(&self) -> Result {
        let (done, wait) = oneshot::channel();
        self.shared
            .commands
            .send(Command::Dispose(self.id, done))
            .map_err(|_| Error::ShuttingDown)?;
        wait.await.map_err(|_| Error::ShuttingDown)
    }
}

impl IntoFuture for &PluginHandle {
    type Output = Result;
    type IntoFuture = BoxFuture<Result>;

    fn into_future(self) -> Self::IntoFuture {
        let mut state = self.state.clone();
        Box::pin(async move {
            loop {
                let snapshot = state.borrow_and_update().clone();
                match snapshot.state {
                    State::Active => return Ok(()),
                    State::Failed => return Err(snapshot.error.unwrap_or(Error::Disposed)),
                    State::Disposed => return Err(snapshot.error.unwrap_or(Error::Disposed)),
                    _ => {}
                }
                if state.changed().await.is_err() {
                    return Err(Error::Disposed);
                }
            }
        })
    }
}

struct Driver {
    shared: Arc<Shared>,
}

impl Driver {
    async fn run(self, mut commands: mpsc::UnboundedReceiver<Command>) {
        while let Some(command) = commands.recv().await {
            match command {
                Command::Wake => {}
                Command::Dispose(id, done) => {
                    self.dispose(id).await;
                    let _ = done.send(());
                }
                Command::Shutdown(done) => {
                    self.shared.lock().shutting_down = true;
                    let children = self.children(ROOT);
                    for child in children.into_iter().rev() {
                        self.dispose(child).await;
                    }
                    let _ = done.send(());
                    continue;
                }
            }
            self.reconcile().await;
        }
    }

    fn children(&self, id: FiberId) -> Vec<FiberId> {
        self.shared
            .lock()
            .fibers
            .get(&id)
            .map(|fiber| fiber.children.clone())
            .unwrap_or_default()
    }

    /// Stops what lost or changed a dependency, then starts what can start,
    /// one plugin at a time, until nothing changes.
    async fn reconcile(&self) {
        loop {
            if let Some(id) = self.next_to_stop() {
                self.stop(id, State::Pending).await;
                continue;
            }
            self.retry_failed();
            let Some(id) = self.next_to_start() else {
                return;
            };
            self.start(id).await;
        }
    }

    fn next_to_stop(&self) -> Option<FiberId> {
        let registry = self.shared.lock();
        registry.fibers.iter().find_map(|(id, fiber)| {
            let plugin = fiber.plugin.as_ref()?;
            let changed = fiber.state == State::Active
                && registry.resolution(&plugin.keys()) != fiber.resolved;
            changed.then_some(*id)
        })
    }

    fn retry_failed(&self) {
        let mut registry = self.shared.lock();
        let retry: Vec<FiberId> = registry
            .fibers
            .iter()
            .filter(|(_, fiber)| fiber.state == State::Failed)
            .filter_map(|(id, fiber)| {
                let plugin = fiber.plugin.as_ref()?;
                (registry.resolution(&plugin.keys()) != fiber.resolved).then_some(*id)
            })
            .collect();
        for id in retry {
            if let Some(fiber) = registry.fibers.get_mut(&id) {
                fiber.state = State::Pending;
                fiber.publish();
            }
        }
    }

    fn next_to_start(&self) -> Option<FiberId> {
        let registry = self.shared.lock();
        if registry.shutting_down {
            return None;
        }
        registry.fibers.iter().find_map(|(id, fiber)| {
            let plugin = fiber.plugin.as_ref()?;
            let parent_active = fiber
                .parent
                .and_then(|parent| registry.fibers.get(&parent))
                .is_some_and(|parent| parent.state == State::Active);
            let ready = plugin
                .keys()
                .iter()
                .all(|key| registry.services.contains_key(key));
            (fiber.state == State::Pending && parent_active && ready).then_some(*id)
        })
    }

    async fn start(&self, id: FiberId) {
        let (run, name) = {
            let mut registry = self.shared.lock();
            let Some(fiber) = registry.fibers.get(&id) else {
                return;
            };
            let Some(plugin) = fiber.plugin.clone() else {
                return;
            };
            let resolved = registry.resolution(&plugin.keys());
            let epoch = fiber.epoch + 1;
            let ctx = Ctx {
                shared: Arc::clone(&self.shared),
                fiber: id,
                epoch,
                injected: Some(Arc::new(
                    plugin
                        .keys()
                        .into_iter()
                        .filter_map(|key| {
                            registry
                                .services
                                .get(&key)
                                .map(|binding| (key, Arc::clone(&binding.value)))
                        })
                        .collect(),
                )),
            };
            let lookup = |key: TypeId| {
                registry
                    .services
                    .get(&ServiceKey::from_type_id(key))
                    .map(|b| Arc::clone(&b.value))
            };
            let Some(run) = Arc::clone(&plugin).start(ctx, &lookup) else {
                return;
            };
            let Some(fiber) = registry.fibers.get_mut(&id) else {
                return;
            };
            fiber.epoch = epoch;
            fiber.generation += 1;
            fiber.state = State::Loading;
            fiber.error = None;
            fiber.resolved = resolved;
            fiber.publish();
            (run, fiber.name.clone())
        };

        let outcome = match tokio::spawn(run).await {
            Ok(result) => result,
            Err(join) => Err(Error::Panicked {
                plugin: name.clone(),
                message: panic_message(join),
            }),
        };

        match outcome {
            Ok(()) => self.set_state(id, State::Active, None),
            Err(error) => {
                self.unwind(id).await;
                self.set_state(id, State::Failed, Some(error));
            }
        }
    }

    /// Releases a fiber's children and cleanups, newest first, and bumps its
    /// epoch so its old context can register nothing more.
    async fn unwind(&self, id: FiberId) {
        let children = {
            let mut registry = self.shared.lock();
            let Some(fiber) = registry.fibers.get_mut(&id) else {
                return;
            };
            fiber.epoch += 1;
            fiber.state = State::Unloading;
            fiber.publish();
            std::mem::take(&mut fiber.children)
        };
        for child in children.into_iter().rev() {
            self.dispose(child).await;
        }
        let cleanups = {
            let mut registry = self.shared.lock();
            registry
                .fibers
                .get_mut(&id)
                .map(|fiber| std::mem::take(&mut fiber.cleanups))
                .unwrap_or_default()
        };
        for cleanup in cleanups.into_iter().rev() {
            match tokio::spawn(cleanup()).await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => eprintln!("a cleanup failed: {error}"),
                Err(join) => eprintln!("a cleanup panicked: {}", panic_message(join)),
            }
        }
    }

    async fn stop(&self, id: FiberId, to: State) {
        self.unwind(id).await;
        self.set_state(id, to, None);
    }

    fn dispose(&self, id: FiberId) -> BoxFuture<()> {
        let shared = Arc::clone(&self.shared);
        Box::pin(async move {
            let driver = Driver { shared };
            if id == ROOT || !driver.shared.lock().fibers.contains_key(&id) {
                return;
            }
            driver.stop(id, State::Disposed).await;
            let mut registry = driver.shared.lock();
            let parent = registry.fibers.remove(&id).and_then(|fiber| fiber.parent);
            if let Some(parent) = parent.and_then(|p| registry.fibers.get_mut(&p)) {
                parent.children.retain(|child| *child != id);
            }
        })
    }

    fn set_state(&self, id: FiberId, state: State, error: Option<Error>) {
        let mut registry = self.shared.lock();
        if let Some(fiber) = registry.fibers.get_mut(&id) {
            fiber.state = state;
            fiber.error = error;
            fiber.publish();
        }
    }
}

fn panic_message(join: tokio::task::JoinError) -> String {
    match join.try_into_panic() {
        Ok(payload) => payload
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| payload.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "a panic".into()),
        Err(join) => join.to_string(),
    }
}
