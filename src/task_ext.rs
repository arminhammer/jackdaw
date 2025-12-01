//! Extension trait for TaskDefinition to provide convenient helper methods

use serverless_workflow_core::models::task::TaskDefinition;
use serverless_workflow_core::models::input::InputDataModelDefinition;
use serverless_workflow_core::models::output::OutputDataModelDefinition;

/// Extension trait providing helper methods for TaskDefinition
pub trait TaskDefinitionExt {
    /// Get the export configuration for this task
    fn export(&self) -> Option<&OutputDataModelDefinition>;

    /// Get the input configuration for this task
    fn input(&self) -> Option<&InputDataModelDefinition>;

    /// Get the type name of this task as a string
    fn type_name(&self) -> &'static str;
}

impl TaskDefinitionExt for TaskDefinition {
    fn export(&self) -> Option<&OutputDataModelDefinition> {
        match self {
            TaskDefinition::Call(t) => t.common.export.as_ref(),
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
            TaskDefinition::Call(t) => t.common.input.as_ref(),
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