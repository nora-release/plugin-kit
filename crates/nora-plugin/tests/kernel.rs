//! Kernel guarantees: panics, leases, stale contexts, duplicate providers,
//! retries and shutdown order.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use nora_plugin::{Ctx, Error, Gate, Plugin, Result, State};

struct Config(&'static str);

struct Provides(&'static str);

impl Plugin for Provides {
    type Deps = ();

    async fn apply(&self, ctx: &Ctx, (): ()) -> Result {
        ctx.provide(Config(self.0))
    }
}

struct Panics;

impl Plugin for Panics {
    type Deps = ();

    async fn apply(&self, _: &Ctx, (): ()) -> Result {
        panic!("boom")
    }
}

#[tokio::test]
async fn k6_a_panic_in_apply_fails_only_that_plugin() {
    let root = Ctx::root().unwrap();
    let panics = root.plugin(Panics);
    let error = (&panics).await.unwrap_err();
    assert!(matches!(error, Error::Panicked { .. }), "{error}");
    assert_eq!(panics.state().state, State::Failed);

    let fine = root.plugin(Provides("ok"));
    (&fine).await.unwrap();
    assert_eq!(root.get::<Config>().unwrap().0, "ok");
    root.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_second_provider_is_refused() {
    let root = Ctx::root().unwrap();
    (&root.plugin(Provides("first"))).await.unwrap();
    let second = root.plugin(Provides("second"));
    let error = (&second).await.unwrap_err();
    assert!(matches!(error, Error::AlreadyProvided { .. }), "{error}");
    assert_eq!(root.get::<Config>().unwrap().0, "first");
    root.shutdown().await.unwrap();
}

/// Keeps its context after apply, as a careless plugin might.
struct Leaky(Arc<Mutex<Option<Ctx>>>);

impl Plugin for Leaky {
    type Deps = ();

    async fn apply(&self, ctx: &Ctx, (): ()) -> Result {
        *self.0.lock().unwrap() = Some(ctx.clone());
        Ok(())
    }
}

#[tokio::test]
async fn a_stopped_plugins_context_registers_nothing() {
    let root = Ctx::root().unwrap();
    let kept = Arc::new(Mutex::new(None));
    let leaky = root.plugin(Leaky(Arc::clone(&kept)));
    (&leaky).await.unwrap();
    leaky.dispose().await.unwrap();

    let stale = kept.lock().unwrap().take().unwrap();
    assert!(matches!(
        stale.provide(Config("late")),
        Err(Error::Inactive)
    ));
    assert!(
        root.get::<Config>().is_none(),
        "a stale context provided a service"
    );
    root.shutdown().await.unwrap();
}

/// Hands out work through a gate, as a tool or a provider would.
struct Worker;

struct WorkGate(Arc<Gate>);

impl Plugin for Worker {
    type Deps = ();

    async fn apply(&self, ctx: &Ctx, (): ()) -> Result {
        let gate = ctx.gate()?;
        ctx.provide(WorkGate(gate))
    }
}

#[tokio::test]
async fn stopping_waits_for_work_in_flight() {
    let root = Ctx::root().unwrap();
    let worker = root.plugin(Worker);
    (&worker).await.unwrap();
    let gate = Arc::clone(&root.get::<WorkGate>().unwrap().0);
    let pass = gate.enter().expect("the gate is open");

    let dispose = tokio::spawn(async move { worker.dispose().await });
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(!dispose.is_finished(), "stopped with work in flight");
    assert!(gate.enter().is_none(), "new work admitted while stopping");

    drop(pass);
    tokio::time::timeout(Duration::from_secs(1), dispose)
        .await
        .expect("the stop never finished")
        .unwrap()
        .unwrap();
    assert_eq!(gate.in_flight(), 0);
    root.shutdown().await.unwrap();
}

/// Fails until it sees the configuration it wants.
struct Picky(Arc<AtomicUsize>);

impl Plugin for Picky {
    type Deps = Arc<Config>;

    async fn apply(&self, _: &Ctx, config: Arc<Config>) -> Result {
        self.0.fetch_add(1, Ordering::SeqCst);
        if config.0 == "good" {
            Ok(())
        } else {
            Err(std::io::Error::other(format!("cannot use {}", config.0)).into())
        }
    }
}

#[tokio::test]
async fn a_failed_plugin_retries_when_its_dependency_changes() {
    let root = Ctx::root().unwrap();
    let tries = Arc::new(AtomicUsize::new(0));
    let bad = root.plugin(Provides("bad"));
    (&bad).await.unwrap();
    let picky = root.plugin(Picky(Arc::clone(&tries)));
    assert!((&picky).await.is_err());

    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(
        tries.load(Ordering::SeqCst),
        1,
        "retried with nothing changed"
    );

    bad.dispose().await.unwrap();
    (&root.plugin(Provides("good"))).await.unwrap();
    (&picky).await.unwrap();
    assert_eq!(tries.load(Ordering::SeqCst), 2);
    root.shutdown().await.unwrap();
}

#[tokio::test]
async fn shutdown_unloads_newest_first_and_refuses_new_plugins() {
    let order = Arc::new(Mutex::new(Vec::new()));

    struct Logs(&'static str, Arc<Mutex<Vec<&'static str>>>);

    impl Plugin for Logs {
        type Deps = ();

        async fn apply(&self, ctx: &Ctx, (): ()) -> Result {
            let (name, order) = (self.0, Arc::clone(&self.1));
            ctx.effect(move || async move {
                order.lock().unwrap().push(name);
                Ok(())
            })
        }
    }

    let root = Ctx::root().unwrap();
    for name in ["first", "second", "third"] {
        (&root.plugin(Logs(name, Arc::clone(&order))))
            .await
            .unwrap();
    }
    root.shutdown().await.unwrap();
    assert_eq!(*order.lock().unwrap(), ["third", "second", "first"]);

    let late = root.plugin(Provides("late"));
    assert!((&late).await.is_err());
}
