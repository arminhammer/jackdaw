//! The error types of the library API.
//!
//! There is one error type for the engine, [`Error`]. Inside tasks and workflows two more appear: [`TaskError`] is
//! what a task returns when it fails, and [`StepError`] (also called [`WorkflowError`]) is what a workflow sees when a
//! step fails. A failed workflow reaches the caller as [`Error::Faulted`] carrying a [`Fault`].
//!
//! See the [error guide](crate::guide::errors) for how they fit together.
use serde::{Deserialize, Serialize};
use std::fmt;
use std::time::Duration;

/// The spec's error type URIs, for [`Fault::kind`] and [`TaskError::fault`].
///
/// These are the error types the Serverless Workflow specification defines. A `try`/`catch` in a YAML workflow filters
/// on them, and so can a Rust workflow by matching [`Fault::kind`].
pub mod error_types {
    /// A configuration problem, such as a missing or invalid setting.
    pub const CONFIGURATION: &str =
        "https://open-workflow-specification.org/dsl/errors/types/configuration";
    /// Input or output did not match what was required.
    pub const VALIDATION: &str =
        "https://open-workflow-specification.org/dsl/errors/types/validation";
    /// An expression could not be evaluated.
    pub const EXPRESSION: &str =
        "https://open-workflow-specification.org/dsl/errors/types/expression";
    /// Authentication failed.
    pub const AUTHENTICATION: &str =
        "https://open-workflow-specification.org/dsl/errors/types/authentication";
    /// The caller is not allowed to do this.
    pub const AUTHORIZATION: &str =
        "https://open-workflow-specification.org/dsl/errors/types/authorization";
    /// Something took too long.
    pub const TIMEOUT: &str = "https://open-workflow-specification.org/dsl/errors/types/timeout";
    /// A network or protocol failure talking to another system.
    pub const COMMUNICATION: &str =
        "https://open-workflow-specification.org/dsl/errors/types/communication";
    /// Anything else that went wrong while running. The default for errors a task returns with `?`.
    pub const RUNTIME: &str = "https://open-workflow-specification.org/dsl/errors/types/runtime";
}

/// A workflow-level error with the fields of an [RFC 7807] problem, as the Serverless Workflow spec defines them.
///
/// [RFC 7807]: https://www.rfc-editor.org/rfc/rfc7807
///
/// ```
/// use jackdaw::prelude::*;
///
/// let fault = Fault {
///     kind: error_types::COMMUNICATION.to_string(),
///     status: Some(502),
///     title: Some("Bad Gateway".into()),
///     detail: Some("the payment gateway is down".into()),
///     instance: None,
/// };
/// assert!(fault.to_string().contains("502"));
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Fault {
    /// The error type, usually one of [`error_types`].
    pub kind: String,
    /// An HTTP-style status code, if the error has one.
    pub status: Option<u16>,
    /// A short, human-readable summary.
    pub title: Option<String>,
    /// A longer explanation of this occurrence.
    pub detail: Option<String>,
    /// Where it happened, as a reference into the workflow.
    pub instance: Option<String>,
}

impl Fault {
    /// A `runtime` fault with the given detail.
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

