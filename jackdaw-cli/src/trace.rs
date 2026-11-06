//! Renders the parts of the library's YAML trace (`tracing` events on the `jackdaw::yaml` target) that the run's events
//! do not carry.
//!
//! Which step started, finished, failed, was tried again or was done before a restart comes from the run's events (see
//! `jackdaw::runner`). What does not is a script's output while it runs and the listeners' startup, always shown, and
//! with `--debug` each task's context, input and run parameters, and its cache hits and misses.

use console::{Color, style};
use indicatif::MultiProgress;
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Mutex;
use tracing::field::{Field, Visit};
use tracing_subscriber::layer::Context;

/// The library's trace target.
pub const TARGET: &str = "jackdaw::yaml";

/// Colors for the labels of script output, one per task index.
static TASK_COLORS: &[Color] = &[
    Color::Cyan,
    Color::Green,
    Color::Yellow,
    Color::Magenta,
    Color::Blue,
    Color::Red,
];

/// Keeps one event's lines together when tasks run concurrently.
static OUTPUT: Mutex<()> = Mutex::new(());

/// The layer: register it with a filter that lets `jackdaw::yaml` through at `DEBUG` (with `--debug`) or `INFO`. It
/// prints around the progress bars of `display`, so the two do not overwrite each other.
pub struct YamlTrace(pub MultiProgress);

#[derive(Default)]
struct Fields(BTreeMap<String, String>);

impl Visit for Fields {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.0.insert(field.name().to_string(), value.to_string());
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.0
            .insert(field.name().to_string(), format!("{value:?}"));
    }
}

impl Fields {
    fn str(&self, name: &str) -> &str {
        self.0.get(name).map_or("", String::as_str)
    }

    fn opt(&self, name: &str) -> Option<&str> {
        self.0.get(name).map(String::as_str)
    }

    fn json(&self, name: &str) -> Option<Value> {
        self.0
            .get(name)
            .and_then(|text| serde_json::from_str(text).ok())
    }

    fn int(&self, name: &str) -> i64 {
        self.0
            .get(name)
            .and_then(|text| text.parse().ok())
            .unwrap_or(0)
    }
}

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for YamlTrace {
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        if event.metadata().target() != TARGET {
            return;
        }
        let mut fields = Fields::default();
        event.record(&mut fields);
        let _lines = OUTPUT
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.0.suspend(|| render(&fields));
    }
}

fn render(f: &Fields) {
    match f.str("event") {
        "script_line" => {
            let color = usize::try_from(f.int("index"))
                .ok()
                .and_then(|index| TASK_COLORS.get(index % TASK_COLORS.len()))
                .copied()
                .unwrap_or(Color::Cyan);
            let stream = f.str("stream");
            let label = style(format!("[{}:{stream}]", f.str("task")))
                .fg(color)
                .bold();
            if stream == "stderr" {
                eprintln!("{label} {}", f.str("line"));
            } else {
                println!("{label} {}", f.str("line"));
            }
        }
        "listener_route" => println!(
            "  Registering {} for task {} on {}",
            f.str("route"),
            f.str("task"),
            f.str("addr")
        ),
        "listener_started" => println!(
            "  {} listener started on {}",
            f.str("protocol"),
            f.str("addr")
        ),
        "cache_hit" => {
            println!("  {}", style("Cache Hit").yellow().bold());
            println!("  {}", "·".repeat(78));
            println!(
                "    {} {}",
                style("Key:").yellow(),
                style(f.str("key")).yellow()
            );
            if let Some(at) = f.opt("cached_at") {
                println!(
                    "    {} {}",
                    style("Cached at:").yellow(),
                    style(at).yellow()
                );
            }
        }
        "cache_miss" => {
            println!("  {}", style("Cache Miss").yellow());
            println!("  {}", "·".repeat(78));
            println!(
                "    {} {}",
                style("Key:").yellow(),
                style(f.str("key")).yellow()
            );
        }
        "task_context" => section(
            "Context",
            &f.json("context").unwrap_or(Value::Null),
            Color::White,
        ),
        "task_input" => section(
            "Input",
            &f.json("input").unwrap_or(Value::Null),
            Color::Cyan,
        ),
        "run_params" => run_params(f),
        _ => {}
    }
}

/// A task's context or input: each field, or `(empty)`.
fn section(title: &str, value: &Value, color: Color) {
    println!("  {}", style(title).fg(color));
    println!("  {}", "·".repeat(78));
    match value.as_object() {
        Some(obj) if obj.is_empty() => println!("    {}", style("(empty)").dim()),
        Some(obj) => {
            for (key, value) in obj {
                println!("    {}", style(format!("{key}:")).fg(color));
                println!("{}", colorize_json(value, 6, color));
            }
        }
        None => println!("{}", colorize_json(value, 4, color)),
    }
}

fn run_params(f: &Fields) {
    if let Some(language) = f.opt("language") {
        println!("  {} {}", style("Language:").cyan(), style(language).cyan());
    }
    if let Some(stdin) = f.opt("stdin") {
        println!(
            "  {} {}",
            style("Stdin:").cyan(),
            style(format!("\"{stdin}\"")).cyan()
        );
    }
    if let Some(arguments) = f
        .json("arguments")
        .and_then(|value| value.as_array().cloned())
        .filter(|arguments| !arguments.is_empty())
    {
        println!("  {}", style("Arguments:").cyan());
        for argument in arguments.iter().filter_map(Value::as_str) {
            println!("    {}", style(format!("- {argument}")).cyan());
        }
    }
    if let Some(environment) = f
        .json("environment")
        .and_then(|value| value.as_object().cloned())
        .filter(|environment| !environment.is_empty())
    {
        println!("  {}", style("Environment:").cyan());
        for (key, value) in &environment {
            if let Some(value) = value.as_str() {
                println!(
                    "    {} {}",
                    style(format!("{key}:")).cyan(),
                    style(value).cyan()
                );
            }
        }
    }
}

fn colorize_json(value: &Value, indent: usize, color: Color) -> String {
    let pad = " ".repeat(indent);
    serde_json::to_string_pretty(value)
        .unwrap_or_else(|_| "{}".to_string())
        .lines()
        .map(|line| format!("{pad}{}", style(line).fg(color)))
        .collect::<Vec<_>>()
        .join("\n")
}
