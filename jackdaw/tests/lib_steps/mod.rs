//! Step definitions for the library feature specs in `tests/lib`.
//!
//! Every scenario gets a fresh [`LibWorld`]: a temporary store, a [`support::TestClock`] that only moves when a step
//! moves it (or when a step waits and nothing else can happen), and a [`support::Probe`] that scripts and records every
//! task invocation. Workflows under test are [`world::Fixture`]s, registered on every engine the scenario builds.
#![allow(dead_code)]

pub mod calling_tasks;
pub mod common;
pub mod conditionals;
pub mod loops;
pub mod parallel;
pub mod support;
pub mod world;

use jackdaw::prelude::*;
use std::sync::Arc;
use support::Probe;
pub use world::LibWorld;
use world::{Fixture, Harnesses, Register};

type TaskSet = Box<dyn Fn(EngineBuilder) -> EngineBuilder + Send + Sync>;

/// The task sets for one scenario, all sharing its probe.
pub fn harnesses(probe: &Arc<Probe>) -> Harnesses {
    calling_tasks::defaults(probe);
    conditionals::defaults(probe);
    loops::defaults(probe);
    parallel::defaults(probe);
    let inventory = Arc::new(calling_tasks::Inventory::default());
    let mut sets: Vec<TaskSet> = Vec::new();
    let p = Arc::clone(probe);
    sets.push(Box::new(move |b| {
        b.tasks(calling_tasks::Calls(Arc::clone(&p)))
    }));
    let held = Arc::clone(&inventory);
    sets.push(Box::new(move |b| b.tasks(Arc::clone(&held))));
    sets.push(Box::new(|b| b.task(calling_tasks::Grind)));
    let p = Arc::clone(probe);
    sets.push(Box::new(move |b| {
        b.tasks(conditionals::Branches(Arc::clone(&p)))
    }));
    let p = Arc::clone(probe);
    sets.push(Box::new(move |b| b.tasks(loops::Looping(Arc::clone(&p)))));
    let p = Arc::clone(probe);
    sets.push(Box::new(move |b| b.tasks(parallel::Fan(Arc::clone(&p)))));
    Harnesses {
        probe: Arc::clone(probe),
        sets,
        inventory,
    }
}

/// Every workflow fixture.
pub fn catalog() -> Vec<Fixture> {
    let mut all = Vec::new();
    all.extend(calling_tasks::fixtures());
    all.extend(conditionals::fixtures());
    all.extend(loops::fixtures());
    all.extend(parallel::fixtures());
    all
}

/// Fixtures that share an identity with a default one (a changed deploy). Registered only when a scenario asks.
pub fn variants() -> Vec<Fixture> {
    let mut all = Vec::new();
    all.extend(conditionals::variants());
    all
}

pub fn variant(name: &str) -> Fixture {
    variants()
        .into_iter()
        .find(|fixture| fixture.name == name)
        .unwrap_or_else(|| panic!("no variant named `{name}`"))
}

/// Registrations for every fixture: the default for an engine.
pub fn all_fixtures() -> Vec<Register> {
    catalog()
        .into_iter()
        .map(|fixture| fixture.register)
        .collect()
}

pub fn fixture(name: &str) -> Fixture {
    catalog()
        .into_iter()
        .find(|fixture| fixture.name == name)
        .unwrap_or_else(|| panic!("no fixture named `{name}`"))
}

/// The fixture whose code a scenario's docstring shows, found by a line only that code has.
pub fn fixture_by_code(code: &str) -> Fixture {
    let mut table = Vec::new();
    table.extend(calling_tasks::by_code());
    table.extend(loops::by_code());
    table.extend(parallel::by_code());
    let mut matches = table.iter().filter(|(needle, _)| code.contains(needle));
    let (_, name) = matches
        .next()
        .unwrap_or_else(|| panic!("no fixture for this code:\n{code}"));
    fixture(name)
}