    /// A `validation` fault (status 400) with the given detail.
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

/// A replay that did not match its history.
///
/// Its `Display` names the step, the code that issued it, and what the history recorded instead, for example
/// `step #0 at src/order.rs:42 expected `event approved`, but the history has `call charge``. You get one inside
/// [`Error::NonDeterminism`], and from [`Engine::replay_all`](crate::api::Engine::replay_all).
/// See [durability](crate::guide::durability) for how to avoid and fix these.
#[derive(Debug, Clone, PartialEq)]
pub struct Divergence {
    /// The instance whose history no longer matches.
    pub instance_id: String,
    /// The workflow's identity, `namespace/name version`.
    pub workflow: String,
    /// Zero-based position of the step in the workflow.
    pub step: usize,
    /// What the code asked for, e.g. `event approved`.
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
///
/// It is [`#[non_exhaustive]`](https://doc.rust-lang.org/reference/attributes/type_system.html#the-non_exhaustive-attribute),
/// so a `match` needs a wildcard arm and new variants are not breaking changes. It is `Send + Sync + 'static`, so it
/// works with `?` into `Box<dyn std::error::Error>` and with crates like `anyhow`.
///
/// ```
/// use jackdaw::prelude::*;
///
/// fn describe(error: &Error) -> &'static str {
///     match error {
///         Error::Faulted(_) => "the workflow failed",
///         Error::Cancelled => "the workflow was cancelled",
///         Error::Timeout { .. } => "something took too long",
///         Error::Invalid(_) => "the workflow or its setup is wrong",
///         _ => "something else",
///     }
/// }
/// assert_eq!(describe(&Error::Cancelled), "the workflow was cancelled");
/// ```
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// The workflow raised an error or a task failed and nothing handled it.
    Faulted(Fault),
    /// The workflow was cancelled with [`ExecutionHandle::cancel`](crate::api::ExecutionHandle::cancel).
    Cancelled,
    /// A workflow- or task-level timeout elapsed.
    Timeout {
        /// Which timeout, and how long it was.
        detail: String,
    },
    /// A workflow, registration or request is invalid. Found before anything runs.
    Invalid(String),
    /// A replay diverged from its history. (Boxed: it is much larger than the other variants.)
    NonDeterminism(Box<Divergence>),
    /// The persistence layer failed.
    Persistence(crate::persistence::Error),
    /// The cache failed.
    Cache(crate::cache::Error),
    /// The engine could not do what was asked (unknown instance, internal failure, ...).
    Engine(String),
}

impl Error {
    /// An [`Error::Invalid`] with the given message.
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

/// What a task returns on failure: a spec [`Fault`], and whether retrying could help.
///
/// Like `anyhow::Error`, it converts from any `std::error::Error` (as a non-retryable `runtime` fault), so `?` works
/// inside tasks. It deliberately does not implement `std::error::Error` itself, which is what makes that conversion
/// possible.
///
/// ```
/// use jackdaw::prelude::*;
/// use std::time::Duration;
///
/// // Worth trying again: the retry policy of the step decides how often.
/// let flaky = TaskError::retryable("the database is restarting");
///
/// // Not worth trying again.
/// let invalid = TaskError::non_retryable("the card number is not valid");
///
/// // A specific spec error, with the status and detail a `try`/`catch` can filter on.
/// let declined = TaskError::fault(error_types::COMMUNICATION)
///     .status(402)
///     .title("Payment declined")
///     .detail("the issuer refused the charge")
///     .next_retry_delay(Duration::from_secs(5))
///     .retry();
/// # let _ = (flaky, invalid, declined);
/// ```
///
/// Anything that implements `std::error::Error` converts with `?`:
///
/// ```
/// use jackdaw::prelude::*;
///
/// async fn parse_amount(text: String) -> Result<u64, TaskError> {
///     Ok(text.parse::<u64>()?) // a ParseIntError becomes a non-retryable runtime fault
/// }
/// # let _ = parse_amount;
/// ```
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
    ///
    /// Not retryable unless you add [`retry`](Self::retry).
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

    /// Sets the fault's status code.
    #[must_use]
    pub fn status(mut self, status: u16) -> Self {
        self.fault.status = Some(status);
        self
    }

    /// Sets the fault's short summary.
    #[must_use]
    pub fn title(mut self, title: impl fmt::Display) -> Self {
        self.fault.title = Some(title.to_string());
        self
    }

    /// Sets the fault's explanation.
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

    /// The fault this error becomes if the step is not retried (or retries run out).
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
///
/// `ctx.call(..).await` gives `Result<Out, StepError>`. Returning the error from `#[run]` (with `?`) faults the
/// workflow with the same [`Fault`]; matching on it lets the workflow react, for example by compensating.
///
/// ```
/// use jackdaw::prelude::*;
///
/// fn is_declined(error: &StepError) -> bool {
///     matches!(error, StepError::Fault(fault) if fault.status == Some(402))
/// }
/// let declined = StepError::Fault(Fault {
///     kind: error_types::COMMUNICATION.to_string(),
///     status: Some(402),
///     title: None,
///     detail: None,
///     instance: None,
/// });
/// assert!(is_declined(&declined));
/// ```
#[derive(Debug)]
#[non_exhaustive]
pub enum StepError {
    /// The task (or child workflow) failed with this fault.
    Fault(Fault),
    /// The step's timeout elapsed.
    Timeout {
        /// Which timeout, and how long it was.
        detail: String,
    },
    /// An engine-level error (cancellation, divergence, persistence, ...). Returning it ends the workflow.
    Engine(Error),
}

/// The error type of a workflow's `#[run]` method. The same type as [`StepError`], so `?` on a step just works.
pub type WorkflowError = StepError;

/// The return type of a workflow's `#[run]` method.
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
