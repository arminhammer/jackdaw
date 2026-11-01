//! Fixtures shared by the library API tests (see `../README.md`).
#![allow(dead_code)]

pub mod shop;

pub use jackdaw::prelude::*;
pub use serde::{Deserialize, Serialize};
pub use std::sync::Arc;
pub use std::sync::atomic::{AtomicUsize, Ordering};

pub type TestResult = Result<(), Box<dyn std::error::Error>>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Name {
    pub name: String,
}

impl Name {
    pub fn new(name: &str) -> Self {
        Self { name: name.into() }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Greeting {
    pub greeting: String,
}

impl Greeting {
    pub fn hello(name: &str) -> Self {
        Self {
            greeting: format!("Hello, {name}!"),
        }
    }
}

/// The smallest useful task: typed in, typed out, no state. The marker generated for it is `Greet`.
#[task]
pub async fn greet(input: Name) -> Result<Greeting, TaskError> {
    Ok(Greeting::hello(&input.name))
}

/// The same greeting as a YAML workflow, for tests that run the declarative path.
pub const HELLO_YAML: &str = r#"
document:
  dsl: '1.0.3'
  namespace: examples
  name: hello
  version: '1.0.0'
do:
  - greet:
      set:
        greeting: '${ "Hello, " + .name + "!" }'
"#;

/// Counts how often a side effect really happened. Cloning shares the count.
#[derive(Debug, Default, Clone)]
pub struct Counter(Arc<AtomicUsize>);

impl Counter {
    /// Records one call and returns the new total.
    pub fn hit(&self) -> usize {
        self.0.fetch_add(1, Ordering::SeqCst) + 1
    }

    pub fn get(&self) -> usize {
        self.0.load(Ordering::SeqCst)
    }
}

pub fn names(events: &[Event]) -> Vec<&'static str> {
    events.iter().map(Event::name).collect()
}

/// Drains an event stream to its end (the stream ends after the terminal event).
pub async fn collect(mut events: EventStream) -> Vec<Event> {
    let mut all = Vec::new();
    while let Some(event) = events.next().await {
        all.push(event);
    }
    all
}

/// Reads events until the named task has completed. Used to stop an engine "mid-workflow".
pub async fn until_task_completed(events: &mut EventStream, task: &str) {
    while let Some(event) = events.next().await {
        if let Event::TaskCompleted { task_name, .. } = &event
            && task_name == task
        {
            return;
        }
    }
    panic!("the workflow ended before task `{task}` completed");
}
