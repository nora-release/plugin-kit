//! The kernel's guarantees, checked through the agent example's plugins.

// The example's modules, shared; a test uses only part of them.
#[allow(dead_code)]
#[path = "../examples/agent/agent.rs"]
mod agent;
#[path = "../examples/agent/extension.rs"]
mod extension;
#[allow(dead_code)]
#[path = "../examples/agent/hooks.rs"]
mod hooks;
#[allow(dead_code)]
#[path = "../examples/agent/plugins.rs"]
mod plugins;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use agent::turn;
use extension::ToolRegistry;
use nora_plugin::Ctx;
use plugins::*;

fn names(registry: &ToolRegistry) -> Vec<String> {
    registry
        .snapshot()
        .iter()
        .map(|tool| tool.name.clone())
        .collect()
}

async fn eventually(registry: &ToolRegistry, ready: impl Fn(&[String]) -> bool) {
    for _ in 0..400 {
        if ready(&names(registry)) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("never reached: {:?}", names(registry));
}

async fn host() -> (Ctx, Arc<ToolRegistry>) {
    let root = Ctx::root().unwrap();
    let tools = root.plugin(AgentTools);
    (&tools).await.unwrap();
    let registry = root.get::<ToolRegistry>().unwrap();
    (root, registry)
}

#[tokio::test]
async fn k1_a_plugin_waits_for_its_dependencies() {
    let root = Ctx::root().unwrap();
    let applies = Arc::new(AtomicUsize::new(0));
    let count = root.plugin(WordCountTool {
        applies: Arc::clone(&applies),
    });
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(
        applies.load(Ordering::SeqCst),
        0,
        "applied without its deps"
    );

    (&root.plugin(AgentTools)).await.unwrap();
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(
        applies.load(Ordering::SeqCst),
        0,
        "applied with one of two deps"
    );

    (&root.plugin(WhitespaceTokenizer)).await.unwrap();
    (&count).await.unwrap();
    assert_eq!(applies.load(Ordering::SeqCst), 1);
    root.shutdown().await.unwrap();
}

#[tokio::test]
async fn k2_a_contribution_leaves_with_its_plugin() {
    let (root, registry) = host().await;
    let read = root.plugin(ReadFileTool);
    (&read).await.unwrap();
    assert_eq!(names(&registry), ["read_file"]);

    read.dispose().await.unwrap();
    assert!(names(&registry).is_empty());
    root.shutdown().await.unwrap();
}

#[tokio::test]
async fn k3_a_failed_apply_rolls_back() {
    let (root, registry) = host().await;
    let broken = root.plugin(BrokenTool);
    assert!((&broken).await.is_err());
    assert!(!names(&registry).contains(&"half_installed".to_string()));

    (&root.plugin(ReadFileTool)).await.unwrap();
    assert_eq!(
        names(&registry),
        ["read_file"],
        "one failure stopped another plugin"
    );
    root.shutdown().await.unwrap();
}

#[tokio::test]
async fn k4_replacing_a_service_restarts_only_its_users() {
    let (root, registry) = host().await;
    let count_applies = Arc::new(AtomicUsize::new(0));
    (&root.plugin(ReadFileTool)).await.unwrap();
    let words = root.plugin(WhitespaceTokenizer);
    (&words).await.unwrap();
    let count = root.plugin(WordCountTool {
        applies: Arc::clone(&count_applies),
    });
    (&count).await.unwrap();
    let read_before = Arc::clone(
        registry
            .snapshot()
            .iter()
            .find(|t| t.name == "read_file")
            .unwrap(),
    );

    words.dispose().await.unwrap();
    (&root.plugin(CharTokenizer)).await.unwrap();
    (&count).await.unwrap();

    assert_eq!(count_applies.load(Ordering::SeqCst), 2);
    let read_after = Arc::clone(
        registry
            .snapshot()
            .iter()
            .find(|t| t.name == "read_file")
            .unwrap(),
    );
    assert!(
        Arc::ptr_eq(&read_before, &read_after),
        "read_file was restarted too"
    );
    let count_tool = registry
        .snapshot()
        .into_iter()
        .find(|t| t.name == "count")
        .unwrap();
    assert_eq!((count_tool.run)("abc"), "3 chars");
    root.shutdown().await.unwrap();
}

#[tokio::test]
async fn k5_ending_a_scope_ends_everything_inside_it() {
    let (root, registry) = host().await;
    let session = root.plugin(Session { id: "s1" });
    (&session).await.unwrap();
    eventually(&registry, |n| n.contains(&"notes_s1".to_string())).await;

    session.dispose().await.unwrap();
    assert!(names(&registry).is_empty());
    let alive: Vec<String> = root
        .diagnostics()
        .into_iter()
        .filter(|p| p.name == "SessionNotesTool")
        .map(|p| format!("{:?}", p.state))
        .collect();
    assert!(
        alive.is_empty(),
        "session children still mounted: {alive:?}"
    );
    root.shutdown().await.unwrap();
}

#[tokio::test]
async fn k7_load_and_unload_many_times_leaves_nothing() {
    let (root, registry) = host().await;
    for _ in 0..200 {
        let read = root.plugin(ReadFileTool);
        (&read).await.unwrap();
        read.dispose().await.unwrap();
    }
    assert!(names(&registry).is_empty());
    assert_eq!(root.diagnostics().len(), 2, "root + AgentTools only");
    root.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_turn_keeps_the_snapshot_it_started_with() {
    let (root, registry) = host().await;
    let read = root.plugin(ReadFileTool);
    (&read).await.unwrap();

    let snapshot = registry.snapshot();
    read.dispose().await.unwrap();
    assert_eq!(snapshot.len(), 1, "the turn's snapshot changed under it");
    assert!(
        names(&registry).is_empty(),
        "the next turn still sees the tool"
    );
    root.shutdown().await.unwrap();
}

#[tokio::test]
async fn hooks_block_and_wrap_in_registration_order() {
    let (root, registry) = host().await;
    (&root.plugin(BashTool)).await.unwrap();
    (&root.plugin(ShellGuard)).await.unwrap();
    (&root.plugin(WorkspaceNote { cwd: "/a" })).await.unwrap();
    (&root.plugin(WorkspaceNote { cwd: "/b" })).await.unwrap();

    let report = turn(
        &root,
        &registry,
        "hi",
        &[("bash", "ls"), ("bash", "rm -rf /")],
    )
    .await
    .unwrap();
    assert_eq!(report.prompt, "hi\n[workspace: /b]\n[workspace: /a]");
    assert!(report.results.first().unwrap().contains("(ran)"));
    assert!(report.results.get(1).unwrap().contains("blocked"));
    root.shutdown().await.unwrap();
}
