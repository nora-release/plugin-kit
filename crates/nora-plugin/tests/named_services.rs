use nora_plugin::{
    Ctx, DepKey, Error, NamedServiceAlreadyProvided, Plugin, Result, ServiceKey, State,
};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

struct Answer(u32);

struct Provider {
    service: String,
    value: u32,
}

impl Plugin for Provider {
    type Deps = ();

    async fn apply(&self, ctx: &Ctx, (): ()) -> Result {
        ctx.provide_named(self.service.clone(), Answer(self.value))
    }
}

#[derive(Clone, Default)]
struct Observations {
    started: Arc<AtomicUsize>,
    stopped: Arc<AtomicUsize>,
    values: Arc<Mutex<Vec<u32>>>,
}

struct Consumer {
    service: String,
    observations: Observations,
}

impl Plugin for Consumer {
    type Deps = ();

    fn dependency_keys(&self) -> Vec<ServiceKey> {
        vec![ServiceKey::named::<Answer>(self.service.clone())]
    }

    async fn apply(&self, ctx: &Ctx, (): ()) -> Result {
        let answer = ctx
            .get_named::<Answer>(self.service.clone())
            .expect("the driver resolved this name");
        self.observations.started.fetch_add(1, Ordering::SeqCst);
        self.observations
            .values
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(answer.0);
        let stopped = Arc::clone(&self.observations.stopped);
        ctx.effect(move || async move {
            stopped.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
    }
}

async fn eventually(mut condition: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(3), async {
        while !condition() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn a_required_name_waits_for_its_provider_and_appears_in_dependency_diagnostics() {
    let root = Ctx::root().unwrap();
    let observations = Observations::default();
    let consumer = root.plugin(Consumer {
        service: "alpha".into(),
        observations: observations.clone(),
    });
    tokio::task::yield_now().await;
    assert_eq!(consumer.state().state, State::Pending);
    assert_eq!(observations.started.load(Ordering::SeqCst), 0);
    let diagnostics = root.dependency_diagnostics();
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].service.name(), Some("alpha"));
    assert!(!diagnostics[0].available);

    let provider = root.plugin(Provider {
        service: "alpha".into(),
        value: 7,
    });
    (&provider).await.unwrap();
    (&consumer).await.unwrap();
    assert_eq!(
        *observations
            .values
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
        vec![7]
    );
    assert!(root.dependency_diagnostics()[0].available);
    assert!(root
        .diagnostics()
        .iter()
        .all(|plugin| plugin.deps.is_empty()));
    root.shutdown().await.unwrap();
}

#[tokio::test]
async fn replacing_one_name_restarts_only_the_consumers_of_that_binding() {
    let root = Ctx::root().unwrap();
    let a = Observations::default();
    let b = Observations::default();
    let consumer_a = root.plugin(Consumer {
        service: "a".into(),
        observations: a.clone(),
    });
    let consumer_b = root.plugin(Consumer {
        service: "b".into(),
        observations: b.clone(),
    });
    let provider_a = root.plugin(Provider {
        service: "a".into(),
        value: 1,
    });
    let provider_b = root.plugin(Provider {
        service: "b".into(),
        value: 10,
    });
    (&provider_a).await.unwrap();
    (&provider_b).await.unwrap();
    (&consumer_a).await.unwrap();
    (&consumer_b).await.unwrap();

    provider_a.dispose().await.unwrap();
    eventually(|| consumer_a.state().state == State::Pending).await;
    assert_eq!(a.stopped.load(Ordering::SeqCst), 1);
    let replacement = root.plugin(Provider {
        service: "a".into(),
        value: 2,
    });
    (&replacement).await.unwrap();
    (&consumer_a).await.unwrap();
    assert_eq!(consumer_a.state().generation, 2);
    assert_eq!(consumer_b.state().generation, 1);
    assert_eq!(b.stopped.load(Ordering::SeqCst), 0);
    assert_eq!(
        *a.values
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
        vec![1, 2]
    );
    assert_eq!(
        *b.values
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
        vec![10]
    );
    root.shutdown().await.unwrap();
    assert_eq!(a.stopped.load(Ordering::SeqCst), 2);
    assert_eq!(b.stopped.load(Ordering::SeqCst), 1);
}

struct Static;
struct StaticProvider;

impl Plugin for StaticProvider {
    type Deps = ();
    async fn apply(&self, ctx: &Ctx, (): ()) -> Result {
        ctx.provide(Static)
    }
}

struct MixedConsumer(Arc<AtomicUsize>);

impl Plugin for MixedConsumer {
    type Deps = Arc<Static>;

    fn dependency_keys(&self) -> Vec<ServiceKey> {
        vec![ServiceKey::named::<Answer>("a")]
    }

    async fn apply(&self, ctx: &Ctx, _: Self::Deps) -> Result {
        assert_eq!(ctx.get_named::<Answer>("a").unwrap().0, 1);
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[tokio::test]
async fn overriding_instance_keys_cannot_remove_required_static_dependencies() {
    let root = Ctx::root().unwrap();
    let named = root.plugin(Provider {
        service: "a".into(),
        value: 1,
    });
    (&named).await.unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let consumer = root.plugin(MixedConsumer(Arc::clone(&calls)));
    tokio::task::yield_now().await;
    assert_eq!(consumer.state().state, State::Pending);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let provider = root.plugin(StaticProvider);
    (&provider).await.unwrap();
    (&consumer).await.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    root.shutdown().await.unwrap();
}

struct FailingProvider;

impl Plugin for FailingProvider {
    type Deps = ();
    async fn apply(&self, ctx: &Ctx, (): ()) -> Result {
        ctx.provide_named("temporary", Answer(1))?;
        Err(std::io::Error::other("startup rejected").into())
    }
}

#[tokio::test]
async fn named_services_from_a_failed_apply_are_rolled_back() {
    let root = Ctx::root().unwrap();
    let observations = Observations::default();
    let consumer = root.plugin(Consumer {
        service: "temporary".into(),
        observations: observations.clone(),
    });
    let failed = root.plugin(FailingProvider);
    assert!((&failed).await.is_err());
    assert_eq!(failed.state().state, State::Failed);
    assert!(root.get_named::<Answer>("temporary").is_none());
    assert_eq!(consumer.state().state, State::Pending);
    assert_eq!(observations.started.load(Ordering::SeqCst), 0);
    root.shutdown().await.unwrap();
}

struct Scope;

impl Plugin for Scope {
    type Deps = ();
    async fn apply(&self, ctx: &Ctx, (): ()) -> Result {
        ctx.plugin(Provider {
            service: "inside".into(),
            value: 3,
        });
        Ok(())
    }
}

#[tokio::test]
async fn ending_a_scope_removes_named_services_and_stops_external_consumers() {
    let root = Ctx::root().unwrap();
    let observations = Observations::default();
    let consumer = root.plugin(Consumer {
        service: "inside".into(),
        observations: observations.clone(),
    });
    let scope = root.plugin(Scope);
    (&scope).await.unwrap();
    (&consumer).await.unwrap();
    scope.dispose().await.unwrap();
    eventually(|| consumer.state().state == State::Pending).await;
    assert!(root.get_named::<Answer>("inside").is_none());
    assert_eq!(observations.stopped.load(Ordering::SeqCst), 1);
    root.shutdown().await.unwrap();
}

struct CaptureProvider(Arc<Mutex<Option<Ctx>>>);

impl Plugin for CaptureProvider {
    type Deps = ();
    async fn apply(&self, ctx: &Ctx, (): ()) -> Result {
        ctx.provide_named("captured", Answer(4))?;
        *self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(ctx.clone());
        Ok(())
    }
}

#[tokio::test]
async fn a_stale_context_cannot_republish_a_named_service() {
    let root = Ctx::root().unwrap();
    let saved = Arc::new(Mutex::new(None));
    let provider = root.plugin(CaptureProvider(Arc::clone(&saved)));
    (&provider).await.unwrap();
    provider.dispose().await.unwrap();
    let saved = saved
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take()
        .unwrap();
    assert!(matches!(
        saved.provide_named("captured", Answer(9)),
        Err(Error::Inactive)
    ));
    assert!(root.get_named::<Answer>("captured").is_none());
    root.shutdown().await.unwrap();
}

#[tokio::test]
async fn names_types_and_unnamed_services_have_distinct_identities() {
    let root = Ctx::root().unwrap();
    root.provide(Answer(3)).unwrap();
    root.provide_named("left", Answer(1)).unwrap();
    root.provide_named("right", Answer(2)).unwrap();
    root.provide_named("left", 8_u32).unwrap();
    assert_eq!(root.get::<Answer>().unwrap().0, 3);
    assert_eq!(root.get_named::<Answer>("left").unwrap().0, 1);
    assert_eq!(root.get_named::<Answer>("right").unwrap().0, 2);
    assert_eq!(*root.get_named::<u32>("left").unwrap(), 8);
    match root.provide_named("left", Answer(99)).unwrap_err() {
        Error::Plugin(error) => assert!(error
            .downcast_ref::<NamedServiceAlreadyProvided>()
            .is_some()),
        other => panic!("unexpected error: {other}"),
    }
    root.shutdown().await.unwrap();
}

struct RestrictedConsumer;

impl Plugin for RestrictedConsumer {
    type Deps = ();
    fn dependency_keys(&self) -> Vec<ServiceKey> {
        vec![ServiceKey::named::<Answer>("declared")]
    }
    async fn apply(&self, ctx: &Ctx, (): ()) -> Result {
        assert_eq!(ctx.get_named::<Answer>("declared").unwrap().0, 1);
        assert!(ctx.get_named::<Answer>("undeclared").is_none());
        Ok(())
    }
}

#[tokio::test]
async fn an_instance_receives_only_the_named_bindings_it_declared() {
    let root = Ctx::root().unwrap();
    root.provide_named("declared", Answer(1)).unwrap();
    root.provide_named("undeclared", Answer(2)).unwrap();
    let consumer = root.plugin(RestrictedConsumer);
    (&consumer).await.unwrap();
    assert!(root.get_named::<Answer>("undeclared").is_some());
    root.shutdown().await.unwrap();
}

#[tokio::test]
async fn repeatedly_replacing_the_same_name_leaves_no_stale_binding_or_cleanup() {
    let root = Ctx::root().unwrap();
    let observations = Observations::default();
    let consumer = root.plugin(Consumer {
        service: "replace".into(),
        observations: observations.clone(),
    });
    for value in 0..50 {
        let provider = root.plugin(Provider {
            service: "replace".into(),
            value,
        });
        (&provider).await.unwrap();
        (&consumer).await.unwrap();
        provider.dispose().await.unwrap();
        eventually(|| consumer.state().state == State::Pending).await;
        assert!(root.get_named::<Answer>("replace").is_none());
    }
    assert_eq!(observations.started.load(Ordering::SeqCst), 50);
    assert_eq!(observations.stopped.load(Ordering::SeqCst), 50);
    root.shutdown().await.unwrap();
}

struct PickyConsumer(Arc<AtomicUsize>);

impl Plugin for PickyConsumer {
    type Deps = ();
    fn dependency_keys(&self) -> Vec<ServiceKey> {
        vec![ServiceKey::named::<Answer>("retry")]
    }
    async fn apply(&self, ctx: &Ctx, (): ()) -> Result {
        self.0.fetch_add(1, Ordering::SeqCst);
        if ctx.get_named::<Answer>("retry").unwrap().0 < 2 {
            return Err(std::io::Error::other("try a new provider").into());
        }
        Ok(())
    }
}

#[tokio::test]
async fn failed_named_consumers_retry_only_when_their_own_binding_changes() {
    let root = Ctx::root().unwrap();
    let provider = root.plugin(Provider {
        service: "retry".into(),
        value: 1,
    });
    (&provider).await.unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let consumer = root.plugin(PickyConsumer(Arc::clone(&calls)));
    assert!((&consumer).await.is_err());
    let unrelated = root.plugin(Provider {
        service: "other".into(),
        value: 99,
    });
    (&unrelated).await.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    provider.dispose().await.unwrap();
    let replacement = root.plugin(Provider {
        service: "retry".into(),
        value: 2,
    });
    (&replacement).await.unwrap();
    (&consumer).await.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    root.shutdown().await.unwrap();
}

#[test]
fn custom_static_dependency_display_names_do_not_change_service_identity() {
    let original = DepKey::of::<Answer>();
    let custom = DepKey {
        id: original.id,
        name: "custom label",
    };
    let map = HashMap::from([(ServiceKey::from(original), 1)]);
    assert_eq!(map.get(&ServiceKey::from(custom)), Some(&1));
}

trait AnswerService: Send + Sync {
    fn answer(&self) -> u32;
}

impl AnswerService for Answer {
    fn answer(&self) -> u32 {
        self.0
    }
}

struct TraitConsumer;

impl Plugin for TraitConsumer {
    type Deps = ();
    fn dependency_keys(&self) -> Vec<ServiceKey> {
        vec![ServiceKey::named::<dyn AnswerService>("rpc")]
    }
    async fn apply(&self, ctx: &Ctx, (): ()) -> Result {
        assert_eq!(
            ctx.get_named::<dyn AnswerService>("rpc").unwrap().answer(),
            5
        );
        Ok(())
    }
}

#[tokio::test]
async fn named_services_can_use_trait_objects_without_protocol_types_in_the_kernel() {
    let root = Ctx::root().unwrap();
    root.provide_named_as::<dyn AnswerService>("rpc", Arc::new(Answer(5)))
        .unwrap();
    let consumer = root.plugin(TraitConsumer);
    (&consumer).await.unwrap();
    root.shutdown().await.unwrap();
}

struct ForwardingProvider;

impl Plugin for ForwardingProvider {
    type Deps = ();
    fn dependency_keys(&self) -> Vec<ServiceKey> {
        vec![ServiceKey::named::<Answer>("input")]
    }
    async fn apply(&self, ctx: &Ctx, (): ()) -> Result {
        ctx.provide_named(
            "output",
            Answer(ctx.get_named::<Answer>("input").unwrap().0 + 1),
        )
    }
}

#[tokio::test]
async fn a_named_binding_replacement_restarts_its_transitive_consumers() {
    let root = Ctx::root().unwrap();
    let observations = Observations::default();
    let leaf = root.plugin(Consumer {
        service: "output".into(),
        observations: observations.clone(),
    });
    let middle = root.plugin(ForwardingProvider);
    let first = root.plugin(Provider {
        service: "input".into(),
        value: 1,
    });
    (&first).await.unwrap();
    (&middle).await.unwrap();
    (&leaf).await.unwrap();
    first.dispose().await.unwrap();
    eventually(|| middle.state().state == State::Pending && leaf.state().state == State::Pending)
        .await;
    let second = root.plugin(Provider {
        service: "input".into(),
        value: 2,
    });
    (&second).await.unwrap();
    (&middle).await.unwrap();
    (&leaf).await.unwrap();
    assert_eq!(middle.state().generation, 2);
    assert_eq!(leaf.state().generation, 2);
    assert_eq!(
        *observations
            .values
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
        vec![2, 3]
    );
    root.shutdown().await.unwrap();
}
