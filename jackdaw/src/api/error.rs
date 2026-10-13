//! One error type for the library API, plus the error types tasks and steps use.
use serde::{Deserialize, Serialize};
use std::fmt;
use std::time::Duration;

/// The spec's error type URIs, for [`Fault::kind`] and [`TaskError::fault`].
pub mod error_types {
    pub const CONFIGURATION: &str =
        "https://open-workflow-specification.org/dsl/errors/types/configuration";
    pub const VALIDATION: &str =
        "https://open-workflow-specification.org/dsl/errors/types/validation";
    pub const EXPRESSION: &str =
        "https://open-workflow-specification.org/dsl/errors/types/expression";
    pub const AUTHENTICATION: &str =
        "https://open-workflow-specification.org/dsl/errors/types/authentication";
    pub const AUTHORIZATION: &str =
        "https://open-workflow-specification.org/dsl/errors/types/authorization";
    pub const TIMEOUT: &str = "https://open-workflow-specification.org/dsl/errors/types/timeout";
    pub const COMMUNICATION: &str =
        "https://open-workflow-specification.org/dsl/errors/types/communication";
    pub const RUNTIME: &str = "https://open-workflow-specification.org/dsl/errors/types/runtime";
}

/// A workflow-level error with the fields of an RFC 7807 problem, as the spec defines them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Fault {
    /// The error type, usually one of [`error_types`].
    pub kind: String,
    pub status: Option<u16>,
    pub title: Option<String>,
    pub detail: Option<String>,
    pub instance: Option<String>,
}

impl Fault {
    #[must_use]
    pub fn runtime(detail: impl fmt::Display) -> Self {
        Self {
            kind: error_types::RUNTIME.to_string(),
            status: None,
            title: None,
            detail: Some(detail.to_string()),
            instance: None,
        }
    }

    #[must_use]
    pub fn validation(detail: impl fmt::Display) -> Self {
        Self {
            kind: error_types::VALIDATION.to_string(),
            status: Some(400),
            title: None,
            detail: Some(detail.to_string()),
            instance: None,
        }
    }
}

impl fmt::Display for Fault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.kind)?;
        if let Some(status) = self.status {
            write!(f, " (status {status})")?;
        }
        if let Some(title) = &self.title {
            write!(f, ": {title}")?;
        }
        if let Some(detail) = &self.detail {
            write!(f, ": {detail}")?;
        }
        Ok(())
    }
}

/// A replay that did not match its history. Its `Display` names the step, the code that issued it, and what the
/// history recorded instead.
#[derive(Debug, Clone, PartialEq)]
pub struct Divergence {
    pub instance_id: String,
    pub workflow: String,
    /// Zero-based position of the step in the workflow.
    pub step: usize,
    /// What the code asked for, e.g. `wait_for_event approved`.
    pub expected: String,
    /// What the history recorded at that position, if anything.
    pub recorded: Option<String>,
    /// `file:line` of the call in the workflow's code.
    pub call_site: String,
}

impl fmt::Display for Divergence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "non-deterministic replay of workflow {} instance {}: step #{} at {} expected `{}`, but the history ",
            self.workflow, self.instance_id, self.step, self.call_site, self.expected
        )?;
        match &self.recorded {
            Some(recorded) => write!(f, "has `{recorded}`"),
            None => write!(f, "has no record of it"),
        }
    }
}

/// The error type of the library API.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// The workflow raised an error or a task failed and nothing handled it.
    Faulted(Fault),
    Cancelled,
    /// A workflow- or task-level timeout elapsed.
    Timeout {
        detail: String,
    },
    /// A workflow, registration or request is invalid. Found before anything runs.
    Invalid(String),
    /// A replay diverged from its history. (Boxed: it is much larger than the other variants.)
    NonDeterminism(Box<Divergence>),
    Persistence(crate::persistence::Error),
    Cache(crate::cache::Error),
    /// The engine could not do what was asked (unknown instance, internal failure, ...).
    Engine(String),
}

impl Error {
    #[must_use]
    pub fn invalid(message: impl fmt::Display) -> Self {
        Self::Invalid(message.to_string())
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Faulted(fault) => write!(f, "workflow faulted: {fault}"),
            Self::Cancelled => write!(f, "workflow was cancelled"),
            Self::Timeout { detail } => write!(f, "timed out: {detail}"),
            Self::Invalid(message) => write!(f, "invalid: {message}"),
            Self::NonDeterminism(divergence) => write!(f, "{divergence}"),
            Self::Persistence(source) => write!(f, "{source}"),
            Self::Cache(source) => write!(f, "{source}"),
            Self::Engine(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Persistence(source) => Some(source),
            Self::Cache(source) => Some(source),
            Self::Faulted(_)
            | Self::Cancelled
            | Self::Timeout { .. }
            | Self::Invalid(_)
            | Self::NonDeterminism(_)
            | Self::Engine(_) => None,
        }
    }
}

