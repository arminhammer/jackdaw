//! Step definitions for the library feature specs in `tests/lib`.
//!
//! Every scenario gets a fresh [`LibWorld`]: a temporary store, a [`support::TestClock`] that only moves when a step
//! moves it (or when a step waits and nothing else can happen), and a [`support::Probe`] that scripts and records every
//! task invocation. Workflows under test are [`world::Fixture`]s, registered on every engine the scenario builds.
#![allow(dead_code)]

pub mod api;
pub mod approval;
pub mod batch;
pub mod calling_tasks;
pub mod cancel;
pub mod children;
pub mod common;
pub mod conditionals;
pub mod durability;
pub mod errors;
pub mod events;
pub mod incident;
pub mod loops;
pub mod parallel;
pub mod recorded;
pub mod saga;
pub mod signals;
pub mod subscription;
pub mod support;
pub mod timers;
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
    errors::defaults(probe);
    timers::defaults(probe);
    events::defaults(probe);
    signals::defaults(probe);
    children::defaults(probe);
    recorded::defaults(probe);
    cancel::defaults(probe);
    durability::defaults(probe);
    saga::defaults(probe);
    approval::defaults(probe);
    batch::defaults(probe);
    incident::defaults(probe);
    subscription::defaults(probe);
    api::defaults(probe);
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
    let p = Arc::clone(probe);
    sets.push(Box::new(move |b| b.tasks(errors::Errs(Arc::clone(&p)))));
    let p = Arc::clone(probe);
    sets.push(Box::new(move |b| b.tasks(timers::Timed(Arc::clone(&p)))));
    let p = Arc::clone(probe);
    sets.push(Box::new(move |b| b.tasks(events::Waits(Arc::clone(&p)))));
    let p = Arc::clone(probe);
    sets.push(Box::new(move |b| b.tasks(signals::Shop(Arc::clone(&p)))));
    let p = Arc::clone(probe);
    sets.push(Box::new(move |b| b.tasks(children::Kids(Arc::clone(&p)))));
    let p = Arc::clone(probe);
    sets.push(Box::new(move |b| {
        b.tasks(recorded::Recorded(Arc::clone(&p)))
    }));
    let p = Arc::clone(probe);
    sets.push(Box::new(move |b| b.tasks(cancel::Cancels(Arc::clone(&p)))));
    let p = Arc::clone(probe);
    sets.push(Box::new(move |b| {
        b.tasks(durability::Durable(Arc::clone(&p)))
    }));
    let p = Arc::clone(probe);
    sets.push(Box::new(move |b| b.tasks(saga::Saga(Arc::clone(&p)))));
    let p = Arc::clone(probe);
    sets.push(Box::new(move |b| {
        b.tasks(approval::Approvals(Arc::clone(&p)))
    }));
    let p = Arc::clone(probe);
    sets.push(Box::new(move |b| b.tasks(batch::Batch(Arc::clone(&p)))));
    let p = Arc::clone(probe);
    sets.push(Box::new(move |b| b.tasks(incident::Ops(Arc::clone(&p)))));
    let p = Arc::clone(probe);
    sets.push(Box::new(move |b| {
        b.tasks(subscription::Billing(Arc::clone(&p)))
    }));
    sets.extend(api::task_sets(probe));
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
    all.extend(errors::fixtures());
    all.extend(timers::fixtures());
    all.extend(events::fixtures());
    all.extend(signals::fixtures());
    all.extend(children::fixtures());
    all.extend(recorded::fixtures());
    all.extend(cancel::fixtures());
    all.extend(durability::fixtures());
    all.extend(saga::fixtures());
    all.extend(approval::fixtures());
    all.extend(batch::fixtures());
    all.extend(incident::fixtures());
    all.extend(subscription::fixtures());
    all.extend(api::fixtures());
    all
}

/// Fixtures that share an identity with a default one (a changed deploy). Registered only when a scenario asks.
pub fn variants() -> Vec<Fixture> {
    let mut all = Vec::new();
    all.extend(conditionals::variants());
    all.extend(durability::variants());
    all.extend(subscription::variants());
    all.extend(api::variants());
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
    table.extend(errors::by_code());
    table.extend(saga::by_code());
    table.extend(approval::by_code());
    table.extend(batch::by_code());
    table.extend(incident::by_code());
    table.extend(subscription::by_code());
    let mut matches = table.iter().filter(|(needle, _)| code.contains(needle));
    let (_, name) = matches
        .next()
        .unwrap_or_else(|| panic!("no fixture for this code:\n{code}"));
    fixture(name)
}
