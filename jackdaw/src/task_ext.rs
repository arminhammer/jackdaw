//! Extension trait for ``TaskDefinition`` to provide convenient helper methods

use crate::dsl::input::InputDataModelDefinition;
use crate::dsl::output::OutputDataModelDefinition;
use crate::dsl::task::CallTaskDefinition;
use crate::dsl::task::TaskDefinition;
use crate::dsl::timeout::OneOfTimeoutDefinitionOrReference;
use serde::ser::Error as _;
use serde_json::Value;
use std::collections::HashMap;

fn to_map<T: serde::Serialize>(
    args: &T,
) -> Result<HashMap<String, serde_json::Value>, serde_json::Error> {
    let value = serde_json::to_value(args)?;
    let serde_json::Value::Object(map) = value else {
        return Err(serde_json::Error::custom(
            "call 'with' must serialize to an object",
        ));
    };
    Ok(map.into_iter().collect())
}

pub fn call_parts(
    t: &CallTaskDefinition,
) -> Result<(&str, HashMap<String, Value>), serde_json::Error> {
    match t {
        CallTaskDefinition::Function(f) => Ok((&f.call, f.with.clone().unwrap_or_default())),
        CallTaskDefinition::Http(h) => Ok((&h.call, to_map(&h.with)?)),
        CallTaskDefinition::Grpc(g) => Ok((&g.call, to_map(&g.with)?)),
        CallTaskDefinition::OpenApi(o) => Ok((&o.call, to_map(&o.with)?)),
        CallTaskDefinition::AsyncApi(a) => Ok((&a.call, to_map(&a.with)?)),
        CallTaskDefinition::A2a(a) => Ok((&a.call, to_map(&a.with)?)),
        CallTaskDefinition::Mcp(m) => Ok((&m.call, to_map(&m.with)?)),
    }
}

/// The name a call task calls (`http`, `greet`, `add-numbers:1.0.0`, ...), whatever kind of call it is.
#[must_use]
pub fn call_name(t: &CallTaskDefinition) -> &str {
    match t {
        CallTaskDefinition::Function(f) => &f.call,
        CallTaskDefinition::Http(h) => &h.call,
        CallTaskDefinition::Grpc(g) => &g.call,
        CallTaskDefinition::OpenApi(o) => &o.call,
        CallTaskDefinition::AsyncApi(a) => &a.call,
        CallTaskDefinition::A2a(a) => &a.call,
        CallTaskDefinition::Mcp(m) => &m.call,
    }
}

/// Extension trait providing helper methods for `TaskDefinition`
pub trait TaskDefinitionExt {
    /// Get the export configuration for this task
    fn export(&self) -> Option<&OutputDataModelDefinition>;

    /// Get the input configuration for this task
    fn input(&self) -> Option<&InputDataModelDefinition>;

    /// Get the timeout configuration for this task
    fn timeout(&self) -> Option<&OneOfTimeoutDefinitionOrReference>;

    /// Get the type name of this task as a string
    fn type_name(&self) -> &'static str;
}

impl TaskDefinitionExt for TaskDefinition {
    fn export(&self) -> Option<&OutputDataModelDefinition> {
        match self {
            TaskDefinition::Call(t) => t.common().export.as_ref(),
            TaskDefinition::Do(t) => t.common.export.as_ref(),
            TaskDefinition::Emit(t) => t.common.export.as_ref(),
            TaskDefinition::For(t) => t.common.export.as_ref(),
            TaskDefinition::Fork(t) => t.common.export.as_ref(),
            TaskDefinition::Listen(t) => t.common.export.as_ref(),
            TaskDefinition::Raise(t) => t.common.export.as_ref(),
            TaskDefinition::Run(t) => t.common.export.as_ref(),
            TaskDefinition::Set(t) => t.common.export.as_ref(),
            TaskDefinition::Switch(t) => t.common.export.as_ref(),
            TaskDefinition::Try(t) => t.common.export.as_ref(),
            TaskDefinition::Wait(t) => t.common.export.as_ref(),
        }
    }

    fn input(&self) -> Option<&InputDataModelDefinition> {
        match self {
            TaskDefinition::Call(t) => t.common().input.as_ref(),
            TaskDefinition::Do(t) => t.common.input.as_ref(),
            TaskDefinition::Emit(t) => t.common.input.as_ref(),
            TaskDefinition::For(t) => t.common.input.as_ref(),
            TaskDefinition::Fork(t) => t.common.input.as_ref(),
            TaskDefinition::Listen(t) => t.common.input.as_ref(),
            TaskDefinition::Raise(t) => t.common.input.as_ref(),
            TaskDefinition::Run(t) => t.common.input.as_ref(),
            TaskDefinition::Set(t) => t.common.input.as_ref(),
            TaskDefinition::Switch(t) => t.common.input.as_ref(),
            TaskDefinition::Try(t) => t.common.input.as_ref(),
            TaskDefinition::Wait(t) => t.common.input.as_ref(),
        }
    }

    fn timeout(&self) -> Option<&OneOfTimeoutDefinitionOrReference> {
        match self {
            TaskDefinition::Call(t) => t.common().timeout.as_ref(),
            TaskDefinition::Do(t) => t.common.timeout.as_ref(),
            TaskDefinition::Emit(t) => t.common.timeout.as_ref(),
            TaskDefinition::For(t) => t.common.timeout.as_ref(),
            TaskDefinition::Fork(t) => t.common.timeout.as_ref(),
            TaskDefinition::Listen(t) => t.common.timeout.as_ref(),
            TaskDefinition::Raise(t) => t.common.timeout.as_ref(),
            TaskDefinition::Run(t) => t.common.timeout.as_ref(),
            TaskDefinition::Set(t) => t.common.timeout.as_ref(),
            TaskDefinition::Switch(t) => t.common.timeout.as_ref(),
            TaskDefinition::Try(t) => t.common.timeout.as_ref(),
            TaskDefinition::Wait(t) => t.common.timeout.as_ref(),
        }
    }

    fn type_name(&self) -> &'static str {
        match self {
            TaskDefinition::Call(_) => "Call",
            TaskDefinition::Set(_) => "Set",
            TaskDefinition::Fork(_) => "Fork",
            TaskDefinition::Run(_) => "Run",
            TaskDefinition::Do(_) => "Do",
            TaskDefinition::For(_) => "For",
            TaskDefinition::Switch(_) => "Switch",
            TaskDefinition::Try(_) => "Try",
            TaskDefinition::Emit(_) => "Emit",
            TaskDefinition::Raise(_) => "Raise",
            TaskDefinition::Wait(_) => "Wait",
            TaskDefinition::Listen(_) => "Listen",
        }
    }
}