impl From<crate::persistence::Error> for Error {
    fn from(source: crate::persistence::Error) -> Self {
        Self::Persistence(source)
    }
}

impl From<crate::cache::Error> for Error {
    fn from(source: crate::cache::Error) -> Self {
        Self::Cache(source)
    }
}

/// What a task returns on failure: a spec fault, and whether retrying could help.
///
/// Like `anyhow::Error`, it converts from any `std::error::Error` (as a non-retryable `runtime` fault), so `?` works
/// inside tasks. It deliberately does not implement `std::error::Error` itself.
#[derive(Debug, Clone)]
pub struct TaskError {
    /// Boxed so a `Result<_, TaskError>` stays small.
    pub(crate) fault: Box<Fault>,
    pub(crate) retryable: bool,
    pub(crate) next_retry_delay: Option<Duration>,
}

impl TaskError {
    /// A failure that the step's retry policy may retry.
    #[must_use]
    pub fn retryable(error: impl fmt::Display) -> Self {
        Self {
            fault: Box::new(Fault::runtime(error)),
            retryable: true,
            next_retry_delay: None,
        }
    }

    /// A failure that must not be retried.
    #[must_use]
    pub fn non_retryable(error: impl fmt::Display) -> Self {
        Self {
            fault: Box::new(Fault::runtime(error)),
            retryable: false,
            next_retry_delay: None,
        }
    }

    /// A failure with an explicit spec error type, e.g. `TaskError::fault(error_types::COMMUNICATION).status(502)`.
    #[must_use]
    pub fn fault(kind: &str) -> Self {
        Self {
            fault: Box::new(Fault {
                kind: kind.to_string(),
                status: None,
                title: None,
                detail: None,
                instance: None,
            }),
            retryable: false,
            next_retry_delay: None,
        }
    }

    #[must_use]
    pub fn status(mut self, status: u16) -> Self {
        self.fault.status = Some(status);
        self
    }

    #[must_use]
    pub fn title(mut self, title: impl fmt::Display) -> Self {
        self.fault.title = Some(title.to_string());
        self
    }

    #[must_use]
    pub fn detail(mut self, detail: impl fmt::Display) -> Self {
        self.fault.detail = Some(detail.to_string());
        self
    }

    /// Allows the step's retry policy to retry this failure.
    #[must_use]
    pub fn retry(mut self) -> Self {
        self.retryable = true;
        self
    }

    /// Overrides the retry policy's delay before the next attempt.
    #[must_use]
    pub fn next_retry_delay(mut self, delay: Duration) -> Self {
        self.next_retry_delay = Some(delay);
        self
    }

    #[must_use]
    pub fn as_fault(&self) -> &Fault {
        &self.fault
    }
}

impl<E: std::error::Error + Send + Sync + 'static> From<E> for TaskError {
    fn from(error: E) -> Self {
        Self::non_retryable(error)
    }
}

/// What a workflow sees when a step fails: match it, handle it, or return it to fault the workflow.
#[derive(Debug)]
#[non_exhaustive]
pub enum StepError {
    Fault(Fault),
    Timeout {
        detail: String,
    },
    /// An engine-level error (cancellation, divergence, persistence, ...). Returning it ends the workflow.
    Engine(Error),
}

pub type WorkflowError = StepError;

pub type WorkflowResult<T> = Result<T, WorkflowError>;

impl fmt::Display for StepError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Fault(fault) => write!(f, "{fault}"),
            Self::Timeout { detail } => write!(f, "timed out: {detail}"),
            Self::Engine(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for StepError {}

impl From<Fault> for StepError {
    fn from(fault: Fault) -> Self {
        Self::Fault(fault)
    }
}

impl From<Error> for StepError {
    fn from(error: Error) -> Self {
        match error {
            Error::Faulted(fault) => Self::Fault(fault),
            Error::Timeout { detail } => Self::Timeout { detail },
            Error::Cancelled
            | Error::Invalid(_)
            | Error::NonDeterminism(_)
            | Error::Persistence(_)
            | Error::Cache(_)
            | Error::Engine(_) => Self::Engine(error),
        }
    }
}

impl From<StepError> for Error {
    fn from(error: StepError) -> Self {
        match error {
            StepError::Fault(fault) => Self::Faulted(fault),
            StepError::Timeout { detail } => Self::Timeout { detail },
            StepError::Engine(error) => error,
        }
    }
}
