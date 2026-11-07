//! An agent's tools and hooks as plugins, walked through one step at a time.
//! Run: `cargo run -p nora-plugin --example agent`.

mod agent;
mod extension;
mod hooks;
mod plugins;

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use agent::{turn, TurnReport};
use extension::ToolRegistry;
use nora_plugin::Ctx;
use plugins::*;

const PLAN: &[(&str, &str)] = &[
    ("read_file", "Cargo.toml"),
    ("count", "hello plugin world"),
    ("bash", "ls"),
    ("bash", "rm -rf /"),
    ("notes_s1", "remember the kernel"),
];

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = Ctx::root()?;
    let applies = Arc::new(AtomicUsize::new(0));
    let calls = Arc::new(Mutex::new(BTreeMap::new()));
    let output_bytes = Arc::new(AtomicUsize::new(0));

    step("1. Mount in any order: count waits for its dependencies");
    let count = root.plugin(WordCountTool {
        applies: Arc::clone(&applies),
    });
    let host = root.plugin(AgentTools);
    (&host).await?;
    for view in [
        root.plugin(ReadFileTool),
        root.plugin(BashTool),
        root.plugin(ShellGuard),
        root.plugin(WorkspaceNote { cwd: "~/nora" }),
        root.plugin(ToolCounter {
            calls: Arc::clone(&calls),
            output_bytes: Arc::clone(&output_bytes),
        }),
    ] {
        (&view).await?;
    }
    println!("count before a tokenizer exists: {:?}", count.state());
    let words = root.plugin(WhitespaceTokenizer);
    (&words).await?;
    (&count).await?;
    println!("count after:                     {:?}", count.state());

    let registry = root.get::<ToolRegistry>().ok_or("no tool registry")?;
    print_turn(turn(&root, &registry, "Summarise the repo", PLAN).await?);

    step("2. A session scope adds its own tool");
    let session = root.plugin(Session { id: "s1" });
    (&session).await?;
    wait_for(&registry, |names| names.contains(&"notes_s1".to_string())).await?;
    print_turn(turn(&root, &registry, "Next turn", PLAN).await?);

    step("3. Replace the tokenizer: only count restarts");
    words.dispose().await?;
    let chars = root.plugin(CharTokenizer);
    (&chars).await?;
    (&count).await?;
    println!("count applied {} times", applies.load(Ordering::SeqCst));
    print_turn(turn(&root, &registry, "After the swap", PLAN).await?);

    step("4. End the session: its tool leaves with it");
    session.dispose().await?;
    print_turn(turn(&root, &registry, "After the session", PLAN).await?);

    step("5. A plugin that fails in apply is rolled back");
    let broken = root.plugin(BrokenTool);
    match (&broken).await {
        Ok(()) => println!("unexpected: BrokenTool is active"),
        Err(error) => println!("BrokenTool failed: {error}"),
    }
    let names = names(&registry);
    println!(
        "half_installed left behind: {}",
        names.contains(&"half_installed".to_string())
    );

    step("6. Diagnostics");
    println!(
        "observed tool calls: {:?}, {} bytes of output",
        calls.lock().unwrap_or_else(PoisonError::into_inner),
        output_bytes.load(Ordering::SeqCst)
    );
    for plugin in root.diagnostics() {
        let deps: Vec<String> = plugin
            .deps
            .iter()
            .map(|(name, ready)| format!("{name} {}", if *ready { "ready" } else { "missing" }))
            .collect();
        println!(
            "  {:<20} {:<8} gen {}  deps [{}]{}",
            plugin.name,
            format!("{:?}", plugin.state),
            plugin.generation,
            deps.join(", "),
            plugin
                .error
                .map(|e| format!("  error: {e}"))
                .unwrap_or_default(),
        );
    }

    root.shutdown().await?;
    println!(
        "\nshut down; tools left in the registry: {}",
        registry.snapshot().len()
    );
    Ok(())
}

fn step(title: &str) {
    println!("\n== {title}");
}

fn names(registry: &ToolRegistry) -> Vec<String> {
    registry
        .snapshot()
        .iter()
        .map(|tool| tool.name.clone())
        .collect()
}

/// A child mounted inside another plugin's apply loads on its own; wait for
/// its contribution instead of sleeping a fixed time.
async fn wait_for(
    registry: &ToolRegistry,
    ready: impl Fn(&[String]) -> bool,
) -> Result<(), &'static str> {
    for _ in 0..200 {
        if ready(&names(registry)) {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    Err("timed out waiting for a contribution")
}

fn print_turn(report: TurnReport) {
    println!("prompt: {:?}", report.prompt);
    println!("tools:  {}", report.tools.join(", "));
    for line in report.results {
        println!("  - {line}");
    }
}
